use gpui::{Bounds, Pixels, Point, Size};
/// Logical window coordinates. Prefer `set_geometry` when clipping is relevant.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub position: Point<Pixels>,
    pub size: Size<Pixels>,
}
impl From<Bounds<Pixels>> for Rect {
    fn from(b: Bounds<Pixels>) -> Self {
        Self {
            position: b.origin,
            size: b.size,
        }
    }
}
impl From<Rect> for Bounds<Pixels> {
    fn from(r: Rect) -> Self {
        Self {
            origin: r.position,
            size: r.size,
        }
    }
}
