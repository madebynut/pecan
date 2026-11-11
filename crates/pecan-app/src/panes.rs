//! Sidebar geometry, including reversible edge collapse and cursor limits.
use gpui_kit::CursorStyle;

pub(crate) const DEFAULT_WIDTH: f32 = 230.;
const MIN_WIDTH: f32 = 160.;
const MAX_WIDTH: f32 = 400.;
const COLLAPSE_WIDTH: f32 = 64.;
const MIN_CENTER: f32 = 360.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Left,
    Right,
}

impl Side {
    fn direction(self) -> f32 {
        match self {
            Self::Left => 1.,
            Self::Right => -1.,
        }
    }
    pub(crate) fn cursor(self, width: Option<f32>, max: f32) -> CursorStyle {
        match (self, width) {
            (Self::Left, None) => CursorStyle::ResizeRight,
            (Self::Right, None) => CursorStyle::ResizeLeft,
            (Self::Left, Some(width)) if width >= max - 0.5 => CursorStyle::ResizeLeft,
            (Self::Right, Some(width)) if width >= max - 0.5 => CursorStyle::ResizeRight,
            _ => CursorStyle::ResizeLeftRight,
        }
    }
}

pub(crate) fn maximum(window: f32, opposite: f32) -> f32 {
    MAX_WIDTH
        .min(window * 0.35)
        .min(window - opposite - MIN_CENTER)
        .max(MIN_WIDTH)
}

pub(crate) struct Drag {
    pub(crate) side: Side,
    previous_x: f32,
    extent: f32,
}

impl Drag {
    pub(crate) fn new(side: Side, pointer_x: f32, width: f32) -> Self {
        Self {
            side,
            previous_x: pointer_x,
            extent: width,
        }
    }
    pub(crate) fn advance(&mut self, pointer_x: f32, max: f32) -> Option<f32> {
        self.extent =
            (self.extent + (pointer_x - self.previous_x) * self.side.direction()).clamp(0., max);
        self.previous_x = pointer_x;
        (self.extent > COLLAPSE_WIDTH).then(|| self.extent.max(MIN_WIDTH).min(max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dragging_to_edge_collapses_and_dragging_back_reopens_both_sides() {
        for side in [Side::Left, Side::Right] {
            let d = side.direction();
            let mut drag = Drag::new(side, 300., DEFAULT_WIDTH);
            assert_eq!(drag.advance(300. - 250. * d, 400.), None);
            assert_eq!(drag.advance(300. - 150. * d, 400.), Some(MIN_WIDTH));
            assert_eq!(drag.advance(300. + 500. * d, 400.), Some(400.));
            // Reverse immediately after the cap; no accumulated overshoot dead zone.
            assert_eq!(drag.advance(300. + 490. * d, 400.), Some(390.));
            assert!(maximum(900., 230.) <= 310.);
        }
        assert_eq!(Side::Left.cursor(Some(400.), 400.), CursorStyle::ResizeLeft);
        assert_eq!(
            Side::Right.cursor(Some(400.), 400.),
            CursorStyle::ResizeRight
        );
        assert_eq!(Side::Left.cursor(None, 400.), CursorStyle::ResizeRight);
    }
}
