//! Safe method dispatch on engine [`Object`] pointers — the layer the panel
//! modules call. Every method goes through the Variant machinery in
//! [`crate::engine::gdext`], which checks each `CallError`; the raw pointer
//! is never dereferenced here (all raw reads live in
//! [`crate::engine::gdext`]'s helpers), so this is the engine layer's
//! only unsafe-free module.

use std::ffi::{c_int, c_void};
use std::ptr;

use crate::engine::gdext::{
    CALL_ERROR_INVALID_METHOD, CALL_OK, CallError, ConstStringNamePtr, ConstVariantPtr,
    GDExtensionInt, GLOBAL, OPAQUE_SIZE, ObjectPtr, Opaque, RetainedVariant, VT_BOOL, VT_COLOR,
    VT_FLOAT, VT_INT, VT_OBJECT, VT_RECT2, VT_VECTOR2, Variant, fail_call_failed, read_payload,
    read_vector2, resource_loader_singleton, retained_object, string_variant, variant_call,
    variant_destroy, variant_type,
};
use crate::engine::math::{Color, Rect2, Vector2};

/// `pub(crate)` so panel modules can wrap the engine pointers their calls
/// hand back.
#[derive(Clone, Copy)]
pub(crate) struct Object(pub(crate) ObjectPtr);

/// The engine clips to `width` regardless of alignment; `LeftClipped`
/// clips glyphs at `pos.x + w` (the pinned-header backstop), `Right` ends
/// them there (a longer string clips left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextAlign {
    Left,
    LeftClipped(f32),
    Right(f32),
    Center(f32),
}

/// `Control.MouseFilter.IGNORE` in the engine's wire representation.
const MOUSE_FILTER_IGNORE: i64 = 2;

impl TextAlign {
    fn engine_args(self) -> (i64, f64) {
        match self {
            TextAlign::Left => (0, -1.0),
            TextAlign::LeftClipped(w) => (0, f64::from(w)),
            TextAlign::Center(w) => (1, f64::from(w)),
            TextAlign::Right(w) => (2, f64::from(w)),
        }
    }
}

impl Object {
    /// A non-OK code is reported once per method.
    fn call(
        self,
        name: &'static str,
        method: ConstStringNamePtr,
        args: &[ConstVariantPtr],
        ret: &mut Variant,
    ) -> c_int {
        if method.is_null() {
            // Off the init thread every cached name is null and handing the
            // engine a null method name is an engine-side null deref; report
            // once and fail the call instead.
            fail_call_failed(name, method);
            return CALL_ERROR_INVALID_METHOD;
        }
        let mut obj_v =
            Variant::from_value(VT_OBJECT, (&self.0 as *const ObjectPtr).cast::<c_void>());
        let mut err = CallError {
            error: CALL_OK,
            argument: 0,
            expected: 0,
        };
        let args_ptr: *const ConstVariantPtr = if args.is_empty() {
            ptr::null()
        } else {
            args.as_ptr()
        };
        variant_call(
            obj_v.ptr(),
            method,
            args_ptr,
            args.len() as GDExtensionInt,
            ret.ptr(),
            &mut err,
        );
        if err.error != CALL_OK {
            fail_call_failed(name, method);
        }
        err.error
    }

    pub(crate) fn set_position(self, pos: Vector2) {
        let value = [pos.x, pos.y];
        let arg = Variant::from_value(VT_VECTOR2, (&value as *const [f32; 2]).cast::<c_void>());
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_set_position);
        self.call("set_position", method, &[arg.const_ptr()], &mut ret);
    }

    pub(crate) fn set_size(self, size: Vector2) {
        let value = [size.x, size.y];
        let arg = Variant::from_value(VT_VECTOR2, (&value as *const [f32; 2]).cast::<c_void>());
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_set_size);
        self.call("set_size", method, &[arg.const_ptr()], &mut ret);
    }

    pub(crate) fn set_visible(self, visible: bool) {
        let value = visible;
        let arg = Variant::from_value(VT_BOOL, (&value as *const bool).cast::<c_void>());
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_set_visible);
        self.call("set_visible", method, &[arg.const_ptr()], &mut ret);
    }

    /// Clips this Control's canvas item to its own rect.
    pub(crate) fn set_clip_contents(self, clip: bool) {
        let value = clip;
        let arg = Variant::from_value(VT_BOOL, (&value as *const bool).cast::<c_void>());
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_set_clip_contents);
        self.call("set_clip_contents", method, &[arg.const_ptr()], &mut ret);
    }

    /// The child must never swallow the parent's gui input.
    pub(crate) fn set_mouse_filter_ignore(self) {
        let value = MOUSE_FILTER_IGNORE;
        let arg = Variant::from_value(VT_INT, (&value as *const i64).cast::<c_void>());
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_set_mouse_filter);
        self.call("set_mouse_filter", method, &[arg.const_ptr()], &mut ret);
    }

    pub(crate) fn add_child(self, child: Object) {
        let arg = Variant::from_value(VT_OBJECT, (&child.0 as *const ObjectPtr).cast::<c_void>());
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_add_child);
        self.call("add_child", method, &[arg.const_ptr()], &mut ret);
    }

    pub(crate) fn queue_redraw(self) {
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_queue_redraw);
        self.call("queue_redraw", method, &[], &mut ret);
    }

    pub(crate) fn draw_rect(self, rect: Rect2, color: Color) -> bool {
        let r = [rect.position.x, rect.position.y, rect.size.x, rect.size.y];
        let c = color.as_array();
        let filled = true;
        let rect_v = Variant::from_value(VT_RECT2, (&r as *const [f32; 4]).cast::<c_void>());
        let color_v = Variant::from_value(VT_COLOR, (&c as *const [f32; 4]).cast::<c_void>());
        let filled_v = Variant::from_value(VT_BOOL, (&filled as *const bool).cast::<c_void>());
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_draw_rect);
        self.call(
            "draw_rect",
            method,
            &[
                rect_v.const_ptr(),
                color_v.const_ptr(),
                filled_v.const_ptr(),
            ],
            &mut ret,
        ) == CALL_OK
    }

    pub(crate) fn draw_string(
        self,
        font: &RetainedVariant,
        pos: Vector2,
        text: &str,
        align: TextAlign,
        size: i32,
        color: Color,
    ) -> bool {
        let text_v = string_variant(text);
        let pos_value = [pos.x, pos.y];
        let pos_v =
            Variant::from_value(VT_VECTOR2, (&pos_value as *const [f32; 2]).cast::<c_void>());
        let (alignment, width) = align.engine_args();
        let font_size: i64 = i64::from(size);
        let color_arr = color.as_array();
        let align_v = Variant::from_value(VT_INT, (&alignment as *const i64).cast::<c_void>());
        let width_v = Variant::from_value(VT_FLOAT, (&width as *const f64).cast::<c_void>());
        let size_v = Variant::from_value(VT_INT, (&font_size as *const i64).cast::<c_void>());
        let color_v =
            Variant::from_value(VT_COLOR, (&color_arr as *const [f32; 4]).cast::<c_void>());
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_draw_string);
        self.call(
            "draw_string",
            method,
            &[
                font.const_ptr(),
                pos_v.const_ptr(),
                text_v.const_ptr(),
                align_v.const_ptr(),
                width_v.const_ptr(),
                size_v.const_ptr(),
                color_v.const_ptr(),
            ],
            &mut ret,
        ) == CALL_OK
    }

    /// The engine slices the self-constructed StyleBoxTexture.
    pub(crate) fn draw_style_box(self, style_box: &RetainedVariant, rect: Rect2) -> bool {
        let r = [rect.position.x, rect.position.y, rect.size.x, rect.size.y];
        let rect_v = Variant::from_value(VT_RECT2, (&r as *const [f32; 4]).cast::<c_void>());
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_draw_style_box);
        self.call(
            "draw_style_box",
            method,
            &[style_box.const_ptr(), rect_v.const_ptr()],
            &mut ret,
        ) == CALL_OK
    }

    pub(crate) fn draw_texture_rect(
        self,
        texture: &RetainedVariant,
        rect: Rect2,
        tile: bool,
        modulate: Color,
    ) -> bool {
        let r = [rect.position.x, rect.position.y, rect.size.x, rect.size.y];
        let m = modulate.as_array();
        let rect_v = Variant::from_value(VT_RECT2, (&r as *const [f32; 4]).cast::<c_void>());
        let tile_v = Variant::from_value(VT_BOOL, (&tile as *const bool).cast::<c_void>());
        let modulate_v = Variant::from_value(VT_COLOR, (&m as *const [f32; 4]).cast::<c_void>());
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_draw_texture_rect);
        self.call(
            "draw_texture_rect",
            method,
            &[
                texture.const_ptr(),
                rect_v.const_ptr(),
                tile_v.const_ptr(),
                modulate_v.const_ptr(),
            ],
            &mut ret,
        ) == CALL_OK
    }

    pub(crate) fn get_viewport(self) -> Option<Object> {
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_get_viewport);
        if self.call("get_viewport", method, &[], &mut ret) != CALL_OK {
            return None;
        }
        let ptr = read_payload::<ObjectPtr>(VT_OBJECT, ret.storage())?;
        if ptr.is_null() {
            return None;
        }
        Some(Object(ptr))
    }

    /// The object Ref keeps the Font alive; `None` on a failed call.
    pub(crate) fn get_theme_default_font(self) -> Option<RetainedVariant> {
        let method = GLOBAL.with(|g| g.borrow().sn_get_theme_default_font);
        if method.is_null() {
            // Off the init thread the StringName cache is empty; None routes
            // into the caller's once-only "font unavailable" warning.
            return None;
        }
        let mut obj_v =
            Variant::from_value(VT_OBJECT, (&self.0 as *const ObjectPtr).cast::<c_void>());
        let mut err = CallError {
            error: CALL_OK,
            argument: 0,
            expected: 0,
        };
        let mut ret = Box::new(Opaque([0; OPAQUE_SIZE]));
        variant_call(
            obj_v.ptr(),
            method,
            ptr::null(),
            0,
            ret.0.as_mut_ptr().cast::<c_void>(),
            &mut err,
        );
        if err.error != CALL_OK || variant_type(&ret) != VT_OBJECT {
            variant_destroy(ret.0.as_mut_ptr().cast::<c_void>());
            return None;
        }
        Some(RetainedVariant(ret))
    }

    pub(crate) fn get_visible_rect(self) -> Option<Rect2> {
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_get_visible_rect);
        if self.call("get_visible_rect", method, &[], &mut ret) != CALL_OK {
            return None;
        }
        let value = read_payload::<[f32; 4]>(VT_RECT2, ret.storage())?;
        Some(Rect2::new(
            Vector2::new(value[0], value[1]),
            Vector2::new(value[2], value[3]),
        ))
    }

    pub(crate) fn get_mouse_position(self) -> Option<Vector2> {
        let mut ret = Variant::uninit();
        let method = GLOBAL.with(|g| g.borrow().sn_get_mouse_position);
        if self.call("get_mouse_position", method, &[], &mut ret) != CALL_OK {
            return None;
        }
        read_vector2(ret.storage())
    }
}

// ── theme asset acquisition (ResourceLoader + self-constructed StyleBoxTexture) ──

/// A missing resource returns None and the caller's tri-state falls back;
/// a CallError is reported once (a wrong method name is a bug).
pub(crate) fn resource_load(path: &str) -> Option<RetainedVariant> {
    let loader = resource_loader_singleton()?;
    let method = GLOBAL.with(|g| g.borrow().sn_load);
    if method.is_null() {
        // Same off-init-thread guard as `Object::call`.
        return None;
    }
    let mut obj_v = Variant::from_value(VT_OBJECT, (&loader as *const ObjectPtr).cast::<c_void>());
    let path_v = string_variant(path);
    let mut err = CallError {
        error: CALL_OK,
        argument: 0,
        expected: 0,
    };
    let mut ret = Box::new(Opaque([0; OPAQUE_SIZE]));
    let args = [path_v.const_ptr()];
    variant_call(
        obj_v.ptr(),
        method,
        args.as_ptr(),
        1,
        ret.0.as_mut_ptr().cast::<c_void>(),
        &mut err,
    );
    if err.error != CALL_OK {
        fail_call_failed("ResourceLoader.load", method);
        variant_destroy(ret.0.as_mut_ptr().cast::<c_void>());
        return None;
    }
    if variant_type(&ret) != VT_OBJECT {
        variant_destroy(ret.0.as_mut_ptr().cast::<c_void>());
        return None;
    }
    Some(RetainedVariant(ret))
}

const SIDES: [i64; 4] = [0, 1, 2, 3];

pub(crate) struct StyleBoxSpec<'a> {
    pub texture: &'a RetainedVariant,
    pub region: [f32; 4],
    pub margins: [f32; 4],
    pub tiled: bool,
    pub modulate: [f32; 4],
}

/// `set_texture` stores a Ref; a failed setter fails the construction,
/// since a half-configured stylebox would draw garbage.
pub(crate) fn construct_style_box(spec: &StyleBoxSpec) -> Option<RetainedVariant> {
    let class = GLOBAL.with(|g| g.borrow().sn_style_box_texture);
    if class.is_null() {
        return None;
    }
    let raw = crate::engine::gdext::classdb_construct_object(class);
    if raw.is_null() {
        return None;
    }
    let style_box = retained_object(raw)?;

    let region_v =
        Variant::from_value(VT_RECT2, (&spec.region as *const [f32; 4]).cast::<c_void>());
    let modulate_v = Variant::from_value(
        VT_COLOR,
        (&spec.modulate as *const [f32; 4]).cast::<c_void>(),
    );
    let stretch: i64 = i64::from(spec.tiled); // AxisStretchMode: 0=STRETCH, 1=TILE
    let stretch_v = Variant::from_value(VT_INT, (&stretch as *const i64).cast::<c_void>());
    let mut ok = call_retained(
        &style_box,
        "set_texture",
        GLOBAL.with(|g| g.borrow().sn_set_texture),
        &[spec.texture.const_ptr()],
    );
    ok &= call_retained(
        &style_box,
        "set_region_rect",
        GLOBAL.with(|g| g.borrow().sn_set_region_rect),
        &[region_v.const_ptr()],
    );
    for (side, margin) in SIDES.into_iter().zip(spec.margins) {
        let side_v = Variant::from_value(VT_INT, (&side as *const i64).cast::<c_void>());
        let size = f64::from(margin);
        let size_v = Variant::from_value(VT_FLOAT, (&size as *const f64).cast::<c_void>());
        ok &= call_retained(
            &style_box,
            "set_texture_margin",
            GLOBAL.with(|g| g.borrow().sn_set_texture_margin),
            &[side_v.const_ptr(), size_v.const_ptr()],
        );
    }
    ok &= call_retained(
        &style_box,
        "set_h_axis_stretch_mode",
        GLOBAL.with(|g| g.borrow().sn_set_h_axis_stretch_mode),
        &[stretch_v.const_ptr()],
    );
    ok &= call_retained(
        &style_box,
        "set_v_axis_stretch_mode",
        GLOBAL.with(|g| g.borrow().sn_set_v_axis_stretch_mode),
        &[stretch_v.const_ptr()],
    );
    ok &= call_retained(
        &style_box,
        "set_modulate",
        GLOBAL.with(|g| g.borrow().sn_set_modulate),
        &[modulate_v.const_ptr()],
    );
    // A failed construction leaks the fresh resource (RetainedVariant never
    // destroys) — leak by design beats risking a live unref.
    ok.then_some(style_box)
}

/// Setter dispatch on a retained Variant.
fn call_retained(
    retained: &RetainedVariant,
    name: &'static str,
    method: ConstStringNamePtr,
    args: &[ConstVariantPtr],
) -> bool {
    if method.is_null() {
        // Off-init-thread guard, mirroring `Object::call`.
        fail_call_failed(name, method);
        return false;
    }
    let mut ret = Variant::uninit();
    let mut err = CallError {
        error: CALL_OK,
        argument: 0,
        expected: 0,
    };
    let args_ptr: *const ConstVariantPtr = if args.is_empty() {
        ptr::null()
    } else {
        args.as_ptr()
    };
    variant_call(
        retained.ptr(),
        method,
        args_ptr,
        args.len() as GDExtensionInt,
        ret.ptr(),
        &mut err,
    );
    if err.error != CALL_OK {
        fail_call_failed(name, method);
        return false;
    }
    true
}
