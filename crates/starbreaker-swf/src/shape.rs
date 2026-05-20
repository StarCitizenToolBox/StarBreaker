//! Convert SWF `ShapeRecord` sequences into kurbo `BezPath`s.
//!
//! A Flash shape is a stateful pen-based stream of `StyleChange` /
//! `StraightEdge` / `CurvedEdge` records. The fill style ID toggles
//! orientation but does not affect the outline shape itself, so for font
//! conversion we drop all style info and keep only the geometry. The pen
//! position is absolute; `StyleChange.move_to` is the only record that
//! teleports it, and all `*Edge` records are deltas.
//!
//! SWF Y axis points down (Flash convention). TrueType Y points up. The
//! caller flips Y when scaling to em-units; this module emits paths in raw
//! SWF coordinates (twips).

use kurbo::{BezPath, Point};
use swf::ShapeRecord;

/// Convert a glyph's `shape_records` into a kurbo `BezPath`.
///
/// Coordinates are emitted as raw twips. Caller is responsible for any
/// scaling, Y-flipping, or em-unit normalization. The path starts with the
/// first `move_to` it encounters; records before the first `move_to` (which
/// shouldn't occur in a well-formed glyph) are ignored.
pub fn shape_records_to_bezpath(records: &[ShapeRecord]) -> BezPath {
    let mut path = BezPath::new();
    let mut pen_x: f64 = 0.0;
    let mut pen_y: f64 = 0.0;
    let mut have_subpath = false;

    for rec in records {
        match rec {
            ShapeRecord::StyleChange(sc) => {
                if let Some(move_to) = sc.move_to {
                    // Close the previous subpath, if any, then start a new one.
                    if have_subpath {
                        path.close_path();
                    }
                    pen_x = move_to.x.get() as f64;
                    pen_y = move_to.y.get() as f64;
                    path.move_to(Point::new(pen_x, pen_y));
                    have_subpath = true;
                }
                // Fill / line style changes are irrelevant for font outlines.
            }
            ShapeRecord::StraightEdge { delta } => {
                if !have_subpath {
                    continue;
                }
                pen_x += delta.dx.get() as f64;
                pen_y += delta.dy.get() as f64;
                path.line_to(Point::new(pen_x, pen_y));
            }
            ShapeRecord::CurvedEdge {
                control_delta,
                anchor_delta,
            } => {
                if !have_subpath {
                    continue;
                }
                let cx = pen_x + control_delta.dx.get() as f64;
                let cy = pen_y + control_delta.dy.get() as f64;
                pen_x = cx + anchor_delta.dx.get() as f64;
                pen_y = cy + anchor_delta.dy.get() as f64;
                path.quad_to(Point::new(cx, cy), Point::new(pen_x, pen_y));
            }
        }
    }

    if have_subpath {
        path.close_path();
    }

    path
}

/// Linearly transform a kurbo `BezPath`'s coordinates.
///
/// `(x, y)` becomes `(x * scale_x + offset_x, y * scale_y + offset_y)`.
/// Used to scale SWF twips to TTF em-units and to flip the Y axis.
pub fn transform_path(path: &BezPath, scale_x: f64, scale_y: f64, offset_x: f64, offset_y: f64) -> BezPath {
    use kurbo::PathEl;
    let map_pt = |p: Point| Point::new(p.x * scale_x + offset_x, p.y * scale_y + offset_y);
    let mut out = BezPath::new();
    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => out.move_to(map_pt(*p)),
            PathEl::LineTo(p) => out.line_to(map_pt(*p)),
            PathEl::QuadTo(c, p) => out.quad_to(map_pt(*c), map_pt(*p)),
            PathEl::CurveTo(c1, c2, p) => out.curve_to(map_pt(*c1), map_pt(*c2), map_pt(*p)),
            PathEl::ClosePath => out.close_path(),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::PathEl;
    use swf::{Point as SwfPoint, PointDelta, ShapeRecord, StyleChangeData, Twips};

    fn mv(x: i32, y: i32) -> ShapeRecord {
        ShapeRecord::StyleChange(Box::new(StyleChangeData {
            move_to: Some(SwfPoint::new(Twips::new(x), Twips::new(y))),
            fill_style_0: None,
            fill_style_1: None,
            line_style: None,
            new_styles: None,
        }))
    }
    fn line(dx: i32, dy: i32) -> ShapeRecord {
        ShapeRecord::StraightEdge {
            delta: PointDelta::new(Twips::new(dx), Twips::new(dy)),
        }
    }
    fn curve(cdx: i32, cdy: i32, adx: i32, ady: i32) -> ShapeRecord {
        ShapeRecord::CurvedEdge {
            control_delta: PointDelta::new(Twips::new(cdx), Twips::new(cdy)),
            anchor_delta: PointDelta::new(Twips::new(adx), Twips::new(ady)),
        }
    }

    #[test]
    fn straight_triangle_traces_correctly() {
        // M 0,0 L 100,0 L 0,100 close
        let recs = vec![mv(0, 0), line(100, 0), line(-100, 100), line(0, -100)];
        let path = shape_records_to_bezpath(&recs);
        let els: Vec<_> = path.elements().to_vec();
        assert!(matches!(els[0], PathEl::MoveTo(p) if p == Point::new(0.0, 0.0)));
        assert!(matches!(els[1], PathEl::LineTo(p) if p == Point::new(100.0, 0.0)));
        assert!(matches!(els[2], PathEl::LineTo(p) if p == Point::new(0.0, 100.0)));
        assert!(matches!(els[3], PathEl::LineTo(p) if p == Point::new(0.0, 0.0)));
        assert!(matches!(els[4], PathEl::ClosePath));
    }

    #[test]
    fn quadratic_curve_anchors_correctly() {
        // M 0,0 then a quadratic with control delta (50,100) and anchor delta (50,-100)
        // → control at (50,100), end at (100,0)
        let recs = vec![mv(0, 0), curve(50, 100, 50, -100)];
        let path = shape_records_to_bezpath(&recs);
        let els: Vec<_> = path.elements().to_vec();
        match &els[1] {
            PathEl::QuadTo(c, p) => {
                assert_eq!(*c, Point::new(50.0, 100.0));
                assert_eq!(*p, Point::new(100.0, 0.0));
            }
            other => panic!("expected QuadTo, got {other:?}"),
        }
    }

    #[test]
    fn multiple_subpaths_close_independently() {
        let recs = vec![
            mv(0, 0), line(10, 0), line(0, 10), line(-10, 0), line(0, -10),
            mv(20, 20), line(5, 0), line(0, 5),
        ];
        let path = shape_records_to_bezpath(&recs);
        let close_count = path.elements().iter().filter(|e| matches!(e, PathEl::ClosePath)).count();
        assert_eq!(close_count, 2);
    }

    #[test]
    fn empty_shape_produces_empty_path() {
        let path = shape_records_to_bezpath(&[]);
        assert!(path.elements().is_empty());
    }
}
