//! Shared geometry for offset, smooth, and symmetry.
//!
//! Flattened outlines, italic frames, contour nesting, join metrics, and
//! `outline_valid` live here so every geometry op uses the same rules.

use serde::Serialize;

use crate::font::{Contour, Font, Glyph, Point, PointKind};

/// Flat 2D vector used throughout the geometry ops.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct V2 {
    pub x: f64,
    pub y: f64,
}

impl V2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn from_point(point: &Point) -> Self {
        Self {
            x: point.x,
            y: point.y,
        }
    }

    pub fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }

    pub fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }

    pub fn mul(self, scale: f64) -> Self {
        Self {
            x: self.x * scale,
            y: self.y * scale,
        }
    }

    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }

    pub fn cross(self, other: Self) -> f64 {
        self.x * other.y - self.y * other.x
    }

    pub fn len(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn norm(self) -> Self {
        let length = self.len();
        if length < 1e-8 {
            Self::ZERO
        } else {
            self.mul(1.0 / length)
        }
    }

    pub fn perp_left(self) -> Self {
        Self {
            x: -self.y,
            y: self.x,
        }
    }

    pub fn perp_right(self) -> Self {
        Self {
            x: self.y,
            y: -self.x,
        }
    }

    pub fn lerp(self, other: Self, t: f64) -> Self {
        self.add(other.sub(self).mul(t))
    }

    pub fn dist(self, other: Self) -> f64 {
        self.sub(other).len()
    }
}

/// How an op decides whether to work in the de-slanted italic frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ItalicMode {
    #[default]
    Auto,
    Force,
    Off,
}

impl ItalicMode {
    pub fn from_field(value: &str) -> Self {
        match value {
            "true" | "force" | "on" => Self::Force,
            "false" | "off" => Self::Off,
            _ => Self::Auto,
        }
    }

    pub fn active(self, font: &Font) -> bool {
        match self {
            Self::Force => true,
            Self::Off => false,
            Self::Auto => font.style.italic && font.style.italic_angle.abs() > 1e-9,
        }
    }
}

/// Shear used by the italic frame: `x' = x - tan(-italic_angle)·(y - pivot_y)`.
#[derive(Debug, Clone, Copy)]
pub struct ItalicFrame {
    pub shear: f64,
    pub pivot_y: f64,
}

impl ItalicFrame {
    pub fn for_font(font: &Font, mode: ItalicMode) -> Option<Self> {
        if !mode.active(font) {
            return None;
        }
        let angle = font.style.italic_angle;
        if angle.abs() < 1e-12 {
            return None;
        }
        Some(Self {
            shear: (-angle).to_radians().tan(),
            pivot_y: font.metrics.x_height / 2.0,
        })
    }

    pub fn enter_point(&self, point: &mut Point) {
        point.x -= self.shear * (point.y - self.pivot_y);
    }

    pub fn leave_point(&self, point: &mut Point) {
        point.x += self.shear * (point.y - self.pivot_y);
    }

    pub fn enter_glyph(&self, glyph: &mut Glyph) {
        for contour in &mut glyph.contours {
            for point in &mut contour.points {
                self.enter_point(point);
            }
        }
    }

    pub fn leave_glyph(&self, glyph: &mut Glyph) {
        for contour in &mut glyph.contours {
            for point in &mut contour.points {
                self.leave_point(point);
            }
        }
    }

    #[allow(dead_code)]
    pub fn map(self, p: V2) -> V2 {
        V2::new(p.x - self.shear * (p.y - self.pivot_y), p.y)
    }

    #[allow(dead_code)]
    pub fn unmap(self, p: V2) -> V2 {
        V2::new(p.x + self.shear * (p.y - self.pivot_y), p.y)
    }
}

/// One segment kind between consecutive on-curve points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegKind {
    Line,
    Quad,
    Cubic,
}

/// A join at an on-curve point.
#[derive(Debug, Clone, Copy)]
pub struct Join {
    pub on: usize,
    /// Signed turn in degrees from the incoming tangent to the outgoing tangent.
    pub turn: f64,
    pub seg_in: SegKind,
    pub seg_out: SegKind,
    pub handle_in: f64,
    pub handle_out: f64,
}

/// Flatten a closed contour to a polygon (16 steps per curve). Open contours return their
/// polyline samples without wrapping.
pub fn flatten_contour(contour: &Contour, steps: usize) -> Vec<V2> {
    let points = &contour.points;
    if points.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut index = 0;
    while index < points.len() {
        let start = &points[index];
        out.push(V2::from_point(start));
        if start.kind != PointKind::On {
            index += 1;
            continue;
        }
        let Some((end_index, offs)) = next_segment(points, index, contour.closed) else {
            break;
        };
        match offs.as_slice() {
            [] => {}
            [c] => sample_quad(
                V2::from_point(start),
                V2::from_point(c),
                V2::from_point(&points[end_index]),
                steps,
                &mut out,
            ),
            [c1, c2] => sample_cubic(
                V2::from_point(start),
                V2::from_point(c1),
                V2::from_point(c2),
                V2::from_point(&points[end_index]),
                steps,
                &mut out,
            ),
            _ => {}
        }
        if end_index <= index {
            break;
        }
        index = end_index;
        if !contour.closed && index + 1 >= points.len() {
            break;
        }
        if contour.closed && end_index == 0 {
            break;
        }
    }
    if contour.closed
        && out.len() > 1
        && let (Some(first), Some(last)) = (out.first().copied(), out.last().copied())
        && first.dist(last) > 1e-6
    {
        out.push(first);
    }
    out
}

fn sample_quad(p0: V2, p1: V2, p2: V2, steps: usize, out: &mut Vec<V2>) {
    for step in 1..steps {
        let t = step as f64 / steps as f64;
        let a = p0.lerp(p1, t);
        let b = p1.lerp(p2, t);
        out.push(a.lerp(b, t));
    }
}

fn sample_cubic(p0: V2, p1: V2, p2: V2, p3: V2, steps: usize, out: &mut Vec<V2>) {
    for step in 1..steps {
        let t = step as f64 / steps as f64;
        let a = p0.lerp(p1, t);
        let b = p1.lerp(p2, t);
        let c = p2.lerp(p3, t);
        let d = a.lerp(b, t);
        let e = b.lerp(c, t);
        out.push(d.lerp(e, t));
    }
}

/// On-curve index → next on-curve index and the off-curve points between them.
pub fn next_segment(points: &[Point], start: usize, closed: bool) -> Option<(usize, Vec<&Point>)> {
    let count = points.len();
    if count == 0 || points[start].kind != PointKind::On {
        return None;
    }
    let mut offs = Vec::new();
    let mut index = start;
    loop {
        index = if index + 1 < count {
            index + 1
        } else if closed {
            0
        } else {
            return None;
        };
        if index == start {
            return None;
        }
        if points[index].kind == PointKind::On {
            return Some((index, offs));
        }
        offs.push(&points[index]);
        if offs.len() > 2 {
            return None;
        }
    }
}

/// Previous on-curve index and offs between that point and `start`.
pub fn prev_segment(points: &[Point], start: usize, closed: bool) -> Option<(usize, Vec<&Point>)> {
    let count = points.len();
    if count == 0 || points[start].kind != PointKind::On {
        return None;
    }
    let mut offs = Vec::new();
    let mut index = start;
    loop {
        index = if index > 0 {
            index - 1
        } else if closed {
            count - 1
        } else {
            return None;
        };
        if index == start {
            return None;
        }
        if points[index].kind == PointKind::On {
            offs.reverse();
            return Some((index, offs));
        }
        offs.push(&points[index]);
        if offs.len() > 2 {
            return None;
        }
    }
}

pub fn signed_area_points(points: &[Point]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for index in 0..points.len() {
        let here = &points[index];
        let next = &points[(index + 1) % points.len()];
        area += here.x * next.y - next.x * here.y;
    }
    area * 0.5
}

pub fn signed_area_poly(poly: &[V2]) -> f64 {
    if poly.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    let n = if poly.first() == poly.last() && poly.len() > 1 {
        poly.len() - 1
    } else {
        poly.len()
    };
    for index in 0..n {
        let here = poly[index];
        let next = poly[(index + 1) % n];
        area += here.x * next.y - next.x * here.y;
    }
    area * 0.5
}

pub fn point_in_poly(point: V2, poly: &[V2]) -> bool {
    let n = if poly.len() > 1 && poly.first() == poly.last() {
        poly.len() - 1
    } else {
        poly.len()
    };
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = n - 1;
    for index in 0..n {
        let a = poly[previous];
        let b = poly[index];
        let crosses = (a.y > point.y) != (b.y > point.y);
        if crosses {
            let x = (b.x - a.x) * (point.y - a.y) / (b.y - a.y + 0.0) + a.x;
            if point.x < x {
                inside = !inside;
            }
        }
        previous = index;
    }
    inside
}

/// Nesting for each closed contour: `outer = depth % 2 == 0`. Open contours get depth 0 / outer.
pub fn contour_nesting(contours: &[Contour]) -> Vec<(usize, bool)> {
    let polys: Vec<Option<Vec<V2>>> = contours
        .iter()
        .map(|contour| {
            if !contour.closed || contour.points.len() < 3 {
                None
            } else {
                Some(flatten_contour(contour, 16))
            }
        })
        .collect();
    contours
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let Some(poly) = &polys[index] else {
                return (0, true);
            };
            let Some(test) = nest_test_point(poly) else {
                return (0, true);
            };
            let depth = polys
                .iter()
                .enumerate()
                .filter(|(other, other_poly)| {
                    *other != index
                        && other_poly
                            .as_ref()
                            .is_some_and(|candidate| point_in_poly(test, candidate))
                })
                .count();
            (depth, depth % 2 == 0)
        })
        .collect()
}

fn nest_test_point(poly: &[V2]) -> Option<V2> {
    let n = if poly.len() > 1 && poly.first() == poly.last() {
        poly.len() - 1
    } else {
        poly.len()
    };
    if n < 3 {
        return None;
    }
    let area = signed_area_poly(poly);
    let interior_left = area > 0.0;
    let mut edges: Vec<(usize, f64)> = (0..n)
        .map(|i| {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            (i, a.dist(b))
        })
        .collect();
    edges.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (i, _) in edges {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let mid = a.lerp(b, 0.5);
        let edge = b.sub(a).norm();
        if edge.len() < 1e-8 {
            continue;
        }
        let inward = if interior_left {
            edge.perp_left()
        } else {
            edge.perp_right()
        };
        let candidate = mid.add(inward.mul(0.01));
        // Prefer a point that is not exactly on another sample.
        if !poly.iter().any(|p| p.dist(candidate) < 1e-6) {
            return Some(candidate);
        }
    }
    Some(poly[0])
}

/// Grow side for adding weight: away from interior for outer, toward for hole.
/// Returns true when the unit normal should point to the grow (ink-expanding) side.
pub fn grow_is_left_of_travel(contour: &Contour, outer: bool) -> bool {
    let area = signed_area_points(&contour.points);
    let interior_is_left = area > 0.0;
    if outer {
        !interior_is_left
    } else {
        interior_is_left
    }
}

/// Joins at every on-curve point of a contour.
pub fn joins(contour: &Contour) -> Vec<Join> {
    let points = &contour.points;
    let mut out = Vec::new();
    for (index, point) in points.iter().enumerate() {
        if point.kind != PointKind::On {
            continue;
        }
        let Some((prev_on, in_offs)) = prev_segment(points, index, contour.closed) else {
            continue;
        };
        let Some((next_on, out_offs)) = next_segment(points, index, contour.closed) else {
            continue;
        };
        let here = V2::from_point(point);
        let (in_tan, seg_in, handle_in) = match in_offs.as_slice() {
            [] => {
                let prev = V2::from_point(&points[prev_on]);
                (here.sub(prev), SegKind::Line, f64::INFINITY)
            }
            [c] => {
                let control = V2::from_point(c);
                (here.sub(control), SegKind::Quad, here.dist(control))
            }
            [_, c2] => {
                let control = V2::from_point(c2);
                (here.sub(control), SegKind::Cubic, here.dist(control))
            }
            _ => continue,
        };
        let (out_tan, seg_out, handle_out) = match out_offs.as_slice() {
            [] => {
                let next = V2::from_point(&points[next_on]);
                (next.sub(here), SegKind::Line, f64::INFINITY)
            }
            [c] => {
                let control = V2::from_point(c);
                (control.sub(here), SegKind::Quad, here.dist(control))
            }
            [c1, _] => {
                let control = V2::from_point(c1);
                (control.sub(here), SegKind::Cubic, here.dist(control))
            }
            _ => continue,
        };
        let in_u = in_tan.norm();
        let out_u = out_tan.norm();
        if in_u.len() < 1e-8 || out_u.len() < 1e-8 {
            continue;
        }
        let turn = signed_angle_degrees(in_u, out_u);
        out.push(Join {
            on: index,
            turn,
            seg_in,
            seg_out,
            handle_in,
            handle_out,
        });
    }
    out
}

pub fn signed_angle_degrees(from: V2, to: V2) -> f64 {
    let cross = from.cross(to);
    let dot = from.dot(to).clamp(-1.0, 1.0);
    cross.atan2(dot).to_degrees()
}

/// One validity problem on a glyph outline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValidityIssue {
    pub code: &'static str,
    pub contour: Option<usize>,
    pub detail: String,
}

/// Self-intersections, contour crossings, and area-sign flips against `source` when given.
pub fn outline_valid(glyph: &Glyph, source: Option<&Glyph>) -> Vec<ValidityIssue> {
    let mut issues = Vec::new();
    let polys: Vec<Vec<V2>> = glyph
        .contours
        .iter()
        .map(|contour| {
            if contour.closed {
                flatten_contour(contour, 16)
            } else {
                Vec::new()
            }
        })
        .collect();
    for (index, poly) in polys.iter().enumerate() {
        if poly.len() < 4 {
            continue;
        }
        if polygon_self_intersects(poly) {
            issues.push(ValidityIssue {
                code: "self-intersection",
                contour: Some(index),
                detail: format!("{} contour {index} crosses itself", glyph.name),
            });
        }
    }
    for left in 0..polys.len() {
        for right in (left + 1)..polys.len() {
            if polys[left].len() < 2 || polys[right].len() < 2 {
                continue;
            }
            if polygons_cross(&polys[left], &polys[right]) {
                issues.push(ValidityIssue {
                    code: "contour-cross",
                    contour: Some(left),
                    detail: format!("{} contours {left} and {right} cross", glyph.name),
                });
            }
        }
    }
    if let Some(source) = source {
        for (index, (contour, source_contour)) in glyph
            .contours
            .iter()
            .zip(source.contours.iter())
            .enumerate()
        {
            if !contour.closed || !source_contour.closed {
                continue;
            }
            let area = signed_area_points(&contour.points);
            let source_area = signed_area_points(&source_contour.points);
            if area.abs() > 1.0 && source_area.abs() > 1.0 && area.signum() != source_area.signum()
            {
                issues.push(ValidityIssue {
                    code: "inverted",
                    contour: Some(index),
                    detail: format!("{} contour {index} inverted its winding", glyph.name),
                });
            }
        }
    }
    issues
}

fn polygon_self_intersects(poly: &[V2]) -> bool {
    let n = if poly.len() > 1 && poly.first() == poly.last() {
        poly.len() - 1
    } else {
        poly.len()
    };
    if n < 4 {
        return false;
    }
    for left in 0..n {
        let a0 = poly[left];
        let a1 = poly[(left + 1) % n];
        for right in (left + 2)..n {
            if left == 0 && right + 1 == n {
                continue;
            }
            let b0 = poly[right];
            let b1 = poly[(right + 1) % n];
            if segments_properly_cross(a0, a1, b0, b1) {
                return true;
            }
        }
    }
    false
}

fn polygons_cross(a: &[V2], b: &[V2]) -> bool {
    let na = if a.len() > 1 && a.first() == a.last() {
        a.len() - 1
    } else {
        a.len()
    };
    let nb = if b.len() > 1 && b.first() == b.last() {
        b.len() - 1
    } else {
        b.len()
    };
    for i in 0..na {
        for j in 0..nb {
            if segments_properly_cross(a[i], a[(i + 1) % na], b[j], b[(j + 1) % nb]) {
                return true;
            }
        }
    }
    false
}

fn segments_properly_cross(a0: V2, a1: V2, b0: V2, b1: V2) -> bool {
    let d1 = orient(a0, a1, b0);
    let d2 = orient(a0, a1, b1);
    let d3 = orient(b0, b1, a0);
    let d4 = orient(b0, b1, a1);
    if ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) {
        // Ignore near-endpoint touches.
        let hit = segment_intersection(a0, a1, b0, b1);
        if let Some(p) = hit {
            let near_end = |p: V2, q: V2| p.dist(q) < 1e-3;
            if near_end(p, a0) || near_end(p, a1) || near_end(p, b0) || near_end(p, b1) {
                return false;
            }
            return true;
        }
    }
    false
}

fn orient(a: V2, b: V2, c: V2) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

pub fn segment_intersection(a0: V2, a1: V2, b0: V2, b1: V2) -> Option<V2> {
    let r = a1.sub(a0);
    let s = b1.sub(b0);
    let den = r.cross(s);
    if den.abs() < 1e-12 {
        return None;
    }
    let t = b0.sub(a0).cross(s) / den;
    let u = b0.sub(a0).cross(r) / den;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some(a0.add(r.mul(t)))
    } else {
        None
    }
}

/// Cast a ray from `origin` along `direction` (unit) against flattened edges. Only edges whose
/// outward normal · direction < 0 count. Returns the nearest hit distance.
pub fn ray_cast_boundary(
    origin: V2,
    direction: V2,
    contours: &[Contour],
    outward_for: &dyn Fn(usize, V2, V2) -> V2,
) -> Option<f64> {
    let dir = direction.norm();
    if dir.len() < 1e-8 {
        return None;
    }
    let mut best = None;
    for (contour_index, contour) in contours.iter().enumerate() {
        if !contour.closed {
            continue;
        }
        let poly = flatten_contour(contour, 16);
        let n = if poly.len() > 1 && poly.first() == poly.last() {
            poly.len() - 1
        } else {
            poly.len()
        };
        for i in 0..n {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            let edge = b.sub(a);
            if edge.len() < 1e-8 {
                continue;
            }
            let outward = outward_for(contour_index, a, b);
            if outward.dot(dir) >= 0.0 {
                continue;
            }
            if let Some(hit) = ray_segment_distance(origin, dir, a, b)
                && hit > 1e-4
            {
                best = Some(best.map_or(hit, |prev: f64| prev.min(hit)));
            }
        }
    }
    best
}

fn ray_segment_distance(origin: V2, dir: V2, a: V2, b: V2) -> Option<f64> {
    let v1 = origin.sub(a);
    let v2 = b.sub(a);
    let v3 = V2::new(-dir.y, dir.x);
    let den = v2.dot(v3);
    if den.abs() < 1e-12 {
        return None;
    }
    let t1 = v2.cross(v1) / den;
    let t2 = v1.dot(v3) / den;
    if t1 > 1e-6 && (0.0..=1.0).contains(&t2) {
        Some(t1)
    } else {
        None
    }
}

/// Horizontal stem widths via ray casts at fractions of the zone height through ink runs.
pub fn measure_stems_ray(glyph: &Glyph, zone_y: f64) -> Vec<f64> {
    let fractions = [0.3, 0.5, 0.7];
    let mut all_runs = Vec::new();
    for &frac in &fractions {
        let y = zone_y * frac;
        all_runs.extend(ink_runs_at_y(glyph, y));
    }
    if all_runs.is_empty() {
        return Vec::new();
    }
    let largest = all_runs.iter().copied().fold(0.0_f64, f64::max);
    let kept: Vec<f64> = all_runs
        .into_iter()
        .filter(|w| *w >= 0.5 * largest)
        .collect();
    if kept.is_empty() {
        return Vec::new();
    }
    // Spec F2: median of the kept runs is the stem width.
    vec![median_f64(&kept)]
}

fn ink_runs_at_y(glyph: &Glyph, y: f64) -> Vec<f64> {
    let mut xs = Vec::new();
    for contour in &glyph.contours {
        if !contour.closed {
            continue;
        }
        let poly = flatten_contour(contour, 16);
        let n = if poly.len() > 1 && poly.first() == poly.last() {
            poly.len() - 1
        } else {
            poly.len()
        };
        for i in 0..n {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            if (a.y > y) == (b.y > y) {
                continue;
            }
            let t = (y - a.y) / (b.y - a.y);
            if (0.0..=1.0).contains(&t) {
                xs.push(a.x + (b.x - a.x) * t);
            }
        }
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Dedup near hits from shared vertices.
    let mut unique = Vec::new();
    for x in xs {
        if unique
            .last()
            .is_none_or(|prev: &f64| (x - *prev).abs() > 0.5)
        {
            unique.push(x);
        }
    }
    let mut widths = Vec::new();
    let mut i = 0;
    while i + 1 < unique.len() {
        widths.push(unique[i + 1] - unique[i]);
        i += 2;
    }
    widths
}

fn median_f64(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// Horizontal bar thickness at mid-x of the glyph ink, via vertical ray through ink runs.
pub fn measure_bars_ray(glyph: &Glyph) -> Vec<f64> {
    let (min_x, max_x, min_y, max_y) = ink_bounds(glyph);
    if !min_x.is_finite() {
        return Vec::new();
    }
    let mid_x = (min_x + max_x) / 2.0;
    let mut ys = Vec::new();
    for contour in &glyph.contours {
        if !contour.closed {
            continue;
        }
        let poly = flatten_contour(contour, 16);
        let n = if poly.len() > 1 && poly.first() == poly.last() {
            poly.len() - 1
        } else {
            poly.len()
        };
        for i in 0..n {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            if (a.x > mid_x) == (b.x > mid_x) {
                continue;
            }
            let t = (mid_x - a.x) / (b.x - a.x);
            if (0.0..=1.0).contains(&t) {
                ys.push(a.y + (b.y - a.y) * t);
            }
        }
    }
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut unique = Vec::new();
    for y in ys {
        if unique
            .last()
            .is_none_or(|prev: &f64| (y - *prev).abs() > 0.5)
        {
            unique.push(y);
        }
    }
    let mut bars = Vec::new();
    let mut i = 0;
    while i + 1 < unique.len() {
        let w = unique[i + 1] - unique[i];
        if w > 1.0 && w < (max_y - min_y) * 0.45 {
            bars.push(w);
        }
        i += 2;
    }
    bars
}

pub fn ink_bounds(glyph: &Glyph) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for contour in &glyph.contours {
        for point in &contour.points {
            min_x = min_x.min(point.x);
            max_x = max_x.max(point.x);
            min_y = min_y.min(point.y);
            max_y = max_y.max(point.y);
        }
    }
    (min_x, max_x, min_y, max_y)
}

/// Hausdorff distance between two flattened outlines.
pub fn hausdorff(a: &Glyph, b: &Glyph) -> f64 {
    let pa: Vec<V2> = a
        .contours
        .iter()
        .flat_map(|c| flatten_contour(c, 12))
        .collect();
    let pb: Vec<V2> = b
        .contours
        .iter()
        .flat_map(|c| flatten_contour(c, 12))
        .collect();
    if pa.is_empty() || pb.is_empty() {
        return 0.0;
    }
    let ab = directed_hausdorff(&pa, &pb);
    let ba = directed_hausdorff(&pb, &pa);
    ab.max(ba)
}

fn directed_hausdorff(from: &[V2], to: &[V2]) -> f64 {
    from.iter()
        .map(|p| to.iter().map(|q| p.dist(*q)).fold(f64::INFINITY, f64::min))
        .fold(0.0_f64, f64::max)
}

pub fn glyph_area(glyph: &Glyph) -> f64 {
    glyph
        .contours
        .iter()
        .map(|contour| signed_area_points(&contour.points).abs())
        .sum()
}

pub fn round_glyph(glyph: &mut Glyph) {
    glyph.advance = glyph.advance.round();
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.x = point.x.round();
            point.y = point.y.round();
        }
    }
}

pub fn neighbor_index(index: usize, count: usize, closed: bool, forward: bool) -> usize {
    if forward {
        if index + 1 < count {
            index + 1
        } else if closed {
            0
        } else {
            index
        }
    } else if index > 0 {
        index - 1
    } else if closed {
        count - 1
    } else {
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::Glyph;

    fn on(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            kind: PointKind::On,
            smooth: false,
        }
    }

    fn square() -> Contour {
        Contour {
            closed: true,
            points: vec![
                on(0.0, 0.0),
                on(100.0, 0.0),
                on(100.0, 100.0),
                on(0.0, 100.0),
            ],
        }
    }

    fn hole() -> Contour {
        Contour {
            closed: true,
            // Clockwise hole inside the square.
            points: vec![
                on(25.0, 25.0),
                on(25.0, 75.0),
                on(75.0, 75.0),
                on(75.0, 25.0),
            ],
        }
    }

    #[test]
    fn nesting_marks_the_inner_contour_as_a_hole() {
        let contours = vec![square(), hole()];
        let nesting = contour_nesting(&contours);
        assert!(nesting[0].1, "outer should be outer");
        assert!(!nesting[1].1, "hole should be a hole");
        assert_eq!(nesting[1].0, 1);
    }

    #[test]
    fn nesting_uses_boundary_test_not_centroid() {
        // Ring where the outer contour's point centroid lies inside the hole.
        let outer = Contour {
            closed: true,
            points: vec![
                on(0.0, 0.0),
                on(200.0, 0.0),
                on(200.0, 200.0),
                on(0.0, 200.0),
            ],
        };
        let inner = Contour {
            closed: true,
            points: vec![
                on(40.0, 40.0),
                on(40.0, 160.0),
                on(160.0, 160.0),
                on(160.0, 40.0),
            ],
        };
        let nesting = contour_nesting(&[outer, inner]);
        assert!(nesting[0].1);
        assert!(!nesting[1].1);
    }

    #[test]
    fn outline_valid_flags_a_bowtie() {
        let glyph = Glyph {
            name: "x".into(),
            unicode: None,
            advance: 100.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    on(0.0, 0.0),
                    on(100.0, 100.0),
                    on(100.0, 0.0),
                    on(0.0, 100.0),
                ],
            }],
        };
        let issues = outline_valid(&glyph, None);
        assert!(
            issues.iter().any(|i| i.code == "self-intersection"),
            "{issues:?}"
        );
    }

    #[test]
    fn joins_report_a_right_angle() {
        let contour = square();
        let list = joins(&contour);
        assert_eq!(list.len(), 4);
        assert!((list[0].turn.abs() - 90.0).abs() < 1e-6, "{}", list[0].turn);
    }
}
