//! Analytic Bezier bounds, intersections and area; no rasterization.
use crate::geometry::next_segment;
use crate::{Contour, Glyph, PointKind};
use kurbo::{
    BezPath, Line, ParamCurve, ParamCurveCurvature, ParamCurveExtrema, PathSeg, Point, Rect, Shape,
};

pub(super) fn path(contour: &Contour) -> BezPath {
    let mut out = BezPath::new();
    let Some(first) = contour.points.iter().position(|p| p.kind == PointKind::On) else {
        return out;
    };
    let points = &contour.points;
    out.move_to((points[first].x, points[first].y));
    let mut i = first;
    while let Some((j, controls)) = next_segment(points, i, contour.closed) {
        let end = (points[j].x, points[j].y);
        match controls.as_slice() {
            [] => out.line_to(end),
            [p] => out.quad_to((p.x, p.y), end),
            [p, q] => out.curve_to((p.x, p.y), (q.x, q.y), end),
            _ => break,
        }
        if j <= i {
            break;
        }
        i = j;
    }
    if contour.closed {
        out.close_path();
    }
    out
}
pub(super) struct Outline {
    pub paths: Vec<BezPath>,
    pub bounds: Option<Rect>,
}
impl Outline {
    pub fn new(g: &Glyph) -> Self {
        let paths: Vec<_> = g
            .contours
            .iter()
            .filter(|c| c.closed && c.points.len() > 1)
            .map(path)
            .collect();
        let bounds = paths
            .iter()
            .filter(|p| p.segments().next().is_some())
            .map(Shape::bounding_box)
            .reduce(|a, b| a.union(b));
        Self { paths, bounds }
    }
    pub fn intervals(&self, at: f64, vertical: bool) -> Vec<(f64, f64)> {
        let Some(b) = self.bounds else {
            return vec![];
        };
        let lo = if vertical { b.y0 } else { b.x0 } - 1.;
        let hi = if vertical { b.y1 } else { b.x1 } + 1.;
        let line = if vertical {
            Line::new((at, lo), (at, hi))
        } else {
            Line::new((lo, at), (hi, at))
        };
        let mut hits = Vec::new();
        for p in &self.paths {
            for s in p.segments() {
                for h in s.intersect_line(line) {
                    let v = s.eval(h.segment_t);
                    hits.push(if vertical { v.y } else { v.x });
                }
            }
        }
        hits.sort_by(f64::total_cmp);
        hits.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
        let mut out: Vec<(f64, f64)> = Vec::new();
        for pair in hits.windows(2) {
            let mid = (pair[0] + pair[1]) * 0.5;
            let point = if vertical {
                Point::new(at, mid)
            } else {
                Point::new(mid, at)
            };
            if self.paths.iter().map(|p| p.winding(point)).sum::<i32>() != 0 {
                if let Some((_, end)) = out.last_mut()
                    && (*end - pair[0]).abs() < 1e-6
                {
                    *end = pair[1];
                } else {
                    out.push((pair[0], pair[1]));
                }
            }
        }
        out
    }
    pub fn stroke(&self, at: f64, vertical: bool) -> Option<f64> {
        self.intervals(at, vertical)
            .into_iter()
            .map(|(a, b)| b - a)
            .filter(|v| *v > 1e-6)
            .min_by(f64::total_cmp)
    }
    pub fn area(&self) -> Option<f64> {
        // Exact signed Bezier integral; correct under consistent nonzero winding without overlaps.
        self.bounds
            .map(|_| self.paths.iter().map(Shape::area).sum::<f64>().abs())
    }
    pub fn union_area(&self) -> Option<f64> {
        let b = self.bounds?;
        let width = |y: f64| {
            self.intervals(y, false)
                .iter()
                .map(|(a, b)| b - a)
                .sum::<f64>()
        };
        // Simpson recursion carries endpoint/midpoint samples to avoid repeated curve intersections.
        #[allow(clippy::too_many_arguments)]
        fn integrate(
            f: &impl Fn(f64) -> f64,
            a: f64,
            b: f64,
            fa: f64,
            fm: f64,
            fb: f64,
            tol: f64,
            depth: u8,
        ) -> f64 {
            let m = (a + b) * 0.5;
            let l = f((a + m) * 0.5);
            let r = f((m + b) * 0.5);
            let coarse = (b - a) * (fa + 4. * fm + fb) / 6.;
            let fine = (b - a) * (fa + 4. * l + 2. * fm + 4. * r + fb) / 12.;
            if depth == 0 || (fine - coarse).abs() < 15. * tol {
                fine + (fine - coarse) / 15.
            } else {
                integrate(f, a, m, fa, l, fm, tol * 0.5, depth - 1)
                    + integrate(f, m, b, fm, r, fb, tol * 0.5, depth - 1)
            }
        }
        let mut breaks = vec![b.y0, b.y1];
        for p in &self.paths {
            for seg in p.segments() {
                breaks.push(seg.eval(0.).y);
                breaks.push(seg.eval(1.).y);
                for t in seg.extrema() {
                    breaks.push(seg.eval(t).y);
                }
            }
        }
        breaks.sort_by(f64::total_cmp);
        breaks.dedup_by(|a, b| (*a - *b).abs() < 1e-8);
        let mut area = 0.;
        let tol = b.width() * b.height() * 1e-8;
        for pair in breaks.windows(2) {
            let a = pair[0];
            let z = pair[1];
            let eps = (z - a) * 1e-8;
            let m = (a + z) * 0.5;
            area += integrate(
                &width,
                a,
                z,
                width(a + eps),
                width(m),
                width(z - eps),
                tol / breaks.len() as f64,
                16,
            );
        }
        Some(area)
    }
    pub fn inner(&self) -> Option<&BezPath> {
        // A counter is a contour whose midpoint lies within another contour.
        self.paths
            .iter()
            .filter(|p| {
                let Some(s) = p.segments().next() else {
                    return false;
                };
                let q = s.eval(0.5);
                self.paths
                    .iter()
                    .filter(|other| !std::ptr::eq(*other, *p))
                    .any(|outer| outer.winding(q) != 0)
            })
            .max_by(|a, b| a.area().abs().total_cmp(&b.area().abs()))
    }
}
pub(super) fn curvature(seg: PathSeg, t: f64) -> f64 {
    match seg {
        PathSeg::Line(s) => s.curvature(t),
        PathSeg::Quad(s) => s.curvature(t),
        PathSeg::Cubic(s) => s.curvature(t),
    }
}
pub(super) fn extrema_missing(g: &Glyph) -> usize {
    g.contours
        .iter()
        .flat_map(|c| path(c).segments().collect::<Vec<_>>())
        .map(|s| {
            s.extrema()
                .iter()
                .filter(|t| **t > 1e-7 && **t < 1. - 1e-7)
                .count()
        })
        .sum()
}
