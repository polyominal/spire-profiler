//! Minimal local stand-ins for the Godot builtin value types the panel
//! plumbing needs, keeping `panel` and `run_panel` engine-free. The structs
//! are plain `f32` data, converted to the engine's variant payloads only
//! inside the engine layer; nothing here touches an engine pointer.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

impl Vector2 {
    pub const ZERO: Vector2 = Vector2::new(0.0, 0.0);

    pub const fn new(x: f32, y: f32) -> Vector2 {
        Vector2 { x, y }
    }
}

impl std::ops::Add for Vector2 {
    type Output = Vector2;

    fn add(self, rhs: Vector2) -> Vector2 {
        Vector2::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl std::ops::Sub for Vector2 {
    type Output = Vector2;

    fn sub(self, rhs: Vector2) -> Vector2 {
        Vector2::new(self.x - rhs.x, self.y - rhs.y)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect2 {
    pub position: Vector2,
    pub size: Vector2,
}

impl Rect2 {
    pub const fn new(position: Vector2, size: Vector2) -> Rect2 {
        Rect2 { position, size }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn from_rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color { r, g, b, a }
    }

    pub const fn as_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}
