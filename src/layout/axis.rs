use niri_config::MainAxis;
use smithay::utils::{Coordinate, Logical, Point, Rectangle, Size};

use crate::utils::ResizeEdge;

/// Maps physical coordinates to scrolling coordinates (X = main, Y = cross), and back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AxisMap {
    main_axis: MainAxis,
}

impl AxisMap {
    pub const fn new(main_axis: MainAxis) -> Self {
        Self { main_axis }
    }

    pub const fn main_axis(self) -> MainAxis {
        self.main_axis
    }

    pub const fn is_vertical(self) -> bool {
        matches!(self.main_axis, MainAxis::Vertical)
    }

    pub fn point_in<N: Coordinate>(self, point: Point<N, Logical>) -> Point<N, Logical> {
        if self.is_vertical() {
            Point::from((point.y, point.x))
        } else {
            point
        }
    }

    pub fn point_out<N: Coordinate>(self, point: Point<N, Logical>) -> Point<N, Logical> {
        self.point_in(point)
    }

    pub fn size_in<N: Coordinate>(self, size: Size<N, Logical>) -> Size<N, Logical> {
        if self.is_vertical() {
            Size::from((size.h, size.w))
        } else {
            size
        }
    }

    pub fn size_out<N: Coordinate>(self, size: Size<N, Logical>) -> Size<N, Logical> {
        self.size_in(size)
    }

    pub fn rect_in<N: Coordinate>(self, rect: Rectangle<N, Logical>) -> Rectangle<N, Logical> {
        Rectangle::new(self.point_in(rect.loc), self.size_in(rect.size))
    }

    pub fn rect_out<N: Coordinate>(self, rect: Rectangle<N, Logical>) -> Rectangle<N, Logical> {
        self.rect_in(rect)
    }

    pub fn point_from_main_cross<N: Coordinate>(self, main: N, cross: N) -> Point<N, Logical> {
        self.point_out(Point::from((main, cross)))
    }

    pub fn size_from_main_cross<N: Coordinate>(self, main: N, cross: N) -> Size<N, Logical> {
        self.size_out(Size::from((main, cross)))
    }

    /// A physical vector along the scrolling strip.
    pub fn main_vec<N: Coordinate>(self, main: N) -> Point<N, Logical> {
        self.point_from_main_cross(main, N::from_f64(0.))
    }

    /// A physical vector perpendicular to the scrolling strip.
    pub fn cross_vec<N: Coordinate>(self, cross: N) -> Point<N, Logical> {
        self.point_from_main_cross(N::from_f64(0.), cross)
    }

    pub fn size_main<N: Coordinate>(self, size: Size<N, Logical>) -> N {
        if self.is_vertical() {
            size.h
        } else {
            size.w
        }
    }

    pub fn size_cross<N: Coordinate>(self, size: Size<N, Logical>) -> N {
        if self.is_vertical() {
            size.w
        } else {
            size.h
        }
    }

    /// Split a physical pointer delta into `(main, cross)`.
    pub fn gesture_components(self, dx: f64, dy: f64) -> (f64, f64) {
        let point = self.point_in(Point::from((dx, dy)));
        (point.x, point.y)
    }

    pub fn resize_edges_in(self, edges: ResizeEdge) -> ResizeEdge {
        if !self.is_vertical() {
            return edges;
        }

        let mut mapped = ResizeEdge::empty();
        if edges.contains(ResizeEdge::LEFT) {
            mapped |= ResizeEdge::TOP;
        }
        if edges.contains(ResizeEdge::RIGHT) {
            mapped |= ResizeEdge::BOTTOM;
        }
        if edges.contains(ResizeEdge::TOP) {
            mapped |= ResizeEdge::LEFT;
        }
        if edges.contains(ResizeEdge::BOTTOM) {
            mapped |= ResizeEdge::RIGHT;
        }
        mapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_mapping_is_identity() {
        let axis = AxisMap::new(MainAxis::Horizontal);
        let point = Point::<f64, Logical>::from((3., 7.));
        let size = Size::<f64, Logical>::from((11., 13.));
        assert_eq!(axis.point_in(point), point);
        assert_eq!(axis.size_out(size), size);
        assert_eq!(axis.gesture_components(3., -7.), (3., -7.));
    }

    #[test]
    fn vertical_mapping_swaps_axes() {
        let axis = AxisMap::new(MainAxis::Vertical);
        assert_eq!(
            axis.point_from_main_cross(3., 7.),
            Point::<f64, Logical>::from((7., 3.))
        );
        assert_eq!(
            axis.size_from_main_cross(11., 13.),
            Size::<f64, Logical>::from((13., 11.))
        );
        assert_eq!(axis.gesture_components(3., -7.), (-7., 3.));
        assert_eq!(axis.size_main(Size::<f64, Logical>::from((11., 13.))), 13.);
        assert_eq!(axis.size_cross(Size::<f64, Logical>::from((11., 13.))), 11.);
    }
}
