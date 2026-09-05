//! Hand-rolled minimal GDExtension FFI — the crate's engine-facing surface:
//! a direct binding to the engine's `GDExtensionInterface` (vendored header
//! `vendor/gdextension_interface.h`, Godot 4.5.1). Exports
//! [`gdextension_entry`] and registers the parent panel classes as `Control`
//! subclasses with one virtual (`_draw`) and the zero-argument `refresh`
//! method the shim calls every frame. Each parent natively instantiates the
//! shared child class through [`instantiate_class`]; that class exposes only
//! `_draw`.
//!
//! # Native code never inspects engine input events — scroll arrives from the shim
//!
//! The extension registers NO `_gui_input` virtual and never touches an
//! engine `InputEvent`: any GDExtension-originated call about an
//! engine-created input event hangs the game's proprietary engine fork,
//! while the fork's own managed→native calls are fine. Scroll instead
//! reaches the panels from the shim: it connects to each panel's `GuiInput`
//! signal and forwards the raw fields through the flat C export
//! [`crate::abi::spire_profiler_scroll_input`]; the per-frame `refresh` consumes the
//! queued pixels and all scroll math stays native ([`crate::ui::scroll`]).
//!
//! Every engine call targets objects the extension itself created (the
//! panel Controls) or engine-owned singletons. Raw engine pointers are
//! trusted only for the callback or wrapper call and only under the pinned
//! 4.5.1 header/runtime contract.
//!
//! # Binding mechanics
//!
//! The engine API resolves by C name at runtime: [`gdextension_entry`] takes
//! the engine's `get_proc_address` and looks up every interface function; a
//! missing symbol fails loudly by name. `_draw` dispatch is `get_virtual2`,
//! and the virtual-name slot compares interned StringName pointers
//! (StringName is one interned pointer in 4.5, so that is exact equality).
//! StringName is interned and engine-owned; String is refcounted, so a
//! temporary String built for a Variant must be destroyed once the Variant
//! copies from it. Nothing verifies Rust→engine method names — grep
//! `extension_api_4.5.1.json` before wiring a new call: a stale name
//! fails loudly at the call site, as a fail-logged ERROR.
//!
//! # Engine-call technique (variant_call everywhere)
//!
//! Every engine method call goes through [`variant_call`], not
//! `object_method_bind_ptrcall`: the ptrcall route requires exact signature
//! hashes only published in `extension_api.json`, while [`variant_call`]
//! needs none. Values are built with [`get_variant_from_type_constructor`],
//! read back with [`variant_get_ptr_internal_getter`] (after a
//! [`variant_get_type`] tag check — the internal getter is undefined behavior
//! on a type mismatch), and every temporary Variant is [`variant_destroy`]ed
//! on drop. Return slots are constructed as NIL first, so a call that
//! fails before assigning its return value still drops a valid Variant.
//!
//! # Where this module's unsafe lives
//!
//! `lib.rs` declares `#![deny(unsafe_code)]`, relaxed in exactly three
//! modules: [`crate::abi`], [`crate::registration`], and this one (whose
//! `#[allow(unsafe_code)]` sits in `engine.rs`). The unsafe concentrates in:
//!
//! 1. **Raw engine pointers** — `*mut c_void` the engine supplies, each valid for its call's
//!    duration: the interned StringName pointer read in [`string_name_eq`], the variant payload
//!    bytes copied out via [`variant_get_ptr_internal_getter`], the class/instance user-data
//!    reborrowed in [`create_instance`]/[`refresh_call`]/[`free_instance`]/[`draw_virtual`], and
//!    the [`Initialization`]/[`CallError`] out-params written in
//!    [`gdextension_entry`]/[`refresh_call`].
//! 2. **The export and C callbacks** — `extern "C"` functions whose pointers the engine stores and
//!    calls back into; every one routes through [`crate::abi::contain`] so a panic never unwinds
//!    into the engine. Each callback's panic label is computed just ahead of the boundary — a deref
//!    of the same engine-supplied pointer the body trusts, plus a UTF-8 `expect` on the class's
//!    `c"…"` name literal — and cannot panic by construction.
//! 3. **Function-pointer resolution** — `get_proc_address` returns an opaque pointer that
//!    [`lookup`] reinterprets only as the pinned header's signature for that symbol.
//!
//! # Trusted engine-layout facts
//!
//! The header exposes Variant, String, and StringName only as opaque
//! pointers. The facts below are properties of the pinned Godot 4.5.1
//! binary; if a pin bump breaks one, [`Opaque`] storage or [`string_name_eq`]
//! becomes unsound, so they belong on the manual re-verification list:
//!
//! * Variant, String, and StringName fit in [`OPAQUE_SIZE`] bytes with alignment ≤ 16.
//! * StringName is one pointer to interned storage, so comparing the first 8 bytes of two live
//!   names is name equality.
//! * Internal-getter payloads match the [`read_payload`] reads: bool is one 0/1 byte, Vector2
//!   `[f32; 2]`, Rect2/Color `[f32; 4]`, object a pointer.
//! * The internal getter never writes through its Variant argument despite the mutable pointer
//!   type, which is what [`read_payload`] hands out from `&Opaque`.
//! * `get_proc_address` returns null for an unknown name and a function matching the
//!   header-documented signature otherwise ([`lookup`]'s decode and transmute).
//! * [`object_set_instance`] binds the instance for the object's life: the engine passes it back to
//!   [`free_instance`] (exactly once), [`draw_virtual`], and [`refresh_call`] only while the object
//!   lives.
//! * Variant from-type constructors copy out of the source pointer, so the source may be a stack
//!   temporary or, for String, destroyed right after ([`string_variant`]); constructing an OBJECT
//!   Variant takes the engine Ref that [`variant_destroy`] later releases ([`retained_object`]).
//! * `variant_call` never writes through `p_self`: the receiver is read-only despite the mutable
//!   pointer type, which is what [`RetainedVariant::ptr`] hands out from `&self`.
//! * The enum values transcribed from the pinned header match it: the `VT_*` Variant type tags,
//!   `CALL_OK`, `INIT_LEVEL_SCENE`, `METHOD_FLAG_NORMAL`, and `MOUSE_BUTTON_LEFT`. Struct layouts
//!   carry compile-time size pins; scalar values cannot, so a pin bump re-checks them by hand.
//!
//! The panel modules hold engine pointers only through the safe [`Object`]
//! newtype (defined in [`crate::engine::object`]) and call its methods, so
//! they never need their own unsafe.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::abi::contain;
use crate::engine::math::Vector2;
use crate::{fail, marker, warn};

// ── engine type aliases (from gdextension_interface.h) ──────────────────────

type GDExtensionBool = u8;
pub(crate) type GDExtensionInt = i64;

pub(crate) type ObjectPtr = *mut c_void;
type ClassInstancePtr = *mut c_void;
type ClassLibraryPtr = *mut c_void;
type StringNamePtr = *mut c_void;
pub(crate) type ConstStringNamePtr = *const c_void;
type StringPtr = *mut c_void;
type VariantPtr = *mut c_void;
pub(crate) type ConstVariantPtr = *const c_void;
type TypePtr = *mut c_void;
type ConstTypePtr = *const c_void;

// ── constants (enum values from the header) ─────────────────────────────────

pub(crate) const CALL_OK: c_int = 0;
/// Header enum value 1; reported when the method name itself is unusable.
pub(crate) const CALL_ERROR_INVALID_METHOD: c_int = 1;
const INIT_LEVEL_SCENE: c_int = 2;
const METHOD_FLAG_NORMAL: u32 = 1;
pub(crate) const MOUSE_BUTTON_LEFT: i64 = 1;

// GDExtensionVariantType values (the header enum's ordinal positions).
pub(crate) const VT_BOOL: c_int = 1;
pub(crate) const VT_INT: c_int = 2;
pub(crate) const VT_FLOAT: c_int = 3;
const VT_STRING: c_int = 4;
pub(crate) const VT_VECTOR2: c_int = 5;
pub(crate) const VT_RECT2: c_int = 7;
pub(crate) const VT_COLOR: c_int = 20;
pub(crate) const VT_OBJECT: c_int = 24;

/// 64 bytes exceeds any engine opaque type in 4.5.1 (Variant 24,
/// String/StringName 8); 16-byte alignment satisfies Variant's union.
#[repr(C, align(16))]
pub(crate) struct Opaque(pub(crate) [u8; OPAQUE_SIZE]);
pub(crate) const OPAQUE_SIZE: usize = 64;

// ── function-pointer types (resolved by name from get_proc_address) ─────────

type InterfaceFunction = unsafe extern "C" fn();
/// `Option<fn()>` is the header's FFI-safe nullable function pointer.
type GetProcAddressFn = unsafe extern "C" fn(*const c_char) -> Option<InterfaceFunction>;

type FromTypeConstructorFn = unsafe extern "C" fn(VariantPtr, *mut c_void);
type InternalGetterFn = unsafe extern "C" fn(VariantPtr) -> *mut c_void;
type InitializeCallbackFn = unsafe extern "C" fn(*mut c_void, c_int);
type CreateInstance2Fn = unsafe extern "C" fn(*mut c_void, GDExtensionBool) -> ObjectPtr;
type FreeInstanceFn = unsafe extern "C" fn(*mut c_void, ClassInstancePtr);
type ClassCallVirtualFn = unsafe extern "C" fn(ClassInstancePtr, *const ConstTypePtr, TypePtr);
type GetVirtual2Fn =
    unsafe extern "C" fn(*mut c_void, ConstStringNamePtr, u32) -> Option<ClassCallVirtualFn>;
type ClassMethodCallFn = unsafe extern "C" fn(
    *mut c_void,
    ClassInstancePtr,
    *const ConstVariantPtr,
    GDExtensionInt,
    VariantPtr,
    *mut CallError,
);
type PtrDestructorFn = unsafe extern "C" fn(TypePtr);
pub(crate) type ObjectId = u64;

/// Every field is a function pointer, so the struct is `Sync`.
#[derive(Clone, Copy)]
struct Api {
    classdb_construct_object: unsafe extern "C" fn(ConstStringNamePtr) -> ObjectPtr,
    classdb_register_extension_class5: unsafe extern "C" fn(
        ClassLibraryPtr,
        ConstStringNamePtr,
        ConstStringNamePtr,
        *const ClassCreationInfo4,
    ),
    classdb_unregister_extension_class: unsafe extern "C" fn(ClassLibraryPtr, ConstStringNamePtr),
    classdb_register_extension_class_method:
        unsafe extern "C" fn(ClassLibraryPtr, ConstStringNamePtr, *const ClassMethodInfo),
    object_set_instance: unsafe extern "C" fn(ObjectPtr, ConstStringNamePtr, ClassInstancePtr),
    object_get_instance_from_id: unsafe extern "C" fn(ObjectId) -> ObjectPtr,
    object_get_instance_id: unsafe extern "C" fn(ObjectPtr) -> ObjectId,
    global_get_singleton: unsafe extern "C" fn(ConstStringNamePtr) -> ObjectPtr,
    get_variant_from_type_constructor: unsafe extern "C" fn(c_int) -> Option<FromTypeConstructorFn>,
    variant_get_ptr_internal_getter: unsafe extern "C" fn(c_int) -> Option<InternalGetterFn>,
    variant_call: unsafe extern "C" fn(
        VariantPtr,
        ConstStringNamePtr,
        *const ConstVariantPtr,
        GDExtensionInt,
        VariantPtr,
        *mut CallError,
    ),
    variant_destroy: unsafe extern "C" fn(VariantPtr),
    variant_new_nil: unsafe extern "C" fn(VariantPtr),
    variant_get_type: unsafe extern "C" fn(ConstVariantPtr) -> c_int,
    variant_get_ptr_destructor: unsafe extern "C" fn(c_int) -> Option<PtrDestructorFn>,
    string_new_with_utf8_chars: unsafe extern "C" fn(StringPtr, *const c_char),
    string_name_new_with_utf8_chars: unsafe extern "C" fn(StringNamePtr, *const c_char),
}

impl Api {
    /// A missing symbol fails the entry point; [`lookup`] names it first.
    ///
    /// # Safety
    /// `get` is the engine's live resolver and every name is a pinned
    /// header symbol.
    unsafe fn resolve(get: GetProcAddressFn) -> Option<Api> {
        // Safety: lookup is unsafe because it reinterprets the engine's
        // function pointer; resolving each symbol by name is the whole point
        // of the entry path.
        unsafe {
            Some(Api {
                classdb_construct_object: lookup(get, c"classdb_construct_object")?,
                classdb_register_extension_class5: lookup(
                    get,
                    c"classdb_register_extension_class5",
                )?,
                classdb_unregister_extension_class: lookup(
                    get,
                    c"classdb_unregister_extension_class",
                )?,
                classdb_register_extension_class_method: lookup(
                    get,
                    c"classdb_register_extension_class_method",
                )?,
                object_set_instance: lookup(get, c"object_set_instance")?,
                object_get_instance_from_id: lookup(get, c"object_get_instance_from_id")?,
                object_get_instance_id: lookup(get, c"object_get_instance_id")?,
                global_get_singleton: lookup(get, c"global_get_singleton")?,
                get_variant_from_type_constructor: lookup(
                    get,
                    c"get_variant_from_type_constructor",
                )?,
                variant_get_ptr_internal_getter: lookup(get, c"variant_get_ptr_internal_getter")?,
                variant_call: lookup(get, c"variant_call")?,
                variant_destroy: lookup(get, c"variant_destroy")?,
                variant_new_nil: lookup(get, c"variant_new_nil")?,
                variant_get_type: lookup(get, c"variant_get_type")?,
                variant_get_ptr_destructor: lookup(get, c"variant_get_ptr_destructor")?,
                string_new_with_utf8_chars: lookup(get, c"string_new_with_utf8_chars")?,
                string_name_new_with_utf8_chars: lookup(get, c"string_name_new_with_utf8_chars")?,
            })
        }
    }
}

static API: OnceLock<Api> = OnceLock::new();

/// By value ([`Api`] is `Copy`) so call sites skip the `&Api` parens.
fn api() -> Api {
    *API.get()
        .expect("the GDExtension interface resolves before any callback runs")
}

/// Resolved once; a static, not a [`Global`] field, so destruction works on
/// any thread.
static STRING_DTOR: OnceLock<Option<PtrDestructorFn>> = OnceLock::new();

// Thin wrappers over the raw function pointers. Their raw arguments must be
// engine-produced and lifetime-valid for the call; each block only forwards.

/// Deprecated `classdb_construct_object`, not `2` — the latter would send
/// NOTIFICATION_POSTINITIALIZE.
pub(crate) fn classdb_construct_object(name: ConstStringNamePtr) -> ObjectPtr {
    // SAFETY: the API is resolved; every pointer argument is valid for this call.
    unsafe { (api().classdb_construct_object)(name) }
}

fn classdb_register_extension_class5(
    library: ClassLibraryPtr,
    class: ConstStringNamePtr,
    parent: ConstStringNamePtr,
    info: *const ClassCreationInfo4,
) {
    // SAFETY: the API is resolved; every pointer argument is valid for this call.
    unsafe { (api().classdb_register_extension_class5)(library, class, parent, info) };
}

fn classdb_unregister_extension_class(library: ClassLibraryPtr, class: ConstStringNamePtr) {
    // SAFETY: the API is resolved; every pointer argument is valid for this call.
    unsafe { (api().classdb_unregister_extension_class)(library, class) };
}

fn classdb_register_extension_class_method(
    library: ClassLibraryPtr,
    class: ConstStringNamePtr,
    method: *const ClassMethodInfo,
) {
    // SAFETY: the API is resolved; every pointer argument is valid for this call.
    unsafe { (api().classdb_register_extension_class_method)(library, class, method) };
}

fn object_set_instance(obj: ObjectPtr, class: ConstStringNamePtr, instance: ClassInstancePtr) {
    // SAFETY: the API is resolved; every pointer argument is valid for this call.
    unsafe { (api().object_set_instance)(obj, class, instance) };
}

fn object_get_instance_from_id(id: ObjectId) -> ObjectPtr {
    // SAFETY: the API is resolved; the id argument is a by-value copy.
    unsafe { (api().object_get_instance_from_id)(id) }
}

fn object_get_instance_id(obj: ObjectPtr) -> ObjectId {
    // SAFETY: the API is resolved; every pointer argument is valid for this call.
    unsafe { (api().object_get_instance_id)(obj) }
}

fn global_get_singleton(name: ConstStringNamePtr) -> ObjectPtr {
    // SAFETY: the API is resolved; every pointer argument is valid for this call.
    unsafe { (api().global_get_singleton)(name) }
}

fn get_variant_from_type_constructor(vtype: c_int) -> FromTypeConstructorFn {
    // SAFETY: the API is resolved; callers pass only valid VT_* ordinals.
    unsafe { (api().get_variant_from_type_constructor)(vtype) }
        .expect("Godot 4.5 defines a from-type constructor for every VT_* ordinal used")
}

fn variant_get_ptr_internal_getter(vtype: c_int) -> Option<InternalGetterFn> {
    // SAFETY: the API is resolved; callers pass only valid VT_* ordinals.
    unsafe { (api().variant_get_ptr_internal_getter)(vtype) }
}

pub(crate) fn variant_call(
    p_self: VariantPtr,
    method: ConstStringNamePtr,
    args: *const ConstVariantPtr,
    arg_count: GDExtensionInt,
    ret: VariantPtr,
    err: *mut CallError,
) {
    // SAFETY: the API is resolved; every pointer argument is valid for this call.
    unsafe { (api().variant_call)(p_self, method, args, arg_count, ret, err) };
}

pub(crate) fn variant_destroy(p: VariantPtr) {
    // SAFETY: the API is resolved; every pointer argument is valid for this call.
    unsafe { (api().variant_destroy)(p) };
}

pub(crate) fn variant_get_type(p: ConstVariantPtr) -> c_int {
    // SAFETY: the API is resolved; every pointer argument is valid for this call.
    unsafe { (api().variant_get_type)(p) }
}

fn variant_get_ptr_destructor(vtype: c_int) -> Option<PtrDestructorFn> {
    // SAFETY: the API is resolved; callers pass only valid VT_* ordinals.
    unsafe { (api().variant_get_ptr_destructor)(vtype) }
}

fn string_new_with_utf8_chars(dest: StringPtr, contents: *const c_char) {
    // SAFETY: dest is owned opaque storage and contents is a valid C string.
    unsafe { (api().string_new_with_utf8_chars)(dest, contents) };
}

fn string_name_new_with_utf8_chars(dest: StringNamePtr, contents: *const c_char) {
    // SAFETY: dest is owned opaque storage and contents is a valid C string.
    unsafe { (api().string_name_new_with_utf8_chars)(dest, contents) };
}

/// Reinterprets one `extern "C"` fn pointer as the pinned header's
/// concrete signature behind the same symbol.
///
/// # Safety
/// `get` is the engine's live resolver, `name` identifies that symbol, and
/// `T` is exactly the header signature for it.
unsafe fn lookup<T>(get: GetProcAddressFn, name: &'static CStr) -> Option<T> {
    // A size mismatch would make transmute_copy read past `generic`; the
    // branch folds away because both sizes are compile-time constants.
    if std::mem::size_of::<T>() != std::mem::size_of::<InterfaceFunction>() {
        fail!("GDExtension symbol {name:?} does not have a pointer-sized signature");
        return None;
    }
    // SAFETY: `get` is the live engine resolver and `name` is a valid C symbol.
    let Some(generic) = (unsafe { get(name.as_ptr()) }) else {
        fail!("cannot resolve GDExtension interface symbol {name:?}");
        return None;
    };
    // Safety: `T` is the header signature for `name`, and the size check
    // above bounds the copy.
    Some(unsafe { std::mem::transmute_copy::<InterfaceFunction, T>(&generic) })
}

// ── C structs (from gdextension_interface.h) ────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CallError {
    pub(crate) error: c_int,
    pub(crate) argument: i32,
    pub(crate) expected: i32,
}

#[repr(C)]
pub struct Initialization {
    minimum_initialization_level: c_int,
    userdata: *mut c_void,
    initialize: Option<InitializeCallbackFn>,
    deinitialize: Option<InitializeCallbackFn>,
}

/// Only create/free/get_virtual are non-null; the engine treats the rest as
/// "not provided".
#[repr(C)]
struct ClassCreationInfo4 {
    is_virtual: GDExtensionBool,
    is_abstract: GDExtensionBool,
    is_exposed: GDExtensionBool,
    is_runtime: GDExtensionBool,
    icon_path: *const c_void,
    set_func: *mut c_void,
    get_func: *mut c_void,
    get_property_list_func: *mut c_void,
    free_property_list_func: *mut c_void,
    property_can_revert_func: *mut c_void,
    property_get_revert_func: *mut c_void,
    validate_property_func: *mut c_void,
    notification_func: *mut c_void,
    to_string_func: *mut c_void,
    reference_func: *mut c_void,
    unreference_func: *mut c_void,
    create_instance_func: Option<CreateInstance2Fn>,
    free_instance_func: Option<FreeInstanceFn>,
    recreate_instance_func: *mut c_void,
    get_virtual_func: Option<GetVirtual2Fn>,
    get_virtual_call_data_func: *mut c_void,
    call_virtual_with_data_func: *mut c_void,
    class_userdata: *mut c_void,
}

/// `refresh` has no args, return value, or defaults — every pointer is null.
#[repr(C)]
struct ClassMethodInfo {
    name: StringNamePtr,
    method_userdata: *mut c_void,
    call_func: Option<ClassMethodCallFn>,
    ptrcall_func: *mut c_void,
    method_flags: u32,
    has_return_value: GDExtensionBool,
    return_value_info: *mut c_void,
    return_value_metadata: c_int,
    argument_count: u32,
    arguments_info: *mut c_void,
    arguments_metadata: *mut c_void,
    default_argument_count: u32,
    default_arguments: *mut c_void,
}

// ── compile-time layout pinning (drift guard) ───────────────────────────────
// The engine passes these structs by pointer; a layout mismatch would
// corrupt the registration silently. Pin each size to the header's 64-bit
// layout. `PropertyInfo` is not transcribed: it appears only as always-null
// `*mut c_void` fields, so there is no struct to drift.
const _: () = assert!(
    core::mem::size_of::<CallError>() == 12,
    "CallError must be 12 bytes"
);
const _: () = assert!(
    core::mem::size_of::<Initialization>() == 32,
    "Initialization must be 32 bytes"
);
const _: () = assert!(
    core::mem::size_of::<ClassCreationInfo4>() == 160,
    "ClassCreationInfo4 must be 160 bytes"
);
const _: () = assert!(
    core::mem::size_of::<ClassMethodInfo>() == 88,
    "ClassMethodInfo must be 88 bytes"
);

// ── process-wide state (single-threaded: the game's logic loop) ─────────────

/// One engine singleton's resolve cache: unresolved until the first [`get`](Self::get),
/// then resolved or failed (warned once, never retried).
#[derive(Default)]
struct SingletonCache {
    ptr: ObjectPtr,
    failed: bool,
}

impl SingletonCache {
    fn get(&mut self, name: ConstStringNamePtr, failure_warning: &str) -> Option<ObjectPtr> {
        if self.ptr.is_null() && !self.failed {
            self.ptr = global_get_singleton(name);
            if self.ptr.is_null() {
                self.failed = true;
                warn!("{failure_warning}");
            }
        }
        if self.ptr.is_null() {
            None
        } else {
            Some(self.ptr)
        }
    }
}

/// Mutable extension state, accessed only from the game thread. Interned
/// StringNames and the Input singleton/mouse cache are process-lifetime.
#[derive(Default)]
pub(crate) struct Global {
    library: ClassLibraryPtr,
    pub(crate) sn_control: StringNamePtr,
    pub(crate) sn_refresh: StringNamePtr,
    pub(crate) sn_draw: StringNamePtr,
    pub(crate) sn_set_visible: StringNamePtr,
    pub(crate) sn_get_viewport: StringNamePtr,
    pub(crate) sn_get_mouse_position: StringNamePtr,
    pub(crate) sn_set_position: StringNamePtr,
    pub(crate) sn_set_size: StringNamePtr,
    pub(crate) sn_input: StringNamePtr,
    pub(crate) sn_is_mouse_button_pressed: StringNamePtr,
    pub(crate) sn_queue_redraw: StringNamePtr,
    pub(crate) sn_draw_rect: StringNamePtr,
    pub(crate) sn_draw_string: StringNamePtr,
    pub(crate) sn_get_theme_default_font: StringNamePtr,
    pub(crate) sn_get_visible_rect: StringNamePtr,
    pub(crate) sn_set_clip_contents: StringNamePtr,
    pub(crate) sn_add_child: StringNamePtr,
    pub(crate) sn_set_mouse_filter: StringNamePtr,
    pub(crate) sn_resource_loader: StringNamePtr,
    pub(crate) sn_load: StringNamePtr,
    pub(crate) sn_style_box_texture: StringNamePtr,
    pub(crate) sn_set_texture: StringNamePtr,
    pub(crate) sn_set_texture_margin: StringNamePtr,
    pub(crate) sn_set_region_rect: StringNamePtr,
    pub(crate) sn_set_h_axis_stretch_mode: StringNamePtr,
    pub(crate) sn_set_v_axis_stretch_mode: StringNamePtr,
    pub(crate) sn_set_modulate: StringNamePtr,
    pub(crate) sn_draw_style_box: StringNamePtr,
    pub(crate) sn_draw_texture_rect: StringNamePtr,
    input: SingletonCache,
    resource_loader: SingletonCache,
    mouse_query_warned: bool,
    /// StringNames of methods whose call already failed (warn once each).
    warned: [usize; 32],
    warned_count: usize,
}

impl Global {
    /// `const` so the `thread_local!` initializer is a constant expression.
    const fn new() -> Global {
        Global {
            library: ptr::null_mut(),
            sn_control: ptr::null_mut(),
            sn_refresh: ptr::null_mut(),
            sn_draw: ptr::null_mut(),
            sn_set_visible: ptr::null_mut(),
            sn_get_viewport: ptr::null_mut(),
            sn_get_mouse_position: ptr::null_mut(),
            sn_set_position: ptr::null_mut(),
            sn_set_size: ptr::null_mut(),
            sn_input: ptr::null_mut(),
            sn_is_mouse_button_pressed: ptr::null_mut(),
            sn_queue_redraw: ptr::null_mut(),
            sn_draw_rect: ptr::null_mut(),
            sn_draw_string: ptr::null_mut(),
            sn_get_theme_default_font: ptr::null_mut(),
            sn_get_visible_rect: ptr::null_mut(),
            sn_set_clip_contents: ptr::null_mut(),
            sn_add_child: ptr::null_mut(),
            sn_set_mouse_filter: ptr::null_mut(),
            sn_resource_loader: ptr::null_mut(),
            sn_load: ptr::null_mut(),
            sn_style_box_texture: ptr::null_mut(),
            sn_set_texture: ptr::null_mut(),
            sn_set_texture_margin: ptr::null_mut(),
            sn_set_region_rect: ptr::null_mut(),
            sn_set_h_axis_stretch_mode: ptr::null_mut(),
            sn_set_v_axis_stretch_mode: ptr::null_mut(),
            sn_set_modulate: ptr::null_mut(),
            sn_draw_style_box: ptr::null_mut(),
            sn_draw_texture_rect: ptr::null_mut(),
            input: SingletonCache {
                ptr: ptr::null_mut(),
                failed: false,
            },
            resource_loader: SingletonCache {
                ptr: ptr::null_mut(),
                failed: false,
            },
            mouse_query_warned: false,
            warned: [0; 32],
            warned_count: 0,
        }
    }
}

thread_local! {
    pub(crate) static GLOBAL: RefCell<Global> = const { RefCell::new(Global::new()) };
}

static CLASSES: OnceLock<[EngineClass; 3]> = OnceLock::new();

/// The engine layer never sees the concrete panel type.
pub(crate) struct EngineClass {
    name: &'static CStr,
    /// The interned StringName, filled by [`init_string_names`].
    name_ptr: AtomicUsize,
    create: unsafe fn(Object) -> *mut c_void,
    free: unsafe fn(*mut c_void),
    draw: unsafe fn(*mut c_void),
    refresh: Option<unsafe fn(*mut c_void)>,
}

impl EngineClass {
    pub(crate) const fn new(
        name: &'static CStr,
        create: unsafe fn(Object) -> *mut c_void,
        free: unsafe fn(*mut c_void),
        draw: unsafe fn(*mut c_void),
        refresh: Option<unsafe fn(*mut c_void)>,
    ) -> Self {
        EngineClass {
            name,
            name_ptr: AtomicUsize::new(0),
            create,
            free,
            draw,
            refresh,
        }
    }

    fn name_ptr(&self) -> Option<StringNamePtr> {
        let ptr = self.name_ptr.load(Ordering::Relaxed);
        if ptr == 0 {
            None
        } else {
            Some(ptr as StringNamePtr)
        }
    }
}

pub(crate) fn object_id(object: Object) -> ObjectId {
    object_get_instance_id(object.0)
}

pub(crate) fn object_from_id(id: ObjectId) -> Option<Object> {
    let object = object_get_instance_from_id(id);
    (!object.is_null()).then_some(Object(object))
}

/// The owning class plus its state pointer.
struct Instance {
    class: &'static EngineClass,
    state: *mut c_void,
}

// Every engine callback runs through [`crate::abi::contain`]: a panic
// unwinding into the engine would crash the game.

/// What the shim's `ClassDB.Instantiate` does, minus the managed
/// round-trip: runs the class's own create path, so the instance binding
/// (and its later free) is indistinguishable from a shim-created panel.
/// The class resolves by name, never table position: an index that ever
/// drifted onto a panel class would recurse through `attach_children`
/// without terminating.
pub(crate) fn instantiate_class(name: &CStr) -> Option<Object> {
    let classes = CLASSES.get()?;
    let class = classes
        .iter()
        .find(|class| class.name.to_bytes() == name.to_bytes())?;
    let class_userdata = (class as *const EngineClass).cast_mut().cast::<c_void>();
    // Safety: identical to the engine invoking the callback; the function
    // is contain-wrapped and self-contained.
    let obj = unsafe { create_instance(class_userdata, 0) };
    if obj.is_null() {
        None
    } else {
        Some(Object(obj))
    }
}

/// The storage `Box` is deliberately leaked: StringName is interned and
/// engine-managed.
fn make_string_name(name: &'static CStr) -> StringNamePtr {
    let storage = Box::into_raw(Box::new(Opaque([0; OPAQUE_SIZE])));
    string_name_new_with_utf8_chars(storage.cast::<c_void>(), name.as_ptr());
    storage.cast::<c_void>()
}

fn init_string_names() {
    GLOBAL.with(|cell| {
        let mut g = cell.borrow_mut();
        g.sn_control = make_string_name(c"Control");
        for class in CLASSES
            .get()
            .expect("the class table is set at entry, before Scene init")
        {
            class
                .name_ptr
                .store(make_string_name(class.name) as usize, Ordering::Relaxed);
        }
        g.sn_refresh = make_string_name(c"refresh");
        g.sn_draw = make_string_name(c"_draw");
        g.sn_set_visible = make_string_name(c"set_visible");
        g.sn_get_viewport = make_string_name(c"get_viewport");
        g.sn_get_mouse_position = make_string_name(c"get_mouse_position");
        g.sn_set_position = make_string_name(c"set_position");
        g.sn_set_size = make_string_name(c"set_size");
        g.sn_input = make_string_name(c"Input");
        g.sn_is_mouse_button_pressed = make_string_name(c"is_mouse_button_pressed");
        g.sn_queue_redraw = make_string_name(c"queue_redraw");
        g.sn_draw_rect = make_string_name(c"draw_rect");
        g.sn_draw_string = make_string_name(c"draw_string");
        g.sn_get_theme_default_font = make_string_name(c"get_theme_default_font");
        g.sn_get_visible_rect = make_string_name(c"get_visible_rect");
        g.sn_set_clip_contents = make_string_name(c"set_clip_contents");
        g.sn_add_child = make_string_name(c"add_child");
        g.sn_set_mouse_filter = make_string_name(c"set_mouse_filter");
        g.sn_resource_loader = make_string_name(c"ResourceLoader");
        g.sn_load = make_string_name(c"load");
        g.sn_style_box_texture = make_string_name(c"StyleBoxTexture");
        g.sn_set_texture = make_string_name(c"set_texture");
        g.sn_set_texture_margin = make_string_name(c"set_texture_margin");
        g.sn_set_region_rect = make_string_name(c"set_region_rect");
        g.sn_set_h_axis_stretch_mode = make_string_name(c"set_h_axis_stretch_mode");
        g.sn_set_v_axis_stretch_mode = make_string_name(c"set_v_axis_stretch_mode");
        g.sn_set_modulate = make_string_name(c"set_modulate");
        g.sn_draw_style_box = make_string_name(c"draw_style_box");
        g.sn_draw_texture_rect = make_string_name(c"draw_texture_rect");
    });
}

/// Complete variant whose `Drop` runs [`variant_destroy`].
pub(crate) struct Variant(Box<Opaque>);

impl Variant {
    /// A constructed NIL, so Drop stays valid even when [`variant_call`]
    /// fails before assigning the return slot.
    pub(crate) fn nil() -> Variant {
        // Resolve before the wrapper exists: a failed resolve must unwind
        // through contain, not abort while Drop resolves a second time.
        let new_nil = api().variant_new_nil;
        let mut v = Variant(Box::new(Opaque([0; OPAQUE_SIZE])));
        // SAFETY: new_nil initializes the owned opaque storage as a valid NIL.
        unsafe { new_nil(v.ptr()) };
        v
    }

    /// Takes a reference, not a raw pointer, so a safe caller cannot smuggle
    /// a dangling or wrongly typed value across the FFI; the engine
    /// constructor resolves before the wrapper exists so a failed resolve
    /// cannot drop never-constructed storage.
    fn from_typed<T>(vtype: c_int, value: &T) -> Variant {
        let ctor = get_variant_from_type_constructor(vtype);
        let mut v = Variant(Box::new(Opaque([0; OPAQUE_SIZE])));
        // SAFETY: ctor matches vtype and value has that constructor's layout.
        unsafe { ctor(v.ptr(), (value as *const T).cast::<c_void>().cast_mut()) };
        v
    }

    pub(crate) fn from_object(object: ObjectPtr) -> Variant {
        Self::from_typed(VT_OBJECT, &object)
    }

    pub(crate) fn from_bool(value: bool) -> Variant {
        Self::from_typed(VT_BOOL, &value)
    }

    pub(crate) fn from_int(value: i64) -> Variant {
        Self::from_typed(VT_INT, &value)
    }

    pub(crate) fn from_float(value: f64) -> Variant {
        Self::from_typed(VT_FLOAT, &value)
    }

    pub(crate) fn from_vector2(value: &[f32; 2]) -> Variant {
        Self::from_typed(VT_VECTOR2, value)
    }

    pub(crate) fn from_rect2(value: &[f32; 4]) -> Variant {
        Self::from_typed(VT_RECT2, value)
    }

    pub(crate) fn from_color(value: &[f32; 4]) -> Variant {
        Self::from_typed(VT_COLOR, value)
    }

    pub(crate) fn ptr(&mut self) -> VariantPtr {
        self.0.0.as_mut_ptr().cast::<c_void>()
    }

    pub(crate) fn const_ptr(&self) -> ConstVariantPtr {
        self.0.0.as_ptr().cast::<c_void>()
    }

    pub(crate) fn storage(&self) -> &Opaque {
        &self.0
    }

    /// Moves the storage to a retained slot; dropping the result never
    /// destroys the engine Variant, which is exactly the object Ref.
    pub(crate) fn into_retained(self) -> RetainedVariant {
        let held = std::mem::ManuallyDrop::new(self);
        // Safety: `held` is never dropped, so the Box leaves with exactly
        // one owner.
        RetainedVariant(unsafe { std::ptr::read(&held.0) })
    }
}

impl Drop for Variant {
    fn drop(&mut self) {
        variant_destroy(self.ptr());
    }
}

/// Drop skips [`variant_destroy`]: the engine's Ref keeps the Font alive.
pub(crate) struct RetainedVariant(pub(crate) Box<Opaque>);

impl RetainedVariant {
    pub(crate) fn const_ptr(&self) -> ConstVariantPtr {
        self.0.0.as_ptr().cast::<c_void>()
    }

    /// The mutable pointer [`variant_call`] wants; it never mutates `self`.
    pub(crate) fn ptr(&self) -> VariantPtr {
        self.0.0.as_ptr().cast_mut().cast::<c_void>()
    }
}

/// Constructing the OBJECT Variant IS the reference; a nil Variant means
/// "could not retain", never a dangling ref.
pub(crate) fn retained_object(object: ObjectPtr) -> Option<RetainedVariant> {
    let variant = Variant::from_object(object);
    if variant_type(variant.storage()) != VT_OBJECT {
        return None;
    }
    Some(variant.into_retained())
}

pub(crate) fn variant_type(variant: &Opaque) -> c_int {
    variant_get_type(variant.0.as_ptr().cast::<c_void>())
}

/// A payload type and its Variant tag, paired so safe callers cannot read
/// one Variant type as another Rust type. Only this module defines pairs.
trait Payload: Copy {
    const VARIANT_TYPE: c_int;

    fn read(raw: *mut c_void) -> Option<Self>;
}

impl Payload for bool {
    const VARIANT_TYPE: c_int = VT_BOOL;

    fn read(raw: *mut c_void) -> Option<Self> {
        // The engine byte must be 0 or 1 for the result to be a valid bool.
        // SAFETY: the tag/type pairing gives a non-null payload of this layout.
        let byte = unsafe { raw.cast::<u8>().read_unaligned() };
        (byte <= 1).then_some(byte != 0)
    }
}

impl Payload for ObjectPtr {
    const VARIANT_TYPE: c_int = VT_OBJECT;

    fn read(raw: *mut c_void) -> Option<Self> {
        // SAFETY: the tag/type pairing gives a non-null payload of this layout.
        Some(unsafe { raw.cast::<ObjectPtr>().read_unaligned() })
    }
}

impl Payload for [f32; 2] {
    const VARIANT_TYPE: c_int = VT_VECTOR2;

    fn read(raw: *mut c_void) -> Option<Self> {
        // SAFETY: the tag/type pairing gives a non-null payload of this layout.
        Some(unsafe { raw.cast::<Self>().read_unaligned() })
    }
}

impl Payload for [f32; 4] {
    const VARIANT_TYPE: c_int = VT_RECT2;

    fn read(raw: *mut c_void) -> Option<Self> {
        // SAFETY: the tag/type pairing gives a non-null payload of this layout.
        Some(unsafe { raw.cast::<Self>().read_unaligned() })
    }
}

/// The payload pointer is only naturally aligned: `read_unaligned`, and
/// the tag/type pairing is checked first (the getter is UB on a mismatch).
fn read_payload<T: Payload>(variant: &Opaque) -> Option<T> {
    if variant_type(variant) != T::VARIANT_TYPE {
        return None;
    }
    let getter = variant_get_ptr_internal_getter(T::VARIANT_TYPE)?;
    // SAFETY: the Variant tag/type pairing is valid for this getter.
    let raw = unsafe { getter(variant.0.as_ptr().cast_mut().cast::<c_void>()) };
    if raw.is_null() {
        return None;
    }
    T::read(raw)
}

pub(crate) fn read_bool(variant: &Opaque) -> Option<bool> {
    read_payload(variant)
}

pub(crate) fn read_object(variant: &Opaque) -> Option<ObjectPtr> {
    read_payload(variant)
}

pub(crate) fn read_rect2(variant: &Opaque) -> Option<[f32; 4]> {
    read_payload(variant)
}

pub(crate) fn read_vector2(variant: &Opaque) -> Option<Vector2> {
    let value = read_payload::<[f32; 2]>(variant)?;
    Some(Vector2::new(value[0], value[1]))
}

std::thread_local! {
    static NUL_IN_TEXT_LOGGED: Cell<bool> = const { Cell::new(false) };
}

/// String is refcounted (unlike interned StringName): without the
/// ptr-destructor call one `_Data` would leak per draw_string.
pub(crate) fn string_variant(text: &str) -> Variant {
    // Ids re-enter from the persisted store, so a corrupted file can smuggle
    // in a NUL; a C string ends at the first NUL anyway, so truncate and
    // degrade rather than panic the draw path every frame.
    let end = text.find('\0').unwrap_or(text.len());
    if end != text.len() {
        crate::fail_once(
            &NUL_IN_TEXT_LOGGED,
            format_args!("draw text holds a NUL byte; truncating"),
        );
    }
    let text = &text[..end];
    let c = std::ffi::CString::new(text).expect("NUL-free after truncation");
    let mut storage = Opaque([0; OPAQUE_SIZE]);
    string_new_with_utf8_chars(storage.0.as_mut_ptr().cast::<c_void>(), c.as_ptr());
    let variant = Variant::from_typed(VT_STRING, &storage);
    let dtor = (*STRING_DTOR.get_or_init(|| variant_get_ptr_destructor(VT_STRING)))
        .expect("Godot 4.5 defines a ptr-destructor for the String variant type");
    // SAFETY: `storage` holds a live String from `string_new_with_utf8_chars`
    // and `dtor` is the VT_STRING ptr-destructor; destroyed in place once, the
    // inert stack bytes need no cleanup.
    unsafe { dtor(storage.0.as_mut_ptr().cast::<c_void>()) };
    variant
}

/// StringName is one pointer to interned `_Data`: compare the first 8 bytes.
///
/// # Safety
/// Non-null arguments point at live engine StringName storage.
unsafe fn string_name_eq(a: ConstStringNamePtr, b: ConstStringNamePtr) -> bool {
    if a.is_null() || b.is_null() {
        return false;
    }
    // Safety: the non-null arguments satisfy the function contract.
    unsafe { *(a as *const usize) == *(b as *const usize) }
}

/// ERROR-shaped: a CallError on a panel method is a real failure the
/// headless audit must catch, unlike the soft degradations below.
pub(crate) fn fail_call_failed(name: &'static str, method: ConstStringNamePtr) {
    GLOBAL.with(|cell| {
        let mut g = cell.borrow_mut();
        let addr = method as usize;
        let count = g.warned_count;
        if g.warned[..count].contains(&addr) {
            return;
        }
        if count < g.warned.len() {
            g.warned[count] = addr;
            g.warned_count = count + 1;
        }
        fail!("panel engine call failed: {name}");
    });
}

/// Resolved once; a failed resolve disables drag/tab clicks.
fn input_singleton() -> Option<ObjectPtr> {
    GLOBAL.with(|cell| {
        let mut g = cell.borrow_mut();
        let name = g.sn_input;
        g.input
            .get(name, "Input singleton not found; drag/tab clicks disabled")
    })
}

/// Only the left button is polled (held across frames, so reliable);
/// native input-event inspection hangs the engine fork.
pub(crate) fn mouse_button_pressed(button: i64) -> bool {
    let method = GLOBAL.with(|g| g.borrow().sn_is_mouse_button_pressed);
    if method.is_null() {
        // Off the init thread the cached StringName is null and variant_call
        // would null-deref engine-side; degrade instead of calling.
        return mouse_query_failed();
    }
    let Some(input) = input_singleton() else {
        return false;
    };
    let mut obj_v = Variant::from_object(input);
    let button_v = Variant::from_int(button);
    let mut ret = Variant::nil();
    let mut err = CallError {
        error: CALL_OK,
        argument: 0,
        expected: 0,
    };
    let args = [button_v.const_ptr()];
    variant_call(obj_v.ptr(), method, args.as_ptr(), 1, ret.ptr(), &mut err);
    if err.error != CALL_OK {
        return mouse_query_failed();
    }
    read_bool(ret.storage()).unwrap_or(false)
}

fn mouse_query_failed() -> bool {
    let warned = GLOBAL.with(|cell| {
        let mut g = cell.borrow_mut();
        let warned = g.mouse_query_warned;
        g.mouse_query_warned = true;
        warned
    });
    if !warned {
        warn!("Input.is_mouse_button_pressed call failed; drag/tab clicks disabled");
    }
    false
}

/// Resolved once; a failed resolve disables the theme.
pub(crate) fn resource_loader_singleton() -> Option<ObjectPtr> {
    GLOBAL.with(|cell| {
        let mut g = cell.borrow_mut();
        let name = g.sn_resource_loader;
        g.resource_loader.get(
            name,
            "ResourceLoader singleton not found; game theme assets disabled",
        )
    })
}

pub(crate) use crate::engine::object::Object;

/// The symbol named in `spire_profiler.gdextension`.
///
/// # Safety
/// The engine calls exactly once per load with a live procedure resolver,
/// library handle, and writable initialization.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gdextension_entry(
    get_proc_address: GetProcAddressFn,
    library: ClassLibraryPtr,
    initialization: *mut Initialization,
) -> GDExtensionBool {
    contain("gdextension_entry", 0, || {
        // SAFETY: the engine supplies a live resolver and this entry has valid arguments.
        let Some(resolved) = (unsafe { Api::resolve(get_proc_address) }) else {
            fail!("cannot resolve GDExtension interface");
            return 0;
        };
        if API.set(resolved).is_err() {
            // A second load would double-register the classes.
            fail!("GDExtension entry called twice");
            return 0;
        }
        // The composition root supplies the classes before any callback.
        let _ = CLASSES.set(crate::registration::engine_classes());
        GLOBAL.with(|cell| cell.borrow_mut().library = library);
        // Safety: the engine hands us a valid, writable Initialization.
        unsafe {
            (*initialization).minimum_initialization_level = INIT_LEVEL_SCENE;
            (*initialization).initialize = Some(on_initialize);
            (*initialization).deinitialize = Some(on_deinitialize);
        }
        1
    })
}

/// # Safety
/// The engine invokes its stored callback with valid userdata.
unsafe extern "C" fn on_initialize(_userdata: *mut c_void, level: c_int) {
    contain("on_initialize", (), || {
        if level != INIT_LEVEL_SCENE {
            return;
        }
        init_string_names();
        let (library, control) = GLOBAL.with(|cell| {
            let g = cell.borrow();
            (g.library, g.sn_control)
        });
        for class in CLASSES.get().expect("the class table is set at entry") {
            register_class(library, control, class);
        }
        // Kept so headless runs can verify the class registered.
        marker!("panel class registered");
    });
}

/// # Safety
/// The engine invokes its stored callback with valid userdata.
unsafe extern "C" fn on_deinitialize(_userdata: *mut c_void, level: c_int) {
    contain("on_deinitialize", (), || {
        if level != INIT_LEVEL_SCENE {
            return;
        }
        let library = GLOBAL.with(|cell| cell.borrow().library);
        for class in CLASSES.get().expect("the class table is set at entry") {
            let class_name = class
                .name_ptr()
                .expect("the class name was interned at Scene init");
            classdb_unregister_extension_class(library, class_name);
        }
    });
}

/// Interns the class name so the shim's `ClassDB.Instantiate` resolves; the
/// concrete panel type lives in the class's callback table.
fn register_class(
    library: ClassLibraryPtr,
    parent: ConstStringNamePtr,
    class: &'static EngineClass,
) {
    let class_name = class
        .name_ptr()
        .expect("the class name was interned at Scene init");
    let class_userdata = (class as *const EngineClass).cast_mut().cast::<c_void>();
    let info = ClassCreationInfo4 {
        is_virtual: 0,
        is_abstract: 0,
        is_exposed: 1,
        is_runtime: 0,
        icon_path: ptr::null(),
        set_func: ptr::null_mut(),
        get_func: ptr::null_mut(),
        get_property_list_func: ptr::null_mut(),
        free_property_list_func: ptr::null_mut(),
        property_can_revert_func: ptr::null_mut(),
        property_get_revert_func: ptr::null_mut(),
        validate_property_func: ptr::null_mut(),
        notification_func: ptr::null_mut(),
        to_string_func: ptr::null_mut(),
        reference_func: ptr::null_mut(),
        unreference_func: ptr::null_mut(),
        create_instance_func: Some(create_instance),
        free_instance_func: Some(free_instance),
        recreate_instance_func: ptr::null_mut(),
        get_virtual_func: Some(get_virtual),
        get_virtual_call_data_func: ptr::null_mut(),
        call_virtual_with_data_func: ptr::null_mut(),
        class_userdata,
    };
    classdb_register_extension_class5(library, class_name, parent, &info);

    if class.refresh.is_none() {
        return;
    }
    let method = ClassMethodInfo {
        name: GLOBAL.with(|g| g.borrow().sn_refresh),
        method_userdata: class_userdata,
        call_func: Some(refresh_call),
        ptrcall_func: ptr::null_mut(),
        method_flags: METHOD_FLAG_NORMAL,
        has_return_value: 0,
        return_value_info: ptr::null_mut(),
        return_value_metadata: 0,
        argument_count: 0,
        arguments_info: ptr::null_mut(),
        arguments_metadata: ptr::null_mut(),
        default_argument_count: 0,
        default_arguments: ptr::null_mut(),
    };
    classdb_register_extension_class_method(library, class_name, &method);
}

// ── generic class callbacks (instance lifecycle, virtual dispatch, refresh) ──
// Every callback is class-agnostic: the concrete panel type lives in the
// EngineClass callback table.

/// # Safety
/// `class_userdata` points to a live `EngineClass` installed in `CLASSES`.
unsafe extern "C" fn create_instance(
    class_userdata: *mut c_void,
    _notify_postinitialize: GDExtensionBool,
) -> ObjectPtr {
    // SAFETY: every caller passes a `&'static EngineClass` from the class
    // table: the engine returns the registered class_userdata, and
    // `instantiate_class` reads the same table.
    let class = unsafe { &*class_userdata.cast::<EngineClass>() };
    let label = class
        .name
        .to_str()
        .expect("registered class names are UTF-8");
    contain(&format!("{label} create_instance"), ptr::null_mut(), || {
        let control = GLOBAL.with(|g| g.borrow().sn_control);
        if control.is_null() {
            fail!("{label} instantiated before Scene init on this thread");
            return ptr::null_mut();
        }
        let obj = classdb_construct_object(control);
        if obj.is_null() {
            return ptr::null_mut();
        }
        // A panic inside `create` is swallowed but leaks the constructed
        // Control: the engine only learns of the instance through
        // `object_set_instance`, which the panic skips. Accepted — a
        // cleanup path would risk a double free.
        // SAFETY: the class create callback accepts this live engine object.
        let state = unsafe { (class.create)(Object(obj)) };
        let instance = Box::into_raw(Box::new(Instance { class, state })).cast::<c_void>();
        let class_name = class
            .name_ptr()
            .expect("the class name was interned at Scene init");
        object_set_instance(obj, class_name, instance);
        obj
    })
}

/// # Safety
/// `class_userdata` is the create-class pointer; non-null `instance` is its
/// live `object_set_instance` pointer.
unsafe extern "C" fn free_instance(class_userdata: *mut c_void, instance: ClassInstancePtr) {
    // SAFETY: every caller passes a `&'static EngineClass` from the class
    // table: the engine returns the registered class_userdata, and
    // `instantiate_class` reads the same table.
    let class = unsafe { &*class_userdata.cast::<EngineClass>() };
    let label = class
        .name
        .to_str()
        .expect("registered class names are UTF-8");
    contain(&format!("{label} free_instance"), (), || {
        if instance.is_null() {
            return;
        }
        // Safety: the engine passes back the object_set_instance pointer for
        // this instance; the class's free drops the panel state.
        let header = unsafe { Box::from_raw(instance.cast::<Instance>()) };
        // A panic inside `free` is swallowed: the state pointer is never
        // freed. Accepted — the state drops only once, so a catch path would
        // risk a double free.
        // SAFETY: the Instance pairs each class with the state its own
        // create boxed; free runs once per object, so this frees a live
        // pointer exactly once.
        unsafe { (header.class.free)(header.state) };
    });
}

/// # Safety
/// `name` is live engine StringName storage, or null.
unsafe extern "C" fn get_virtual(
    _class_userdata: *mut c_void,
    name: ConstStringNamePtr,
    _hash: u32,
) -> Option<ClassCallVirtualFn> {
    contain("get_virtual", None, || {
        let draw = GLOBAL.with(|g| g.borrow().sn_draw);
        // SAFETY: both names are live engine storage or null.
        if unsafe { string_name_eq(name, draw) } {
            Some(draw_virtual)
        } else {
            None
        }
    })
}

/// # Safety
/// Non-null `instance` is the live `object_set_instance` pointer for an
/// `Instance`.
unsafe extern "C" fn draw_virtual(
    instance: ClassInstancePtr,
    _args: *const ConstTypePtr,
    _ret: TypePtr,
) {
    // Read the owning class before the boundary so the label names the panel.
    let label = if instance.is_null() {
        "panel"
    } else {
        // Safety: instance is the object_set_instance pointer for a live panel.
        unsafe {
            (*instance.cast::<Instance>())
                .class
                .name
                .to_str()
                .expect("registered class names are UTF-8")
        }
    };
    contain(&format!("{label} _draw"), (), || {
        if instance.is_null() {
            return;
        }
        // SAFETY: `instance` is non-null (checked above) and the live
        // `object_set_instance` pointer for an `Instance`.
        let header = unsafe { &*instance.cast::<Instance>() };
        // SAFETY: the Instance pairs each class with the state its own
        // create boxed; the object lives, so the state is not yet freed.
        unsafe { (header.class.draw)(header.state) };
    });
}

/// # Safety
/// `method_userdata` points to a live `EngineClass`; non-null `instance` is
/// its live instance pointer; `error_out`, if non-null, is writable.
unsafe extern "C" fn refresh_call(
    method_userdata: *mut c_void,
    instance: ClassInstancePtr,
    _args: *const ConstVariantPtr,
    _arg_count: GDExtensionInt,
    _ret: VariantPtr,
    error_out: *mut CallError,
) {
    // SAFETY: method_userdata is the `&'static EngineClass` registered as
    // the refresh method's userdata.
    let class = unsafe { &*method_userdata.cast::<EngineClass>() };
    let Some(refresh) = class.refresh else {
        return;
    };
    let label = class
        .name
        .to_str()
        .expect("registered class names are UTF-8");
    if !error_out.is_null() {
        // SAFETY: `error_out` is non-null and a writable engine out-param
        // for the duration of this call.
        unsafe {
            (*error_out).error = CALL_OK;
            (*error_out).argument = 0;
            (*error_out).expected = 0;
        }
    }
    contain(&format!("{label} refresh"), (), || {
        if instance.is_null() {
            return;
        }
        // Safety: instance is the object_set_instance pointer for a live panel.
        let header = unsafe { &*instance.cast::<Instance>() };
        // SAFETY: the Instance pairs each class with the state its own
        // create boxed; the object lives, so the state is not yet freed.
        unsafe { refresh(header.state) };
    });
}
