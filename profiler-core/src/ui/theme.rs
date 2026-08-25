//! The game-native theme: the panels' typefaces, chrome plate, scrollbar
//! sprites, and tab sprites, loaded at runtime out of the shipped PCK by
//! `res://` path. Nothing is bundled with the mod.
//!
//! # Fonts
//!
//! The game sets no project-wide theme or font: every label picks a
//! FontVariation (a letter-tracking wrapper over the TTFs in
//! `res://fonts/`) plus an explicit pixel size, and font oversampling is
//! off project-wide, so loaded variations rasterize like native text.
//! The panels run the native scale: 32px Kreon Bold gs2 headers/numbers,
//! 24px Kreon Regular gs1 body text, 22px tooltip text. Where the game
//! uses Kreon Bold gs1 (24px lead-ins, tab labels), the panels draw the
//! gs2 face: the 1px tracking delta is not worth a third font asset.
//!
//! # Palette
//!
//! The colors are verbatim game colors from `StsColors.cs` or shipped
//! scenes, and live in [`crate::ui::palette`]. The two chart sections run
//! distinct temperature families — warm hues for damage (harm), cool for
//! defense (protection) — so a bar's leading segment names its section.
//! [`crate::ui::palette::slot_color`] is the single resolution point:
//! bars, legend chips, and tooltip values all call it, so no two views
//! can drift. Text shadows follow the game's offsets.
//!
//! # Metrics
//!
//! UI lives in the game's fixed 1920×1080 virtual canvas space
//! (`stretch/mode = canvas_items`, aspect expand): hard-coded pixel
//! offsets scale and letterbox exactly like native UI, so no DPI
//! handling. The plate is the game's hover-tip nine-patch (margins
//! pinned from `hover_tip.tscn`: L55/T43/R91/B32, tiled), its shadow
//! 8px down-right at 25% black. [`content_box`] is the single
//! content-area computation: the plate body (box minus the shadow inset)
//! further inset by the content padding L22/T16/R37/B20, minus the 32px
//! scrollbar gutter while the bar shows. The right pad is asymmetric on
//! purpose — the plate's right nine-patch slice is 91px against 55px
//! left, and the scene compensates with the wider right margin. In the
//! flat fallback the insets reduce to the pre-plate 12px geometry.
//!
//! # Acquisition
//!
//! Loads dispatch on the ResourceLoader engine singleton with
//! `variant_call("load", [path])` — the Input singleton's daily-proven
//! shape, no new C ABI. Loaded resources are engine-created, so they only
//! ever pass as Variant arguments, and each is retained forever: an OBJECT
//! Variant holds a real ref, so never destroying it is the panel-lifetime
//! leak-by-design the font fetch already lives by. Loads happen lazily on
//! the first draw, already on the game thread.
//!
//! # Fallback policy
//!
//! Every asset is an independent tri-state: a failed load warns once per
//! asset — never an ERROR, the headless gate fails on those — and only
//! that element degrades (failed font → theme default, failed chrome →
//! flat rects, failed scrollbar → none, wheel still works). A missing
//! asset never disables a panel. The one-shot `theme assets: N/M loaded`
//! line aids real-play diagnosis.

use std::cell::Cell;

use crate::engine::gdext::RetainedVariant;
use crate::engine::math::{Rect2, Vector2};
use crate::engine::object;
use crate::{marker, warn};

/// Kreon Bold, 2px glyph tracking.
pub(crate) const FONT_TITLE_PATH: &str = "res://themes/kreon_bold_glyph_space_two.tres";

/// Kreon Regular, 1px glyph tracking.
pub(crate) const FONT_BODY_PATH: &str = "res://themes/kreon_regular_glyph_space_one.tres";

pub(crate) const PLATE_PATH: &str = "res://images/ui/hover_tip.png";
/// Overshoot samples the transparent padding.
pub(crate) const PLATE_REGION: [f32; 4] = [0.0, 0.0, 339.0, 107.0];
pub(crate) const PLATE_MARGINS: [f32; 4] = [55.0, 43.0, 91.0, 32.0];

// Margins exceeding the region draw garbage; pin the fit at compile time.
const _: () = assert!(PLATE_MARGINS[0] + PLATE_MARGINS[2] < PLATE_REGION[2]);
const _: () = assert!(PLATE_MARGINS[1] + PLATE_MARGINS[3] < PLATE_REGION[3]);
pub(crate) const PLATE_SHADOW_MODULATE: [f32; 4] = [0.0, 0.0, 0.0, 0.25098];
/// Down-right hang past the plate.
pub(crate) const PLATE_SHADOW_OFFSET: f32 = 8.0;

#[allow(dead_code)]
pub(crate) const BACKDROP_ALPHA: f32 = 0.8;

pub(crate) const SCROLL_TRACK_CENTER_PATH: &str =
    "res://images/atlases/ui_atlas.sprites/scrollbar_track_center.tres";
pub(crate) const SCROLL_TRACK_EDGE_PATH: &str =
    "res://images/atlases/ui_atlas.sprites/scrollbar_track_edge2.tres";
pub(crate) const SCROLL_TRAIN_PATH: &str =
    "res://images/atlases/ui_atlas.sprites/scrollbar_train_large.tres";
/// The art is white, so the color is all modulation.
pub(crate) const SCROLL_TRACK_MODULATE: [f32; 4] = [0.164706, 0.290196, 0.321569, 1.0];

// NO unselected sprite: the scene draws the same TabImage in both states;
// selection is the stroke's visibility plus the label's modulate.

pub(crate) const TAB_PLATE_PATH: &str =
    "res://images/atlases/ui_atlas.sprites/settings_tab_selected.tres";
pub(crate) const TAB_STROKE_PATH: &str =
    "res://images/atlases/ui_atlas.sprites/settings_tab_stroke.tres";
pub(crate) const TAB_ART_SIZE: [f32; 2] = [515.0, 181.0];
/// The scene's HSV value-0.9 recolor is an 0.9 RGB multiply.
pub(crate) const TAB_PLATE_MODULATE: [f32; 4] = [0.9, 0.9, 0.9, 1.0];
/// The scene draws ADDITIVE; alpha-blending is the near-equivalent.
pub(crate) const TAB_STROKE_MODULATE: [f32; 4] = [0.3648, 0.9104, 0.96, 0.752941];

/// The run-history page's deselected-icon treatment (HSV s=0.3, v=0.55)
/// as a plain multiply: scaling HSV value by 0.55 is exactly an 0.55 RGB
/// multiply; the desaturation is not expressible in a multiply.
pub(crate) const AVATAR_DIM_MODULATE: [f32; 4] = [0.55, 0.55, 0.55, 1.0];

const ASSET_COUNT: usize = 8;

// The portrait is per-run DATA, not a static theme asset.

/// Five real characters plus placeholders; overflow fails loud.
const DYNAMIC_CACHE_CAP: usize = 8;

pub(crate) const SIZE_HEADER: i32 = 32;
pub(crate) const SIZE_BODY: i32 = 24;
/// The tooltip's wrap budgets derive at exactly this size.
pub(crate) const SIZE_TOOLTIP: i32 = 22;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRole {
    Title,
    Body,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconId {
    Character(u8),
    TabPlate,
    TabStroke,
}

/// A failed load warns once and falls back.
pub(crate) enum AssetState {
    Unfetched,
    Failed,
    Loaded(RetainedVariant),
}

impl AssetState {
    fn is_loaded(&self) -> bool {
        matches!(self, AssetState::Loaded(_))
    }
}

pub(crate) struct Plate {
    pub(crate) body: RetainedVariant,
    pub(crate) shadow: RetainedVariant,
}

enum PlateState {
    Unfetched,
    Failed,
    Loaded(Plate),
}

pub(crate) struct ScrollbarSprites<'a> {
    pub center: &'a RetainedVariant,
    pub edge: &'a RetainedVariant,
    pub train: &'a RetainedVariant,
}

pub(crate) struct Theme {
    font_title: AssetState,
    font_body: AssetState,
    plate: PlateState,
    scroll_track_center: AssetState,
    scroll_track_edge: AssetState,
    scroll_train: AssetState,
    tab_plate: AssetState,
    tab_stroke: AssetState,
    dynamic: Vec<(String, AssetState)>,
}

impl Theme {
    pub(crate) const fn new() -> Theme {
        Theme {
            font_title: AssetState::Unfetched,
            font_body: AssetState::Unfetched,
            plate: PlateState::Unfetched,
            scroll_track_center: AssetState::Unfetched,
            scroll_track_edge: AssetState::Unfetched,
            scroll_train: AssetState::Unfetched,
            tab_plate: AssetState::Unfetched,
            tab_stroke: AssetState::Unfetched,
            dynamic: Vec::new(),
        }
    }

    /// Returns true when any state transitioned (the panel rebuilds).
    pub(crate) fn resolve(&mut self) -> bool {
        let mut changed = false;
        changed |= load_asset(
            &mut self.font_title,
            FONT_TITLE_PATH,
            "titles fall back to the theme default font",
        );
        changed |= load_asset(
            &mut self.font_body,
            FONT_BODY_PATH,
            "body text falls back to the theme default font",
        );
        changed |= resolve_plate(&mut self.plate);
        changed |= load_asset(
            &mut self.scroll_track_center,
            SCROLL_TRACK_CENTER_PATH,
            "the scrollbar stays hidden (wheel scrolling still works)",
        );
        changed |= load_asset(
            &mut self.scroll_track_edge,
            SCROLL_TRACK_EDGE_PATH,
            "the scrollbar stays hidden (wheel scrolling still works)",
        );
        changed |= load_asset(
            &mut self.scroll_train,
            SCROLL_TRAIN_PATH,
            "the scrollbar stays hidden (wheel scrolling still works)",
        );
        changed |= load_asset(
            &mut self.tab_plate,
            TAB_PLATE_PATH,
            "the tab strip falls back to text tabs (the gold underline marks the active tab)",
        );
        changed |= load_asset(
            &mut self.tab_stroke,
            TAB_STROKE_PATH,
            "the tab strip falls back to text tabs (the gold underline marks the active tab)",
        );
        if changed && self.resolved() {
            report_once(self.loaded_count());
        }
        changed
    }

    pub(crate) fn face(&self, role: TextRole) -> Option<&RetainedVariant> {
        let state = match role {
            TextRole::Title => &self.font_title,
            TextRole::Body => &self.font_body,
        };
        match state {
            AssetState::Loaded(font) => Some(font),
            _ => None,
        }
    }

    pub(crate) fn plate(&self) -> Option<&Plate> {
        match &self.plate {
            PlateState::Loaded(plate) => Some(plate),
            _ => None,
        }
    }

    /// A missing piece hides the whole bar: a partial bar reads as broken.
    pub(crate) fn scrollbar(&self) -> Option<ScrollbarSprites<'_>> {
        let (AssetState::Loaded(center), AssetState::Loaded(edge), AssetState::Loaded(train)) = (
            &self.scroll_track_center,
            &self.scroll_track_edge,
            &self.scroll_train,
        ) else {
            return None;
        };
        Some(ScrollbarSprites {
            center,
            edge,
            train,
        })
    }

    pub(crate) fn tab_plate(&self) -> Option<&RetainedVariant> {
        match &self.tab_plate {
            AssetState::Loaded(sprite) => Some(sprite),
            _ => None,
        }
    }

    pub(crate) fn tab_stroke(&self) -> Option<&RetainedVariant> {
        match &self.tab_stroke {
            AssetState::Loaded(sprite) => Some(sprite),
            _ => None,
        }
    }

    /// Both or neither: the text tabs' gold underline is the stronger
    /// fallback marker.
    pub(crate) fn tab_sprites(&self) -> bool {
        self.tab_plate.is_loaded() && self.tab_stroke.is_loaded()
    }

    /// None past the cap, fail-logged once per process. Split from the
    /// load so the map is unit-testable without an engine.
    fn dynamic_entry(&mut self, path: &str) -> Option<usize> {
        if let Some(index) = self.dynamic.iter().position(|(p, _)| p == path) {
            return Some(index);
        }
        if self.dynamic.len() >= DYNAMIC_CACHE_CAP {
            DYNAMIC_OVERFLOW_LOGGED.with(|logged| {
                if !logged.get() {
                    logged.set(true);
                    crate::fail!(
                        "theme per-run asset cache full ({DYNAMIC_CACHE_CAP}); icon skipped: {path}"
                    );
                }
            });
            return None;
        }
        self.dynamic.push((path.to_owned(), AssetState::Unfetched));
        Some(self.dynamic.len() - 1)
    }

    /// ResourceLoader's REUSE cache dedupes engine-side; the entry lives
    /// for the process lifetime.
    pub(crate) fn resolve_dynamic(&mut self, path: &str) -> bool {
        let Some(index) = self.dynamic_entry(path) else {
            return false;
        };
        load_asset(
            &mut self.dynamic[index].1,
            path,
            "the icon is skipped (the layout re-flows without it)",
        )
    }

    pub(crate) fn dynamic(&self, path: &str) -> Option<&RetainedVariant> {
        self.dynamic
            .iter()
            .find(|(p, _)| p == path)
            .and_then(|(_, state)| match state {
                AssetState::Loaded(texture) => Some(texture),
                _ => None,
            })
    }

    fn resolved(&self) -> bool {
        !matches!(self.font_title, AssetState::Unfetched)
            && !matches!(self.font_body, AssetState::Unfetched)
            && !matches!(self.plate, PlateState::Unfetched)
            && !matches!(self.scroll_track_center, AssetState::Unfetched)
            && !matches!(self.scroll_track_edge, AssetState::Unfetched)
            && !matches!(self.scroll_train, AssetState::Unfetched)
            && !matches!(self.tab_plate, AssetState::Unfetched)
            && !matches!(self.tab_stroke, AssetState::Unfetched)
    }

    fn loaded_count(&self) -> usize {
        usize::from(self.face(TextRole::Title).is_some())
            + usize::from(self.face(TextRole::Body).is_some())
            + usize::from(self.plate().is_some())
            + usize::from(self.scroll_track_center.is_loaded())
            + usize::from(self.scroll_track_edge.is_loaded())
            + usize::from(self.scroll_train.is_loaded())
            + usize::from(self.tab_plate.is_loaded())
            + usize::from(self.tab_stroke.is_loaded())
    }
}

/// A failed construction leaks the texture by design: RetainedVariants are
/// never destroyed.
fn resolve_plate(state: &mut PlateState) -> bool {
    if !matches!(state, PlateState::Unfetched) {
        return false;
    }
    *state = match object::resource_load(PLATE_PATH).and_then(|texture| construct_plate(&texture)) {
        Some(plate) => PlateState::Loaded(plate),
        None => {
            warn!(
                "theme asset failed to load: {PLATE_PATH}; panel chrome falls back to flat rects"
            );
            PlateState::Failed
        }
    };
    true
}

/// The plain offset keeps UV math out of the scene's negative-origin trick.
fn construct_plate(texture: &RetainedVariant) -> Option<Plate> {
    let body = object::construct_style_box(&object::StyleBoxSpec {
        texture,
        region: PLATE_REGION,
        margins: PLATE_MARGINS,
        tiled: true,
        modulate: [1.0, 1.0, 1.0, 1.0],
    })?;
    let shadow = object::construct_style_box(&object::StyleBoxSpec {
        texture,
        region: PLATE_REGION,
        margins: PLATE_MARGINS,
        tiled: true,
        modulate: PLATE_SHADOW_MODULATE,
    })?;
    Some(Plate { body, shadow })
}

/// The body insets by the shadow offset; the shadow shifts down-right.
pub(crate) fn plate_rects(box_size: Vector2) -> (Rect2, Rect2) {
    let inset = Vector2::new(PLATE_SHADOW_OFFSET, PLATE_SHADOW_OFFSET);
    let body = Rect2::new(Vector2::ZERO, box_size - inset);
    let shadow = Rect2::new(inset, body.size);
    (shadow, body)
}

// The one computation every layout consumes; it derives entirely from
// the plate's visual geometry.

pub(crate) const PLATE_PAD_LEFT: f32 = 22.0;
pub(crate) const PLATE_PAD_TOP: f32 = 16.0;
/// The scene's margin_right (45) measures from the OUTER box; the body
/// already insets the 8px shadow, so the plate-relative pad is 37.
pub(crate) const PLATE_PAD_RIGHT: f32 = 37.0;
/// The scene's margin_bottom (28) minus the 8px shadow allowance.
pub(crate) const PLATE_PAD_BOTTOM: f32 = 20.0;

// The pads are the scene's margins minus the external-shadow allowance;
// pin the derivation so an edit re-derives deliberately.
const _: () = assert!(PLATE_PAD_RIGHT == 45.0 - PLATE_SHADOW_OFFSET);
const _: () = assert!(PLATE_PAD_BOTTOM == 28.0 - PLATE_SHADOW_OFFSET);

/// The pre-plate geometry; a failed plate degrades to exactly this.
pub(crate) const FLAT_PAD: f32 = 12.0;

/// The gutter is a parameter because it comes and goes with the
/// scrollbar; the reflow can never flip the overflow verdict.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContentBox {
    pub x: f32,
    pub top: f32,
    pub w: f32,
    /// The slack below the last content line, so scrolled content never
    /// ends flush against the plate's bottom edge.
    pub bottom_pad: f32,
}

impl ContentBox {
    pub(crate) fn right(&self) -> f32 {
        self.x + self.w
    }
}

/// The fallback must never look broken, just less native.
pub(crate) fn content_box(width: f32, plate: bool, right_gutter: f32) -> ContentBox {
    if plate {
        ContentBox {
            x: PLATE_PAD_LEFT,
            top: PLATE_PAD_TOP,
            w: width - PLATE_SHADOW_OFFSET - PLATE_PAD_LEFT - PLATE_PAD_RIGHT - right_gutter,
            bottom_pad: PLATE_PAD_BOTTOM,
        }
    } else {
        ContentBox {
            x: FLAT_PAD,
            top: FLAT_PAD,
            w: width - 2.0 * FLAT_PAD - right_gutter,
            bottom_pad: FLAT_PAD,
        }
    }
}

/// A missing resource warns once and degrades; a wrong method name is a
/// bug reported elsewhere, not a degradation.
fn load_asset(state: &mut AssetState, path: &str, fallback: &str) -> bool {
    if !matches!(state, AssetState::Unfetched) {
        return false;
    }
    match object::resource_load(path) {
        Some(resource) => {
            *state = AssetState::Loaded(resource);
        }
        None => {
            *state = AssetState::Failed;
            warn!("theme asset failed to load: {path}; {fallback}");
        }
    }
    true
}

thread_local! {
    /// Per process, not per panel: both load the same cached assets.
    static REPORTED: Cell<bool> = const { Cell::new(false) };
    /// The per-run cache overflow is a data bug worth one ERROR line per
    /// process, not per offending frame.
    static DYNAMIC_OVERFLOW_LOGGED: Cell<bool> = const { Cell::new(false) };
}

fn report_once(loaded: usize) {
    REPORTED.with(|reported| {
        if !reported.get() {
            reported.set(true);
            marker!("theme assets: {loaded}/{ASSET_COUNT} loaded");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_table_paths_are_distinct_res_urls() {
        let paths = [
            FONT_TITLE_PATH,
            FONT_BODY_PATH,
            PLATE_PATH,
            SCROLL_TRACK_CENTER_PATH,
            SCROLL_TRACK_EDGE_PATH,
            SCROLL_TRAIN_PATH,
            TAB_PLATE_PATH,
            TAB_STROKE_PATH,
        ];
        assert_eq!(paths.len(), ASSET_COUNT);
        for path in paths {
            assert!(path.starts_with("res://"), "not a res:// path: {path}");
        }
        let mut sorted = paths;
        sorted.sort_unstable();
        for pair in sorted.windows(2) {
            assert_ne!(pair[0], pair[1], "duplicate asset path");
        }
        // FontVariation wrappers, never raw TTFs: the tracking is the game's.
        assert!(FONT_TITLE_PATH.ends_with(".tres"));
        assert!(FONT_BODY_PATH.ends_with(".tres"));
    }

    #[test]
    fn plate_rects_inset_the_body_and_offset_the_shadow() {
        let box_size = Vector2::new(600.0, 400.0);
        let (shadow, body) = plate_rects(box_size);
        assert_eq!(body.position, Vector2::ZERO);
        assert_eq!(
            body.size,
            box_size - Vector2::new(PLATE_SHADOW_OFFSET, PLATE_SHADOW_OFFSET)
        );
        assert_eq!(shadow.size, body.size);
        assert_eq!(
            shadow.position,
            Vector2::new(PLATE_SHADOW_OFFSET, PLATE_SHADOW_OFFSET)
        );
        assert_eq!(shadow.position.x + shadow.size.x, box_size.x);
        assert_eq!(shadow.position.y + shadow.size.y, box_size.y);
    }

    #[test]
    fn content_box_insets_the_plate_body_and_reduces_to_flat() {
        let width = 600.0;
        let plate = content_box(width, true, 0.0);
        assert_eq!(plate.x, PLATE_PAD_LEFT);
        assert_eq!(plate.top, PLATE_PAD_TOP);
        assert_eq!(plate.right(), width - PLATE_SHADOW_OFFSET - PLATE_PAD_RIGHT);
        assert_eq!(plate.bottom_pad, PLATE_PAD_BOTTOM);

        let guttered = content_box(width, true, 32.0);
        assert_eq!(guttered.x, plate.x);
        assert_eq!(guttered.right(), plate.right() - 32.0);

        let flat = content_box(width, false, 0.0);
        assert_eq!(flat.x, FLAT_PAD);
        assert_eq!(flat.top, FLAT_PAD);
        assert_eq!(flat.right(), width - FLAT_PAD);
        assert_eq!(flat.bottom_pad, FLAT_PAD);
    }

    #[test]
    fn fresh_theme_is_unresolved_with_no_faces() {
        let theme = Theme::new();
        assert!(!theme.resolved());
        assert_eq!(theme.loaded_count(), 0);
        assert!(theme.face(TextRole::Title).is_none());
        assert!(theme.face(TextRole::Body).is_none());
        assert!(theme.plate().is_none());
        assert!(theme.scrollbar().is_none());
        assert!(theme.tab_plate().is_none());
        assert!(theme.tab_stroke().is_none());
        assert!(!theme.tab_sprites());
        assert!(theme.dynamic("res://anything").is_none());
    }

    #[test]
    fn dynamic_cache_dedupes_by_path_and_fails_loud_at_the_cap() {
        let mut theme = Theme::new();
        let first = theme.dynamic_entry("res://a.png").expect("inserts");
        assert_eq!(theme.dynamic_entry("res://a.png"), Some(first));
        for i in 1..DYNAMIC_CACHE_CAP {
            let path = format!("res://{i}.png");
            assert!(theme.dynamic_entry(&path).is_some(), "{path} fits");
        }
        assert_eq!(theme.dynamic.len(), DYNAMIC_CACHE_CAP);
        assert!(theme.dynamic_entry("res://overflow.png").is_none());
        assert!(theme.dynamic_entry("res://overflow2.png").is_none());
        assert_eq!(theme.dynamic_entry("res://a.png"), Some(first));
    }
}
