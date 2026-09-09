//! Engine identities resolve at each call, so deleting an object between
//! calls produces a failed lookup. Variant arguments and receivers use the
//! same resolution path. Call paths reject off-thread access before creating
//! engine values; [`crate::engine::gdext::CallError`] also rejects failed method dispatch.

use std::marker::PhantomData;
use std::rc::Rc;

use crate::engine::gdext::{
    CALL_OK, ConstStringNamePtr, ConstVariantPtr, GLOBAL, ObjectId, RetainedVariant, Variant,
    construct_object, fail_call_failed, on_engine_thread, resource_loader_singleton,
    retained_object, string_variant, variant_call,
};
use crate::engine::math::{Color, Rect2, Vector2};

/// Identity survives engine deletion; every use resolves the current object.
#[derive(Clone, Copy)]
pub(crate) struct Object {
    id: ObjectId,
    _thread: PhantomData<Rc<()>>,
}

/// The engine clips to `width` regardless of alignment; [`LeftClipped`](Self::LeftClipped)
/// clips glyphs at `pos.x + w` (the pinned-header backstop), [`Right`](Self::Right) ends
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
    pub(crate) const fn from_id(id: ObjectId) -> Self {
        Self {
            id,
            _thread: PhantomData,
        }
    }

    pub(crate) const fn id(self) -> ObjectId {
        self.id
    }

    /// A non-OK code is reported once per method.
    fn call(
        self,
        name: &'static str,
        method: ConstStringNamePtr,
        args: &[ConstVariantPtr],
    ) -> Option<Variant> {
        if !on_engine_thread() || method.is_null() {
            // Off the init thread every cached name is null and handing the
            // engine a null method name is an engine-side null deref; report
            // once and fail the call instead.
            fail_call_failed(name, method);
            return None;
        }
        let Some(mut obj_v) = Variant::from_object(self) else {
            fail_call_failed(name, method);
            return None;
        };
        let (ret, err) = variant_call(&mut obj_v, method, args);
        if err.error != CALL_OK {
            fail_call_failed(name, method);
            return None;
        }
        Some(ret)
    }

    pub(crate) fn set_position(self, pos: Vector2) {
        if !on_engine_thread() {
            return;
        }
        let value = [pos.x, pos.y];
        let arg = Variant::from_vector2(&value);
        let method = GLOBAL.with(|g| g.borrow().sn_set_position);
        self.call("set_position", method, &[arg.const_ptr()]);
    }

    pub(crate) fn set_size(self, size: Vector2) {
        if !on_engine_thread() {
            return;
        }
        let value = [size.x, size.y];
        let arg = Variant::from_vector2(&value);
        let method = GLOBAL.with(|g| g.borrow().sn_set_size);
        self.call("set_size", method, &[arg.const_ptr()]);
    }

    pub(crate) fn set_visible(self, visible: bool) {
        if !on_engine_thread() {
            return;
        }
        let arg = Variant::from_bool(visible);
        let method = GLOBAL.with(|g| g.borrow().sn_set_visible);
        self.call("set_visible", method, &[arg.const_ptr()]);
    }

    /// Clips this Control's canvas item to its own rect.
    pub(crate) fn set_clip_contents(self, clip: bool) {
        if !on_engine_thread() {
            return;
        }
        let arg = Variant::from_bool(clip);
        let method = GLOBAL.with(|g| g.borrow().sn_set_clip_contents);
        self.call("set_clip_contents", method, &[arg.const_ptr()]);
    }

    /// The child must never swallow the parent's gui input.
    pub(crate) fn set_mouse_filter_ignore(self) {
        if !on_engine_thread() {
            return;
        }
        let arg = Variant::from_int(MOUSE_FILTER_IGNORE);
        let method = GLOBAL.with(|g| g.borrow().sn_set_mouse_filter);
        self.call("set_mouse_filter", method, &[arg.const_ptr()]);
    }

    pub(crate) fn add_child(self, child: Object) {
        if !on_engine_thread() {
            return;
        }
        let Some(arg) = Variant::from_object(child) else {
            return;
        };
        let method = GLOBAL.with(|g| g.borrow().sn_add_child);
        self.call("add_child", method, &[arg.const_ptr()]);
    }

    pub(crate) fn queue_redraw(self) {
        if !on_engine_thread() {
            return;
        }
        let method = GLOBAL.with(|g| g.borrow().sn_queue_redraw);
        self.call("queue_redraw", method, &[]);
    }

    pub(crate) fn draw_rect(self, rect: Rect2, color: Color) -> bool {
        if !on_engine_thread() {
            return false;
        }
        let r = [rect.position.x, rect.position.y, rect.size.x, rect.size.y];
        let c = color.as_array();
        let rect_v = Variant::from_rect2(&r);
        let color_v = Variant::from_color(&c);
        let filled_v = Variant::from_bool(true);
        let method = GLOBAL.with(|g| g.borrow().sn_draw_rect);
        self.call(
            "draw_rect",
            method,
            &[
                rect_v.const_ptr(),
                color_v.const_ptr(),
                filled_v.const_ptr(),
            ],
        )
        .is_some()
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
        if !on_engine_thread() {
            return false;
        }
        let Some(font_v) = Variant::from_object(font.object()) else {
            return false;
        };
        let text_v = string_variant(text);
        let pos_value = [pos.x, pos.y];
        let pos_v = Variant::from_vector2(&pos_value);
        let (alignment, width) = align.engine_args();
        let color_arr = color.as_array();
        let align_v = Variant::from_int(alignment);
        let width_v = Variant::from_float(width);
        let size_v = Variant::from_int(i64::from(size));
        let color_v = Variant::from_color(&color_arr);
        let method = GLOBAL.with(|g| g.borrow().sn_draw_string);
        self.call(
            "draw_string",
            method,
            &[
                font_v.const_ptr(),
                pos_v.const_ptr(),
                text_v.const_ptr(),
                align_v.const_ptr(),
                width_v.const_ptr(),
                size_v.const_ptr(),
                color_v.const_ptr(),
            ],
        )
        .is_some()
    }

    /// The engine slices the self-constructed StyleBoxTexture.
    pub(crate) fn draw_style_box(self, style_box: &RetainedVariant, rect: Rect2) -> bool {
        if !on_engine_thread() {
            return false;
        }
        let Some(style_box_v) = Variant::from_object(style_box.object()) else {
            return false;
        };
        let r = [rect.position.x, rect.position.y, rect.size.x, rect.size.y];
        let rect_v = Variant::from_rect2(&r);
        let method = GLOBAL.with(|g| g.borrow().sn_draw_style_box);
        self.call(
            "draw_style_box",
            method,
            &[style_box_v.const_ptr(), rect_v.const_ptr()],
        )
        .is_some()
    }

    pub(crate) fn draw_texture_rect(
        self,
        texture: &RetainedVariant,
        rect: Rect2,
        tile: bool,
        modulate: Color,
    ) -> bool {
        if !on_engine_thread() {
            return false;
        }
        let Some(texture_v) = Variant::from_object(texture.object()) else {
            return false;
        };
        let r = [rect.position.x, rect.position.y, rect.size.x, rect.size.y];
        let m = modulate.as_array();
        let rect_v = Variant::from_rect2(&r);
        let tile_v = Variant::from_bool(tile);
        let modulate_v = Variant::from_color(&m);
        let method = GLOBAL.with(|g| g.borrow().sn_draw_texture_rect);
        self.call(
            "draw_texture_rect",
            method,
            &[
                texture_v.const_ptr(),
                rect_v.const_ptr(),
                tile_v.const_ptr(),
                modulate_v.const_ptr(),
            ],
        )
        .is_some()
    }

    pub(crate) fn get_viewport(self) -> Option<Object> {
        if !on_engine_thread() {
            return None;
        }
        let method = GLOBAL.with(|g| g.borrow().sn_get_viewport);
        self.call("get_viewport", method, &[])?.object()
    }

    /// The object Ref keeps the Font alive; `None` on a failed call.
    pub(crate) fn get_theme_default_font(self) -> Option<RetainedVariant> {
        if !on_engine_thread() {
            return None;
        }
        let method = GLOBAL.with(|g| g.borrow().sn_get_theme_default_font);
        self.call("get_theme_default_font", method, &[])?
            .into_retained()
    }

    pub(crate) fn get_visible_rect(self) -> Option<Rect2> {
        if !on_engine_thread() {
            return None;
        }
        let method = GLOBAL.with(|g| g.borrow().sn_get_visible_rect);
        let ret = self.call("get_visible_rect", method, &[])?;
        let value = ret.rect2()?;
        Some(Rect2::new(
            Vector2::new(value[0], value[1]),
            Vector2::new(value[2], value[3]),
        ))
    }

    pub(crate) fn get_mouse_position(self) -> Option<Vector2> {
        if !on_engine_thread() {
            return None;
        }
        let method = GLOBAL.with(|g| g.borrow().sn_get_mouse_position);
        self.call("get_mouse_position", method, &[])?.vector2()
    }
}

// ── theme asset acquisition (ResourceLoader + self-constructed StyleBoxTexture) ──

/// A missing resource returns None and the caller's tri-state falls back;
/// a CallError is reported once (a wrong method name is a bug).
pub(crate) fn resource_load(path: &str) -> Option<RetainedVariant> {
    if !on_engine_thread() {
        return None;
    }
    let method = GLOBAL.with(|g| g.borrow().sn_load);
    if method.is_null() {
        return None;
    }
    let loader = resource_loader_singleton()?;
    let path_v = string_variant(path);
    loader
        .call("ResourceLoader.load", method, &[path_v.const_ptr()])?
        .into_retained()
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
    if !on_engine_thread() {
        return None;
    }
    let class = GLOBAL.with(|g| g.borrow().sn_style_box_texture);
    let style_box = retained_object(construct_object(class)?)?;

    let region_v = Variant::from_rect2(&spec.region);
    let modulate_v = Variant::from_color(&spec.modulate);
    let texture_v = Variant::from_object(spec.texture.object())?;
    // AxisStretchMode: 0=STRETCH, 1=TILE
    let stretch_v = Variant::from_int(i64::from(spec.tiled));
    let mut ok = style_box.call(
        "set_texture",
        GLOBAL.with(|g| g.borrow().sn_set_texture),
        &[texture_v.const_ptr()],
    );
    ok &= style_box.call(
        "set_region_rect",
        GLOBAL.with(|g| g.borrow().sn_set_region_rect),
        &[region_v.const_ptr()],
    );
    for (side, margin) in SIDES.into_iter().zip(spec.margins) {
        let side_v = Variant::from_int(side);
        let size_v = Variant::from_float(f64::from(margin));
        ok &= style_box.call(
            "set_texture_margin",
            GLOBAL.with(|g| g.borrow().sn_set_texture_margin),
            &[side_v.const_ptr(), size_v.const_ptr()],
        );
    }
    ok &= style_box.call(
        "set_h_axis_stretch_mode",
        GLOBAL.with(|g| g.borrow().sn_set_h_axis_stretch_mode),
        &[stretch_v.const_ptr()],
    );
    ok &= style_box.call(
        "set_v_axis_stretch_mode",
        GLOBAL.with(|g| g.borrow().sn_set_v_axis_stretch_mode),
        &[stretch_v.const_ptr()],
    );
    ok &= style_box.call(
        "set_modulate",
        GLOBAL.with(|g| g.borrow().sn_set_modulate),
        &[modulate_v.const_ptr()],
    );
    // A failed construction leaks the fresh resource (RetainedVariant never
    // destroys) — leak by design beats risking a live unref.
    ok.then_some(style_box)
}

impl RetainedVariant {
    fn call(
        &self,
        name: &'static str,
        method: ConstStringNamePtr,
        args: &[ConstVariantPtr],
    ) -> bool {
        self.object().call(name, method, args).is_some()
    }
}
