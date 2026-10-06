//! Geometry for the drawing window: the glyph outline as the session reports it, the
//! font-to-screen viewport, outline flattening, filling, and handle hit testing.
//!
//! Nothing here changes a font. The window reads glyphs with the `glyph` command and writes
//! through session commands such as `move_points` and `add_contour`.

use serde_json::Value;

/// Havelin v2 chrome colors, as `0xRRGGBB`.
pub mod palette {
    pub const PAGE: u32 = 0x030303;
    pub const PANEL: u32 = 0x090907;
    pub const RAISED: u32 = 0x11110D;
    pub const HAIRLINE: u32 = 0x302A1E;
    pub const HAIRLINE_STRONG: u32 = 0x5B4A2D;
    pub const INK: u32 = 0xDED9CE;
    pub const MUTED: u32 = 0x9D988C;
    pub const BONE: u32 = 0xF3EEE4;
    pub const FOCUS: u32 = 0xD8FF00;
    pub const AMBER: u32 = 0xE8B65A;
    /// Editor guides. Bright enough to read on the bone paper and on black.
    pub const GUIDE: u32 = 0x3D8BFF;
    pub const SIGNAL: u32 = 0x8AE6A3;
    pub const ALERT: u32 = 0xF07461;
    pub const INVERSE: u32 = 0x030303;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn lerp(self, other: Pt, t: f64) -> Pt {
        Pt::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }

    fn distance(self, other: Pt) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

/// An axis-aligned box. In font units `y` grows up; on screen it grows down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: Pt,
    pub max: Pt,
}

impl Bounds {
    pub fn width(&self) -> f64 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f64 {
        self.max.y - self.min.y
    }

    fn include(&mut self, point: Pt) {
        self.min.x = self.min.x.min(point.x);
        self.min.y = self.min.y.min(point.y);
        self.max.x = self.max.x.max(point.x);
        self.max.y = self.max.y.max(point.y);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutlinePoint {
    pub at: Pt,
    pub on: bool,
    pub smooth: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutlineContour {
    pub closed: bool,
    pub points: Vec<OutlinePoint>,
}

/// One glyph as returned by the `glyph` command.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    pub name: String,
    pub unicode: Option<u32>,
    pub advance: f64,
    pub contours: Vec<OutlineContour>,
}

impl Outline {
    /// Read the `data` of a `glyph` command response.
    pub fn from_json(data: &Value) -> Option<Self> {
        let contours = data["contours"]
            .as_array()?
            .iter()
            .map(|contour| {
                let points = contour["points"]
                    .as_array()?
                    .iter()
                    .map(|point| {
                        Some(OutlinePoint {
                            at: Pt::new(point["x"].as_f64()?, point["y"].as_f64()?),
                            on: point["kind"].as_str()? == "on",
                            smooth: point["smooth"].as_bool().unwrap_or(false),
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(OutlineContour {
                    closed: contour["closed"].as_bool()?,
                    points,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            name: data["name"].as_str()?.to_string(),
            unicode: data["unicode"]
                .as_u64()
                .and_then(|code| u32::try_from(code).ok()),
            advance: data["advance"].as_f64()?,
            contours,
        })
    }

    /// The box to fit: every point, the advance, and the vertical metrics given.
    pub fn bounds(&self, descender: f64, ascender: f64) -> Bounds {
        let mut bounds = Bounds {
            min: Pt::new(0.0, descender.min(ascender)),
            max: Pt::new(self.advance.max(0.0), descender.max(ascender)),
        };
        for contour in &self.contours {
            for point in &contour.points {
                bounds.include(point.at);
            }
        }
        bounds
    }

    pub fn point(&self, handle: Handle) -> Option<&OutlinePoint> {
        self.contours.get(handle.contour)?.points.get(handle.point)
    }
}

/// A point address, as `move_point` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle {
    pub contour: usize,
    pub point: usize,
}

/// Font units to screen points. `origin` is where font (0, 0) lands on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub scale: f64,
    pub origin: Pt,
}

impl Viewport {
    pub const MIN_SCALE: f64 = 0.005;
    pub const MAX_SCALE: f64 = 200.0;

    /// Center `bounds` in `canvas`, leaving `margin` screen points on the tighter side.
    pub fn fit(canvas: Bounds, bounds: Bounds, margin: f64) -> Self {
        let room_x = (canvas.width() - 2.0 * margin).max(1.0);
        let room_y = (canvas.height() - 2.0 * margin).max(1.0);
        let width = bounds.width().max(1.0);
        let height = bounds.height().max(1.0);
        let scale = (room_x / width)
            .min(room_y / height)
            .clamp(Self::MIN_SCALE, Self::MAX_SCALE);
        let canvas_center = Pt::new(
            (canvas.min.x + canvas.max.x) / 2.0,
            (canvas.min.y + canvas.max.y) / 2.0,
        );
        let font_center = Pt::new(
            (bounds.min.x + bounds.max.x) / 2.0,
            (bounds.min.y + bounds.max.y) / 2.0,
        );
        Self {
            scale,
            origin: Pt::new(
                canvas_center.x - font_center.x * scale,
                canvas_center.y + font_center.y * scale,
            ),
        }
    }

    pub fn to_screen(&self, font: Pt) -> Pt {
        Pt::new(
            self.origin.x + font.x * self.scale,
            self.origin.y - font.y * self.scale,
        )
    }

    pub fn to_font(&self, screen: Pt) -> Pt {
        Pt::new(
            (screen.x - self.origin.x) / self.scale,
            (self.origin.y - screen.y) / self.scale,
        )
    }

    /// Zoom by `factor`, keeping the font point under `anchor` where it is.
    pub fn zoom_at(&mut self, anchor: Pt, factor: f64) {
        let fixed = self.to_font(anchor);
        self.scale = (self.scale * factor).clamp(Self::MIN_SCALE, Self::MAX_SCALE);
        self.origin = Pt::new(
            anchor.x - fixed.x * self.scale,
            anchor.y + fixed.y * self.scale,
        );
    }

    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.origin = Pt::new(self.origin.x + dx, self.origin.y + dy);
    }
}

/// The handle under `screen`, within `radius` screen points. An on-curve point wins over any
/// off-curve point in range; otherwise the nearest point wins.
pub fn hit_test(outline: &Outline, view: &Viewport, screen: Pt, radius: f64) -> Option<Handle> {
    let mut best_on: Option<(f64, Handle)> = None;
    let mut best_off: Option<(f64, Handle)> = None;
    for (contour_index, contour) in outline.contours.iter().enumerate() {
        for (point_index, point) in contour.points.iter().enumerate() {
            let distance = view.to_screen(point.at).distance(screen);
            if distance > radius {
                continue;
            }
            let handle = Handle {
                contour: contour_index,
                point: point_index,
            };
            let slot = if point.on {
                &mut best_on
            } else {
                &mut best_off
            };
            if slot.is_none_or(|(best, _)| distance < best) {
                *slot = Some((distance, handle));
            }
        }
    }
    best_on.or(best_off).map(|(_, handle)| handle)
}

/// Flatten one contour to a polyline in font units. One off-point between on-points is a
/// quadratic, two is a cubic. Longer runs are read as a quadratic chain with implied on-points.
/// Closed contours wrap; the polyline does not repeat its first point.
pub fn flatten(contour: &OutlineContour, steps: usize) -> Vec<Pt> {
    let steps = steps.max(1);
    let points = &contour.points;
    let Some(start) = points.iter().position(|point| point.on) else {
        return flatten_all_off(points, steps);
    };
    let count = points.len();
    let span = if contour.closed { count } else { count - start };
    let mut out = vec![points[start].at];
    let mut offs: Vec<Pt> = Vec::new();
    let mut from = points[start].at;
    for step in 1..=span {
        let index = (start + step) % count;
        if !contour.closed && start + step >= count {
            break;
        }
        let point = &points[index];
        if !point.on {
            offs.push(point.at);
            continue;
        }
        emit_segment(&mut out, from, &offs, point.at, steps);
        offs.clear();
        from = point.at;
    }
    if contour.closed && out.len() > 1 {
        out.pop();
    }
    out
}

fn emit_segment(out: &mut Vec<Pt>, from: Pt, offs: &[Pt], to: Pt, steps: usize) {
    match offs {
        [] => out.push(to),
        [control] => push_quad(out, from, *control, to, steps),
        [first, second] => {
            for step in 1..=steps {
                let t = step as f64 / steps as f64;
                out.push(cubic(from, *first, *second, to, t));
            }
        }
        _ => {
            let mut start = from;
            for (index, control) in offs.iter().enumerate() {
                let end = match offs.get(index + 1) {
                    Some(next) => control.lerp(*next, 0.5),
                    None => to,
                };
                push_quad(out, start, *control, end, steps);
                start = end;
            }
        }
    }
}

fn flatten_all_off(points: &[OutlinePoint], steps: usize) -> Vec<Pt> {
    let count = points.len();
    if count == 0 {
        return Vec::new();
    }
    let mid = |index: usize| points[index].at.lerp(points[(index + 1) % count].at, 0.5);
    let mut out = vec![mid(count - 1)];
    for (index, point) in points.iter().enumerate() {
        let from = *out.last().unwrap_or(&point.at);
        push_quad(&mut out, from, point.at, mid(index), steps);
    }
    out.pop();
    out
}

fn push_quad(out: &mut Vec<Pt>, from: Pt, control: Pt, to: Pt, steps: usize) {
    for step in 1..=steps {
        let t = step as f64 / steps as f64;
        out.push(from.lerp(control, t).lerp(control.lerp(to, t), t));
    }
}

fn cubic(p0: Pt, p1: Pt, p2: Pt, p3: Pt, t: f64) -> Pt {
    let a = p0.lerp(p1, t);
    let b = p1.lerp(p2, t);
    let c = p2.lerp(p3, t);
    a.lerp(b, t).lerp(b.lerp(c, t), t)
}

/// Filled spans of a horizontal line through closed polygons, by the nonzero winding rule.
pub fn scanline_spans(polygons: &[Vec<Pt>], y: f64) -> Vec<(f64, f64)> {
    let mut crossings: Vec<(f64, i32)> = Vec::new();
    for polygon in polygons {
        let count = polygon.len();
        if count < 3 {
            continue;
        }
        for index in 0..count {
            let a = polygon[index];
            let b = polygon[(index + 1) % count];
            let (winding, low, high) = if a.y < b.y { (1, a, b) } else { (-1, b, a) };
            // Half-open so a vertex shared by two edges counts once.
            if y < low.y || y >= high.y {
                continue;
            }
            let t = (y - low.y) / (high.y - low.y);
            crossings.push((low.x + (high.x - low.x) * t, winding));
        }
    }
    crossings.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut spans = Vec::new();
    let mut winding = 0;
    let mut start = 0.0;
    for (x, delta) in crossings {
        let before = winding;
        winding += delta;
        if before == 0 && winding != 0 {
            start = x;
        } else if before != 0 && winding == 0 && x > start {
            spans.push((start, x));
        }
    }
    spans
}

/// A 2D affine matrix `[a, b, c, d, e, f]`, the same layout as the `transform` command:
/// `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
pub type Matrix = [f64; 6];

pub mod matrix {
    use super::{Matrix, Pt};

    pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

    pub fn apply(m: &Matrix, p: Pt) -> Pt {
        Pt::new(
            m[0] * p.x + m[2] * p.y + m[4],
            m[1] * p.x + m[3] * p.y + m[5],
        )
    }

    /// `first`, then `second`.
    pub fn then(first: &Matrix, second: &Matrix) -> Matrix {
        let [a1, b1, c1, d1, e1, f1] = *first;
        let [a2, b2, c2, d2, e2, f2] = *second;
        [
            a2 * a1 + c2 * b1,
            b2 * a1 + d2 * b1,
            a2 * c1 + c2 * d1,
            b2 * c1 + d2 * d1,
            a2 * e1 + c2 * f1 + e2,
            b2 * e1 + d2 * f1 + f2,
        ]
    }

    pub fn translate(dx: f64, dy: f64) -> Matrix {
        [1.0, 0.0, 0.0, 1.0, dx, dy]
    }

    /// Scale about `center`.
    pub fn scale(sx: f64, sy: f64, center: Pt) -> Matrix {
        around(&[sx, 0.0, 0.0, sy, 0.0, 0.0], center)
    }

    /// Rotate counter-clockwise by `degrees` about `center`.
    pub fn rotate(degrees: f64, center: Pt) -> Matrix {
        let (sin, cos) = degrees.to_radians().sin_cos();
        around(&[cos, sin, -sin, cos, 0.0, 0.0], center)
    }

    /// Slant like an oblique: x moves right by `tan(degrees) * y`, so the baseline stays put.
    pub fn slant(degrees: f64) -> Matrix {
        [1.0, 0.0, degrees.to_radians().tan(), 1.0, 0.0, 0.0]
    }

    pub fn flip_horizontal(center_x: f64) -> Matrix {
        [-1.0, 0.0, 0.0, 1.0, 2.0 * center_x, 0.0]
    }

    pub fn flip_vertical(center_y: f64) -> Matrix {
        [1.0, 0.0, 0.0, -1.0, 0.0, 2.0 * center_y]
    }

    /// `m` applied about `center` instead of the origin.
    pub fn around(m: &Matrix, center: Pt) -> Matrix {
        then(
            &then(&translate(-center.x, -center.y), m),
            &translate(center.x, center.y),
        )
    }
}

impl Outline {
    /// A copy with `m` applied to every point, or only to `only` when given.
    pub fn transformed(&self, m: &Matrix, only: Option<&[Handle]>) -> Outline {
        let mut out = self.clone();
        for (contour_index, contour) in out.contours.iter_mut().enumerate() {
            for (point_index, point) in contour.points.iter_mut().enumerate() {
                let chosen = only.is_none_or(|handles| {
                    handles.contains(&Handle {
                        contour: contour_index,
                        point: point_index,
                    })
                });
                if chosen {
                    point.at = matrix::apply(m, point.at);
                }
            }
        }
        out
    }

    /// The box around the points only, or `None` for an empty glyph.
    pub fn ink_bounds(&self) -> Option<Bounds> {
        let mut points = self
            .contours
            .iter()
            .flat_map(|contour| contour.points.iter().map(|point| point.at));
        let first = points.next()?;
        let mut bounds = Bounds {
            min: first,
            max: first,
        };
        for point in points {
            bounds.include(point);
        }
        Some(bounds)
    }

    pub fn handles(&self) -> Vec<Handle> {
        self.contours
            .iter()
            .enumerate()
            .flat_map(|(contour, found)| {
                (0..found.points.len()).map(move |point| Handle { contour, point })
            })
            .collect()
    }
}

/// Where an effect is centered, matching the `transform` command's `anchor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Origin,
    Center,
    Advance,
}

impl Anchor {
    /// The name the `transform` command takes.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Origin => "origin",
            Self::Center => "center",
            Self::Advance => "advance",
        }
    }

    /// The pivot for this outline, or for `only` its chosen points, in font units.
    pub fn point(self, outline: &Outline, only: Option<&[Handle]>) -> Pt {
        let chosen: Vec<Pt> = match only {
            Some(handles) => handles
                .iter()
                .filter_map(|handle| outline.point(*handle).map(|point| point.at))
                .collect(),
            None => outline
                .contours
                .iter()
                .flat_map(|contour| contour.points.iter().map(|point| point.at))
                .collect(),
        };
        let center = chosen.first().map(|first| {
            let (mut lo, mut hi) = (*first, *first);
            for p in &chosen {
                lo = Pt::new(lo.x.min(p.x), lo.y.min(p.y));
                hi = Pt::new(hi.x.max(p.x), hi.y.max(p.y));
            }
            Pt::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0)
        });
        let center = center.unwrap_or(Pt::new(outline.advance / 2.0, 0.0));
        match self {
            Self::Origin => Pt::new(0.0, 0.0),
            Self::Center => center,
            Self::Advance => Pt::new(outline.advance / 2.0, center.y),
        }
    }
}

/// Even-odd test. Points exactly on an edge are not treated as inside.
pub fn point_in_polygon(point: Pt, polygon: &[Pt]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for index in 0..polygon.len() {
        let (a, b) = (polygon[previous], polygon[index]);
        let crosses = (a.y > point.y) != (b.y > point.y);
        if crosses {
            let x = a.x + (b.x - a.x) * (point.y - a.y) / (b.y - a.y);
            if point.x < x {
                inside = !inside;
            }
        }
        previous = index;
    }
    inside
}

/// Handles whose screen position falls inside a lasso drawn in screen points.
pub fn handles_in_polygon(outline: &Outline, view: &Viewport, polygon: &[Pt]) -> Vec<Handle> {
    outline
        .handles()
        .into_iter()
        .filter(|handle| {
            outline
                .point(*handle)
                .is_some_and(|point| point_in_polygon(view.to_screen(point.at), polygon))
        })
        .collect()
}

/// Handles whose screen position falls inside the box spanned by two screen corners.
pub fn handles_in_rect(outline: &Outline, view: &Viewport, a: Pt, b: Pt) -> Vec<Handle> {
    let (min_x, max_x) = (a.x.min(b.x), a.x.max(b.x));
    let (min_y, max_y) = (a.y.min(b.y), a.y.max(b.y));
    outline
        .handles()
        .into_iter()
        .filter(|handle| {
            outline.point(*handle).is_some_and(|point| {
                let at = view.to_screen(point.at);
                (min_x..=max_x).contains(&at.x) && (min_y..=max_y).contains(&at.y)
            })
        })
        .collect()
}

/// A place on an outline segment, as `split_segment` takes it: the contour, the on-curve point
/// that ends the segment, and the curve parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentHit {
    pub contour: usize,
    pub end: usize,
    pub t: f64,
    pub at: Pt,
}

/// The nearest point on any segment within `radius` screen points of `screen`.
pub fn nearest_segment(
    outline: &Outline,
    view: &Viewport,
    screen: Pt,
    radius: f64,
) -> Option<SegmentHit> {
    const SAMPLES: usize = 64;
    let mut best: Option<(f64, SegmentHit)> = None;
    for (contour_index, contour) in outline.contours.iter().enumerate() {
        let count = contour.points.len();
        for (end, point) in contour.points.iter().enumerate() {
            if !point.on {
                continue;
            }
            // Walk back to the previous on-curve point.
            let mut offs = Vec::new();
            let mut cursor = end;
            let start = loop {
                if cursor == 0 {
                    if !contour.closed {
                        break None;
                    }
                    cursor = count;
                }
                cursor -= 1;
                if cursor == end {
                    break None;
                }
                if contour.points[cursor].on {
                    break Some(cursor);
                }
                offs.push(contour.points[cursor].at);
            };
            let Some(start) = start else {
                continue;
            };
            offs.reverse();
            if offs.len() > 2 {
                continue;
            }
            let p0 = contour.points[start].at;
            let p3 = point.at;
            for step in 1..SAMPLES {
                let t = step as f64 / SAMPLES as f64;
                let at = match offs.as_slice() {
                    [] => p0.lerp(p3, t),
                    [c] => p0.lerp(*c, t).lerp(c.lerp(p3, t), t),
                    [c1, c2] => cubic(p0, *c1, *c2, p3, t),
                    _ => continue,
                };
                let distance = view.to_screen(at).distance(screen);
                if distance <= radius && best.is_none_or(|(d, _)| distance < d) {
                    best = Some((
                        distance,
                        SegmentHit {
                            contour: contour_index,
                            end,
                            t,
                            at,
                        },
                    ));
                }
            }
        }
    }
    best.map(|(_, hit)| hit)
}

/// Draw the closed contours of `outline` into a `width` x `height` coverage mask (0 to 255,
/// row 0 at the top). `frame` is the font-unit box that fills the mask.
pub fn rasterize(outline: &Outline, frame: Bounds, width: usize, height: usize) -> Vec<u8> {
    const SUB: usize = 4;
    let mut mask = vec![0u8; width * height];
    if width == 0 || height == 0 || frame.width() <= 0.0 || frame.height() <= 0.0 {
        return mask;
    }
    let scale = (width as f64 / frame.width()).min(height as f64 / frame.height());
    let pad_x = (width as f64 - frame.width() * scale) / 2.0;
    let pad_y = (height as f64 - frame.height() * scale) / 2.0;
    let polygons: Vec<Vec<Pt>> = outline
        .contours
        .iter()
        .filter(|contour| contour.closed)
        .map(|contour| {
            flatten(contour, 12)
                .into_iter()
                .map(|p| {
                    Pt::new(
                        pad_x + (p.x - frame.min.x) * scale,
                        pad_y + (frame.max.y - p.y) * scale,
                    )
                })
                .collect()
        })
        .collect();
    let mut row = vec![0f64; width];
    for y in 0..height {
        row.iter_mut().for_each(|cell| *cell = 0.0);
        for sub in 0..SUB {
            let sample = y as f64 + (sub as f64 + 0.5) / SUB as f64;
            for (left, right) in scanline_spans(&polygons, sample) {
                let left = left.clamp(0.0, width as f64);
                let right = right.clamp(0.0, width as f64);
                let mut x = left.floor() as usize;
                while (x as f64) < right && x < width {
                    let cover = (right.min(x as f64 + 1.0) - left.max(x as f64)).max(0.0);
                    row[x] += cover / SUB as f64;
                    x += 1;
                }
            }
        }
        for (x, cover) in row.iter().enumerate() {
            mask[y * width + x] = (cover.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn on(x: f64, y: f64) -> OutlinePoint {
        OutlinePoint {
            at: Pt::new(x, y),
            on: true,
            smooth: false,
        }
    }

    fn off(x: f64, y: f64) -> OutlinePoint {
        OutlinePoint {
            at: Pt::new(x, y),
            on: false,
            smooth: false,
        }
    }

    fn close(a: Pt, b: Pt) -> bool {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
    }

    fn canvas(width: f64, height: f64) -> Bounds {
        Bounds {
            min: Pt::new(0.0, 0.0),
            max: Pt::new(width, height),
        }
    }

    #[test]
    fn fit_centers_and_flips_the_glyph() {
        let glyph = Bounds {
            min: Pt::new(0.0, -200.0),
            max: Pt::new(500.0, 800.0),
        };
        let view = Viewport::fit(canvas(800.0, 600.0), glyph, 50.0);
        assert!((view.scale - 0.5).abs() < 1e-9, "height limits: 500 / 1000");
        let top_left = view.to_screen(Pt::new(0.0, 800.0));
        let bottom_right = view.to_screen(Pt::new(500.0, -200.0));
        assert!(close(top_left, Pt::new(275.0, 50.0)), "{top_left:?}");
        assert!(
            close(bottom_right, Pt::new(525.0, 550.0)),
            "{bottom_right:?}"
        );
        let back = view.to_font(Pt::new(400.0, 300.0));
        assert!(close(back, Pt::new(250.0, 300.0)), "{back:?}");
    }

    #[test]
    fn zoom_keeps_the_anchor_still() {
        let glyph = Bounds {
            min: Pt::new(0.0, 0.0),
            max: Pt::new(100.0, 100.0),
        };
        let mut view = Viewport::fit(canvas(200.0, 200.0), glyph, 0.0);
        let anchor = Pt::new(30.0, 170.0);
        let under = view.to_font(anchor);
        view.zoom_at(anchor, 3.0);
        assert!((view.scale - 6.0).abs() < 1e-9);
        assert!(close(view.to_font(anchor), under));
    }

    #[test]
    fn on_point_wins_over_a_nearer_off_point() {
        let outline = Outline {
            name: "o".into(),
            unicode: None,
            advance: 100.0,
            contours: vec![OutlineContour {
                closed: true,
                points: vec![on(0.0, 0.0), off(3.0, 0.0), off(50.0, 50.0), on(100.0, 0.0)],
            }],
        };
        let view = Viewport {
            scale: 1.0,
            origin: Pt::new(0.0, 0.0),
        };
        let hit = hit_test(&outline, &view, Pt::new(2.5, 0.0), 6.0);
        assert_eq!(
            hit,
            Some(Handle {
                contour: 0,
                point: 0
            })
        );
        let only_off = hit_test(&outline, &view, Pt::new(50.0, -50.0), 6.0);
        assert_eq!(
            only_off,
            Some(Handle {
                contour: 0,
                point: 2
            })
        );
        assert_eq!(hit_test(&outline, &view, Pt::new(30.0, 30.0), 6.0), None);
    }

    #[test]
    fn flattens_a_cubic_with_two_offs() {
        let contour = OutlineContour {
            closed: false,
            points: vec![
                on(0.0, 0.0),
                off(0.0, 100.0),
                off(100.0, 100.0),
                on(100.0, 0.0),
            ],
        };
        let line = flatten(&contour, 4);
        assert_eq!(line.len(), 5);
        assert!(close(line[0], Pt::new(0.0, 0.0)));
        assert!(close(line[2], Pt::new(50.0, 75.0)), "{:?}", line[2]);
        assert!(close(line[4], Pt::new(100.0, 0.0)));
    }

    #[test]
    fn flattens_a_quadratic_with_one_off() {
        let contour = OutlineContour {
            closed: true,
            points: vec![on(0.0, 0.0), off(50.0, 100.0), on(100.0, 0.0)],
        };
        let line = flatten(&contour, 2);
        // Quadratic midpoint, the end point, then the straight closing edge back to the start.
        assert_eq!(line.len(), 3);
        assert!(close(line[1], Pt::new(50.0, 50.0)), "{:?}", line[1]);
        assert!(close(line[2], Pt::new(100.0, 0.0)));
    }

    #[test]
    fn closed_contours_wrap_leading_offs() {
        let contour = OutlineContour {
            closed: true,
            points: vec![off(50.0, 100.0), on(100.0, 0.0), on(0.0, 0.0)],
        };
        let line = flatten(&contour, 2);
        assert!(close(line[0], Pt::new(100.0, 0.0)));
        assert!(line.iter().any(|point| close(*point, Pt::new(50.0, 50.0))));
    }

    #[test]
    fn nonzero_fill_leaves_counters_open() {
        let outer = vec![
            Pt::new(0.0, 0.0),
            Pt::new(100.0, 0.0),
            Pt::new(100.0, 100.0),
            Pt::new(0.0, 100.0),
        ];
        let inner = vec![
            Pt::new(25.0, 25.0),
            Pt::new(25.0, 75.0),
            Pt::new(75.0, 75.0),
            Pt::new(75.0, 25.0),
        ];
        let spans = scanline_spans(&[outer.clone(), inner], 50.0);
        assert_eq!(spans, vec![(0.0, 25.0), (75.0, 100.0)]);
        assert_eq!(scanline_spans(&[outer], 50.0), vec![(0.0, 100.0)]);
    }

    #[test]
    fn reads_a_glyph_response() {
        let data = json!({
            "name": "H", "unicode": 72, "advance": 600.0,
            "contours": [{ "closed": true, "points": [
                { "x": 0.0, "y": 0.0, "kind": "on", "smooth": false },
                { "x": 10.0, "y": 20.0, "kind": "off", "smooth": false }
            ]}]
        });
        let outline = Outline::from_json(&data).unwrap();
        assert_eq!(outline.advance, 600.0);
        assert!(!outline.contours[0].points[1].on);
        let bounds = outline.bounds(-200.0, 800.0);
        assert_eq!(bounds.max, Pt::new(600.0, 800.0));
        assert!(Outline::from_json(&json!({ "name": "H" })).is_none());
    }

    fn square_outline() -> Outline {
        Outline {
            name: "sq".into(),
            unicode: Some(65),
            advance: 100.0,
            contours: vec![OutlineContour {
                closed: true,
                points: vec![
                    on(0.0, 0.0),
                    on(100.0, 0.0),
                    on(100.0, 100.0),
                    on(0.0, 100.0),
                ],
            }],
        }
    }

    fn near(a: Pt, b: Pt) -> bool {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
    }

    #[test]
    fn a_lasso_contains_the_points_inside_it() {
        let square = [
            Pt::new(0.0, 0.0),
            Pt::new(10.0, 0.0),
            Pt::new(10.0, 10.0),
            Pt::new(0.0, 10.0),
        ];
        assert!(point_in_polygon(Pt::new(5.0, 5.0), &square));
        assert!(!point_in_polygon(Pt::new(15.0, 5.0), &square));
        assert!(!point_in_polygon(Pt::new(5.0, 5.0), &square[..2]));
        let outline = square_outline();
        let view = Viewport {
            scale: 1.0,
            origin: Pt::new(0.0, 100.0),
        };
        // Font (100, 0) lands at screen (100, 100). The lasso covers that corner only.
        let lasso = [
            Pt::new(90.0, 90.0),
            Pt::new(120.0, 90.0),
            Pt::new(120.0, 120.0),
            Pt::new(90.0, 120.0),
        ];
        let hits = handles_in_polygon(&outline, &view, &lasso);
        assert_eq!(
            hits,
            vec![Handle {
                contour: 0,
                point: 1
            }]
        );
    }

    #[test]
    fn effect_matrices() {
        let p = Pt::new(10.0, 100.0);
        let slanted = matrix::apply(&matrix::slant(45.0), p);
        assert!(near(slanted, Pt::new(110.0, 100.0)), "{slanted:?}");
        let turned = matrix::apply(&matrix::rotate(90.0, Pt::new(0.0, 0.0)), Pt::new(1.0, 0.0));
        assert!(near(turned, Pt::new(0.0, 1.0)), "{turned:?}");
        let scaled = matrix::apply(&matrix::scale(2.0, 0.5, Pt::new(50.0, 50.0)), p);
        assert!(near(scaled, Pt::new(-30.0, 75.0)), "{scaled:?}");
        let flipped = matrix::apply(&matrix::flip_horizontal(50.0), p);
        assert!(near(flipped, Pt::new(90.0, 100.0)));
        let both = matrix::then(&matrix::translate(5.0, 0.0), &matrix::flip_vertical(0.0));
        assert!(near(matrix::apply(&both, p), Pt::new(15.0, -100.0)));
    }

    #[test]
    fn transformed_copy_and_selection() {
        let outline = square_outline();
        let only = [Handle {
            contour: 0,
            point: 1,
        }];
        let moved = outline.transformed(&matrix::translate(5.0, 5.0), Some(&only));
        assert!(near(moved.contours[0].points[1].at, Pt::new(105.0, 5.0)));
        assert!(near(moved.contours[0].points[0].at, Pt::new(0.0, 0.0)));
        let bounds = moved.ink_bounds().unwrap();
        assert_eq!(bounds.max, Pt::new(105.0, 100.0));
        assert_eq!(outline.handles().len(), 4);
    }

    #[test]
    fn marquee_and_segment_hits() {
        let outline = square_outline();
        let view = Viewport {
            scale: 1.0,
            origin: Pt::new(0.0, 0.0),
        };
        // Screen y is flipped: font (100, 0) is screen (100, 0); font (100, 100) is (100, -100).
        let picked = handles_in_rect(&outline, &view, Pt::new(50.0, 10.0), Pt::new(150.0, -150.0));
        assert_eq!(
            picked,
            vec![
                Handle {
                    contour: 0,
                    point: 1
                },
                Handle {
                    contour: 0,
                    point: 2
                }
            ]
        );
        let hit = nearest_segment(&outline, &view, Pt::new(25.0, 2.0), 5.0).unwrap();
        assert_eq!((hit.contour, hit.end), (0, 1));
        assert!((hit.t - 0.25).abs() < 1e-9);
        // The closing edge runs from point 3 back to point 0.
        let closing = nearest_segment(&outline, &view, Pt::new(-1.0, -50.0), 5.0).unwrap();
        assert_eq!(closing.end, 0);
        assert!(nearest_segment(&outline, &view, Pt::new(50.0, -50.0), 5.0).is_none());
    }

    #[test]
    fn rasterizes_with_coverage() {
        let outline = square_outline();
        let frame = Bounds {
            min: Pt::new(0.0, 0.0),
            max: Pt::new(200.0, 100.0),
        };
        let mask = rasterize(&outline, frame, 20, 10);
        // The square fills the left half; the frame is centered, so columns 0-9 are ink.
        assert_eq!(mask[5 * 20 + 2], 255);
        assert_eq!(mask[5 * 20 + 15], 0);
        assert!(mask.iter().filter(|value| **value == 255).count() >= 90);
    }
}
