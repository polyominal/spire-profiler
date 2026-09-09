//! Hand-rolled minimal GDExtension FFI — the crate's engine-facing surface:
//! a direct binding to the engine's `GDExtensionInterface` (vendored header
//! `vendor/gdextension_interface.h`, Godot 4.5.1). Exports
//! [`gdextension_entry`] and registers the panel classes as `Control`
//! subclasses (one virtual, `_draw`, plus the zero-argument `refresh` the
//! shim calls every frame); each parent instantiates the shared child class
//! through [`instantiate_class`].
//!
//! # Native code never inspects engine input events — scroll arrives from the shim
//!
//! The extension registers NO `_gui_input` virtual and never touches an
//! engine `InputEvent`: any GDExtension-originated call about an
//! engine-created input event hangs the game's proprietary engine fork,
//! while the fork's own managed→native calls are fine. Scroll arrives
//! from the shim instead: it connects each panel's `GuiInput` signal to
//! the C export [`crate::abi::spire_profiler_scroll_input`], `refresh`
//! consumes the queued pixels, and the scroll math stays native
//! ([`crate::ui::scroll`]).
//!
//! # Binding mechanics
//!
//! The engine API resolves by C name at runtime: [`gdextension_entry`]
//! looks up every interface function through `get_proc_address`; a
//! missing symbol fails loudly by name. `_draw` dispatch is
//! `get_virtual2`, keyed on the interned StringName pointer. Nothing
//! verifies Rust→engine method names — grep `extension_api_4.5.1.json`
//! before wiring a new call: a stale name fails loudly at the call site.
//!
//! # Engine-call technique (variant_call everywhere)
//!
//! [`variant_call`] needs none of the exact signature hashes
//! `object_method_bind_ptrcall` requires (published only in
//! `extension_api.json`). Values built with
//! [`get_variant_from_type_constructor`] are read back with
//! [`variant_get_ptr_internal_getter`] after a [`variant_get_type`] tag
//! check (the internal getter is undefined behavior on a type mismatch);
//! every temporary Variant is [`variant_destroy`]ed on drop. Return slots
//! contain no engine value before the call; normal return establishes the
//! initialized Variant even when method dispatch reports an error.
//!
//! # Where this module's unsafe lives
//!
//! One of the three `unsafe_code` allow sites `lib.rs` names (its allow
//! sits in `engine.rs`). Calls target only objects the extension created
//! (the panel Controls) or engine-owned singletons; the unsafe lives in:
//!
//! 1. **Raw engine pointers** — `*mut c_void` the engine supplies, each valid for its call's
//!    duration: the interned StringName pointer read in [`string_name_eq`], the variant payload
//!    bytes copied out via [`variant_get_ptr_internal_getter`], the class/instance user-data
//!    reborrowed in [`create_instance`]/[`refresh_call`]/[`free_instance`]/[`draw_virtual`], and
//!    the [`Initialization`]/[`CallError`] out-params written in
//!    [`gdextension_entry`]/[`refresh_call`].
//! 2. **The export and C callbacks**: `extern "C"` functions whose pointers the engine stores and
//!    calls back into. Every callback enters [`crate::abi::contain`] with a constant label before
//!    inspecting callback data, so ordinary unwinding panics are contained at the boundary.
//! 3. **Function-pointer resolution** — `get_proc_address` returns an opaque pointer that
//!    [`lookup`] reinterprets only as the pinned header's signature for that symbol.
//!
//! # Trusted engine-layout facts
//!
//! The deployed engine must uphold these premises beyond the pinned
//! Godot 4.5.1 header's opaque-pointer declarations. A game or engine update
//! requires rechecking them; the header alone cannot establish them:
//!
//! * Variant, String, and StringName fit in [`OPAQUE_SIZE`] bytes with alignment ≤ 16.
//! * StringName is one pointer to interned storage, so comparing the first 8 bytes of two live
//!   names is name equality.
//! * Internal-getter payloads match the [`read_payload`] reads: bool is one 0/1 byte, Vector2
//!   `[f32; 2]`, Rect2 `[f32; 4]`.
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
//! * ObjectDB lookup, ID queries and value constructors do not dispatch callbacks. Foreign objects
//!   cannot be destroyed concurrently with these engine-thread operations; a resolved pointer stays
//!   live through the immediately following constructor or ID query. Engine method dispatch keeps
//!   its receiver and arguments live until their last use.
//! * The enum values transcribed from the pinned header match it: the `VT_*` Variant type tags,
//!   `CALL_OK`, `INIT_LEVEL_SCENE`, `METHOD_FLAG_NORMAL`, and `MOUSE_BUTTON_LEFT`. Struct layouts
//!   carry compile-time size pins; scalar values cannot, so a pin bump re-checks them by hand.
//!
//! [`Object`] stores an engine id and resolves it before every use. It and
//! engine Variants cannot move between threads; call paths also reject a
//! thread other than the initialization thread. Retained resources keep an
//! unreleased reference increment, not a borrowed Variant byte buffer.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ptr;
use std::rc::Rc;
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
#[cfg(test)]
const CALL_ERROR_INVALID_METHOD: c_int = 1;
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
/// MaybeUninit admits foreign padding and inactive payload bytes.
#[repr(C, align(16))]
struct Opaque(MaybeUninit<[u8; OPAQUE_SIZE]>);
const OPAQUE_SIZE: usize = 64;

impl Opaque {
    fn new() -> Self {
        Self(MaybeUninit::uninit())
    }

    fn ptr(&mut self) -> *mut c_void {
        self.0.as_mut_ptr().cast()
    }

    fn const_ptr(&self) -> *const c_void {
        self.0.as_ptr().cast()
    }
}

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
    variant_get_type: unsafe extern "C" fn(ConstVariantPtr) -> c_int,
    variant_get_object_instance_id: unsafe extern "C" fn(ConstVariantPtr) -> ObjectId,
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
                variant_get_type: lookup(get, c"variant_get_type")?,
                variant_get_object_instance_id: lookup(get, c"variant_get_object_instance_id")?,
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
fn classdb_construct_object(name: ConstStringNamePtr) -> ObjectPtr {
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
    receiver: &mut Variant,
    method: ConstStringNamePtr,
    args: &[ConstVariantPtr],
) -> (Variant, CallError) {
    debug_assert!(
        on_engine_thread(),
        "Variant dispatch requires the initialization thread"
    );
    debug_assert!(
        !method.is_null(),
        "method lookup must succeed before engine dispatch"
    );
    let call = api().variant_call;
    let mut storage = Box::new(Opaque::new());
    let mut error = CallError {
        error: CALL_OK,
        argument: 0,
        expected: 0,
    };
    let args_ptr = if args.is_empty() {
        ptr::null()
    } else {
        args.as_ptr()
    };
    // SAFETY: receiver and arguments remain initialized for this call. The
    // private aligned destination contains no engine value. The interface
    // constructs its return Variant on every normal return, including errors;
    // only then is storage given the destructor-owning Variant wrapper.
    unsafe {
        call(
            receiver.ptr(),
            method,
            args_ptr,
            args.len() as GDExtensionInt,
            storage.ptr(),
            &mut error,
        )
    };
    (Variant::initialized(storage), error)
}

fn variant_destroy(p: VariantPtr) {
    // SAFETY: the API is resolved; every pointer argument is valid for this call.
    unsafe { (api().variant_destroy)(p) };
}

fn variant_get_type(p: ConstVariantPtr) -> c_int {
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
    object: Option<Object>,
    failed: bool,
}

impl SingletonCache {
    fn get(&mut self, name: ConstStringNamePtr, failure_warning: &str) -> Option<Object> {
        if !on_engine_thread() || name.is_null() {
            return None;
        }
        if self.object.is_none() && !self.failed {
            let raw = global_get_singleton(name);
            self.object = (!raw.is_null()).then(|| Object::from_id(object_get_instance_id(raw)));
            if self.object.is_none() {
                self.failed = true;
                warn!("{failure_warning}");
            }
        }
        self.object
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
                object: None,
                failed: false,
            },
            resource_loader: SingletonCache {
                object: None,
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
    create: fn(Object) -> *mut c_void,
    free: unsafe fn(*mut c_void),
    draw: unsafe fn(*mut c_void),
    refresh: Option<unsafe fn(*mut c_void)>,
}

impl EngineClass {
    pub(crate) const fn new(
        name: &'static CStr,
        create: fn(Object) -> *mut c_void,
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
    object.id()
}

pub(crate) fn object_from_id(id: ObjectId) -> Option<Object> {
    if !on_engine_thread() {
        return None;
    }
    let object = object_get_instance_from_id(id);
    (!object.is_null()).then_some(Object::from_id(id))
}

pub(crate) fn construct_object(class: ConstStringNamePtr) -> Option<Object> {
    if !on_engine_thread() || class.is_null() {
        return None;
    }
    let object = classdb_construct_object(class);
    (!object.is_null()).then(|| Object::from_id(object_get_instance_id(object)))
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
    if !on_engine_thread() {
        return None;
    }
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
        Some(Object::from_id(object_get_instance_id(obj)))
    }
}

/// The storage `Box` is deliberately leaked: StringName is interned and
/// engine-managed.
fn make_string_name(name: &'static CStr) -> StringNamePtr {
    let storage = Box::into_raw(Box::new(Opaque::new()));
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
pub(crate) struct Variant {
    storage: Option<Box<Opaque>>,
    _thread: PhantomData<Rc<()>>,
}

impl Variant {
    fn initialized(storage: Box<Opaque>) -> Self {
        Self {
            storage: Some(storage),
            _thread: PhantomData,
        }
    }

    /// Only the fixed typed constructors pair tags with input layouts.
    /// Unconstructed storage has no engine destructor during unwinding.
    fn from_typed<T>(vtype: c_int, value: &T) -> Variant {
        debug_assert!(
            on_engine_thread(),
            "engine values are constructed on the initialization thread"
        );
        let ctor = get_variant_from_type_constructor(vtype);
        let mut storage = Box::new(Opaque::new());
        // SAFETY: this module pairs each tag with its pinned input layout.
        // The constructor copies the live source without modifying/retaining
        // it and initializes the exclusive, aligned destination. MaybeUninit
        // allows the foreign value's padding bytes to remain uninitialized.
        unsafe {
            ctor(
                storage.ptr(),
                (value as *const T).cast::<c_void>().cast_mut(),
            )
        };
        Self::initialized(storage)
    }

    pub(crate) fn from_object(object: Object) -> Option<Variant> {
        if !on_engine_thread() {
            return None;
        }
        let ctor = get_variant_from_type_constructor(VT_OBJECT);
        let mut storage = Box::new(Opaque::new());
        let object = object_get_instance_from_id(object.id());
        if object.is_null() {
            return None;
        }
        // SAFETY: ObjectDB just resolved this id on the engine thread, with
        // no intervening engine call. The constructor copies this ObjectPtr
        // local into exclusive aligned storage and acquires the resource Ref.
        unsafe { ctor(storage.ptr(), (&raw const object).cast_mut().cast()) };
        Some(Self::initialized(storage))
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

    fn ptr(&mut self) -> VariantPtr {
        self.storage
            .as_mut()
            .expect("a live Variant owns its initialized storage")
            .ptr()
    }

    pub(crate) fn const_ptr(&self) -> ConstVariantPtr {
        self.storage().const_ptr()
    }

    fn storage(&self) -> &Opaque {
        self.storage
            .as_ref()
            .expect("only into_retained removes Variant storage")
    }

    pub(crate) fn object(&self) -> Option<Object> {
        if !on_engine_thread() {
            return None;
        }
        if variant_type(self.storage()) != VT_OBJECT {
            return None;
        }
        // SAFETY: self owns an initialized OBJECT Variant. The interface
        // reads its stored id without dereferencing the referenced object.
        let id = unsafe { (api().variant_get_object_instance_id)(self.const_ptr()) };
        (id != 0).then_some(Object::from_id(id))
    }

    pub(crate) fn rect2(&self) -> Option<[f32; 4]> {
        if !on_engine_thread() {
            return None;
        }
        read_payload(self.storage())
    }

    pub(crate) fn vector2(&self) -> Option<Vector2> {
        if !on_engine_thread() {
            return None;
        }
        let value = read_payload::<[f32; 2]>(self.storage())?;
        Some(Vector2::new(value[0], value[1]))
    }

    /// The reference increment outlives its byte storage: skipping the engine
    /// destructor deliberately pins the resource until process exit.
    pub(crate) fn into_retained(mut self) -> Option<RetainedVariant> {
        let object = self.object()?;
        drop(self.storage.take());
        Some(RetainedVariant { object })
    }
}

impl Drop for Variant {
    fn drop(&mut self) {
        if let Some(mut storage) = self.storage.take() {
            variant_destroy(storage.ptr());
        }
    }
}

/// A resource pinned by an intentionally unreleased engine reference.
pub(crate) struct RetainedVariant {
    object: Object,
}

impl RetainedVariant {
    pub(crate) fn object(&self) -> Object {
        self.object
    }
}

/// Selected resource classes are RefCounted; retention omits their unref.
pub(crate) fn retained_object(object: Object) -> Option<RetainedVariant> {
    Variant::from_object(object)?.into_retained()
}

fn variant_type(variant: &Opaque) -> c_int {
    variant_get_type(variant.const_ptr())
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
    let raw = unsafe { getter(variant.const_ptr().cast_mut()) };
    if raw.is_null() {
        return None;
    }
    T::read(raw)
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
    let mut storage = Opaque::new();
    string_new_with_utf8_chars(storage.ptr(), c.as_ptr());
    let variant = Variant::from_typed(VT_STRING, &storage);
    let dtor = (*STRING_DTOR.get_or_init(|| variant_get_ptr_destructor(VT_STRING)))
        .expect("Godot 4.5 defines a ptr-destructor for the String variant type");
    // SAFETY: `storage` holds a live String from `string_new_with_utf8_chars`
    // and `dtor` is the VT_STRING ptr-destructor; destroyed in place once, the
    // inert stack bytes need no cleanup.
    unsafe { dtor(storage.ptr()) };
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
fn input_singleton() -> Option<Object> {
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
    if !on_engine_thread() {
        return false;
    }
    let method = GLOBAL.with(|g| g.borrow().sn_is_mouse_button_pressed);
    if method.is_null() {
        // Off the init thread the cached StringName is null and variant_call
        // would null-deref engine-side; degrade instead of calling.
        return mouse_query_failed();
    }
    let Some(input) = input_singleton() else {
        return false;
    };
    let button_v = Variant::from_int(button);
    let Some(mut obj_v) = Variant::from_object(input) else {
        return mouse_query_failed();
    };
    let args = [button_v.const_ptr()];
    let (ret, err) = variant_call(&mut obj_v, method, &args);
    if err.error != CALL_OK {
        return mouse_query_failed();
    }
    read_payload::<bool>(ret.storage()).unwrap_or(false)
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
pub(crate) fn resource_loader_singleton() -> Option<Object> {
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

#[cfg(test)]
#[path = "gdext_tests.rs"]
mod tests;

static ENGINE_THREAD: OnceLock<std::thread::ThreadId> = OnceLock::new();

pub(crate) fn on_engine_thread() -> bool {
    ENGINE_THREAD
        .get()
        .is_some_and(|id| *id == std::thread::current().id())
}

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
        if ENGINE_THREAD.set(std::thread::current().id()).is_err() {
            fail!("GDExtension entry called twice");
            return 0;
        }
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
            (*initialization).userdata = ptr::null_mut();
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
        if !on_engine_thread() {
            fail!("on_initialize called outside the engine thread");
            return;
        }
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
        if !on_engine_thread() {
            fail!("on_deinitialize called outside the engine thread");
            return;
        }
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

// Callbacks check thread affinity before touching thread-confined handles.

/// # Safety
/// `class_userdata` points to a live `EngineClass` installed in `CLASSES`.
unsafe extern "C" fn create_instance(
    class_userdata: *mut c_void,
    _notify_postinitialize: GDExtensionBool,
) -> ObjectPtr {
    contain("create_instance", ptr::null_mut(), || {
        if !on_engine_thread() {
            fail!("create_instance called outside the engine thread");
            return ptr::null_mut();
        }
        // SAFETY: the engine or instantiate_class passes the registered
        // static class table entry; it outlives every engine call below.
        let class = unsafe { &*class_userdata.cast::<EngineClass>() };
        let control = GLOBAL.with(|g| g.borrow().sn_control);
        if control.is_null() {
            fail!("panel instantiated before Scene init");
            return ptr::null_mut();
        }
        let obj = classdb_construct_object(control);
        if obj.is_null() {
            return ptr::null_mut();
        }
        // A panic before object_set_instance leaks the constructed Control:
        // engine ownership during failed creation does not permit safe retry.
        let object = Object::from_id(object_get_instance_id(obj));
        let state = (class.create)(object);
        let obj = object_get_instance_from_id(object.id());
        if obj.is_null() {
            // SAFETY: create returned this matching owned handle on the engine
            // thread; no binding exposed it to the engine's free callback.
            unsafe { (class.free)(state) };
            fail!("panel freed during creation; unbound state released");
            return ptr::null_mut();
        }
        let instance = Box::into_raw(Box::new(Instance { class, state })).cast::<c_void>();
        let class_name = class
            .name_ptr()
            .expect("the class name was interned at Scene init");
        object_set_instance(obj, class_name, instance);
        object_get_instance_from_id(object.id())
    })
}

/// # Safety
/// Non-null `instance` is an unreleased `Box<Instance>` pointer published by
/// `create_instance`, freed once; no later callback may use it. Wrong-thread
/// free leaves its handle leaked.
unsafe extern "C" fn free_instance(_class_userdata: *mut c_void, instance: ClassInstancePtr) {
    contain("free_instance", (), || {
        if !on_engine_thread() {
            fail!("free_instance called outside the engine thread; state leaked");
            return;
        }
        if instance.is_null() {
            return;
        }
        // SAFETY: the engine returns the exact live Box pointer once. The
        // other callbacks retain no reference into this header across calls.
        let header = unsafe { Box::from_raw(instance.cast::<Instance>()) };
        // SAFETY: the header pairs the matching create/free and live state;
        // the thread guard permits dropping its thread-confined Rc handles.
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
        if !on_engine_thread() {
            fail!("get_virtual called outside the engine thread");
            return None;
        }
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
/// Non-null `instance` is an unreleased `Box<Instance>` pointer published by
/// `create_instance`, still live when this callback enters.
unsafe extern "C" fn draw_virtual(
    instance: ClassInstancePtr,
    _args: *const ConstTypePtr,
    _ret: TypePtr,
) {
    contain("draw_virtual", (), || {
        if !on_engine_thread() {
            fail!("draw_virtual called outside the engine thread");
            return;
        }
        if instance.is_null() {
            return;
        }
        // SAFETY: the live header is read before dispatch can reenter free;
        // only copied function/state pointers survive the short borrow.
        let (draw, state) = unsafe {
            let header = &*instance.cast::<Instance>();
            (header.class.draw, header.state)
        };
        // SAFETY: the class table pairs draw with this live state type.
        unsafe { draw(state) };
    });
}

/// # Safety
/// `method_userdata` points to the registered `EngineClass`; non-null
/// `instance` is that class's unreleased `Box<Instance>` pointer published by
/// `create_instance`, live at entry; `error_out`, if non-null, is writable
/// for the call.
unsafe extern "C" fn refresh_call(
    method_userdata: *mut c_void,
    instance: ClassInstancePtr,
    _args: *const ConstVariantPtr,
    _arg_count: GDExtensionInt,
    _ret: VariantPtr,
    error_out: *mut CallError,
) {
    contain("refresh_call", (), || {
        if !error_out.is_null() {
            // SAFETY: the non-null engine out-param is writable for this call.
            unsafe {
                error_out.write(CallError {
                    error: CALL_OK,
                    argument: 0,
                    expected: 0,
                })
            };
        }
        if !on_engine_thread() {
            fail!("refresh_call called outside the engine thread");
            return;
        }
        if instance.is_null() {
            return;
        }
        // SAFETY: the static method table entry outlives this callback.
        let class = unsafe { &*method_userdata.cast::<EngineClass>() };
        let Some(refresh) = class.refresh else { return };
        // SAFETY: copy the live header's state pointer before dispatch can
        // reenter free. No reference into the header survives dispatch.
        let state = unsafe { (*instance.cast::<Instance>()).state };
        // SAFETY: method_userdata and instance name the same registered class.
        unsafe { refresh(state) };
    });
}
