//! Move existing points along their normals so a weight change stays compatible.
//!
//! Horizontal and vertical amounts are separate: a stem can grow more than a hairline. By default
//! the point count does not change. Vertical extrema (baseline, overshoot, cap height) stay where
//! they are. Where two edges already face each other, growth stops at `gap` so a counter does not
//! close and a corner does not cross itself.
//!
//! `add_points` with `corner: round` changes the count on purpose. Each sharp corner on the outside
//! of a turn becomes an arc: two on-curve points at the ends of the offset edges, joined by one
//! cubic. That is the only way a round join can be drawn with points, so the result no longer
//! blends with the original master, the same as a stroke.

use serde::Deserialize;

use crate::error::FoundryError;
use crate::font::{Contour, Font, Glyph, Point, PointKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Corner {
    /// Extend the corner until the offset edges meet, clamped so a sharp serif cannot spike.
    Miter,
    /// One point cannot become an arc. The corner moves by the same amount as [`Corner::Angle`].
    Round,
    /// Move along the corner bisector and keep the original corner.
    #[default]
    Angle,
}

/// How [`stroke_font`] turns an offset into a display style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrokeKind {
    /// A band around the original edge: offset out and offset in.
    Outline,
    /// A band inside the letter: two inward offsets.
    Inline,
}

#[derive(Debug, Clone)]
pub struct OffsetOptions {
    pub horizontal: f64,
    pub vertical: f64,
    pub gap: f64,
    pub corner: Corner,
    pub sidebearing: bool,
    pub keep_metrics: bool,
    /// Insert arc points at round joins. Needs `corner: round`.
    pub add_points: bool,
    pub names: Option<Vec<String>>,
}

impl Default for OffsetOptions {
    fn default() -> Self {
        Self {
            horizontal: 0.0,
            vertical: 0.0,
            gap: 0.0,
            corner: Corner::Angle,
            sidebearing: false,
            keep_metrics: true,
            add_points: false,
            names: None,
        }
    }
}

/// Offset the named glyphs, or every glyph. Returns the names that changed.
pub fn offset_font(font: &mut Font, options: &OffsetOptions) -> Result<Vec<String>, FoundryError> {
    check_options(options)?;
    let names = target_names(font, options.names.as_deref())?;
    for name in &names {
        let glyph = font
            .glyph_mut(name)
            .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))?;
        offset_glyph(glyph, options)?;
    }
    Ok(names)
}

/// Build an outline or inline style. Each source contour becomes two, so the result does not
/// blend with the original master.
pub fn stroke_font(
    font: &mut Font,
    options: &OffsetOptions,
    kind: StrokeKind,
) -> Result<Vec<String>, FoundryError> {
    check_options(options)?;
    let names = target_names(font, options.names.as_deref())?;
    let (outer, inner) = match kind {
        StrokeKind::Outline => (1.0, -1.0),
        StrokeKind::Inline => (-1.0, -2.0),
    };
    for name in &names {
        let glyph = font
            .glyph_mut(name)
            .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))?;
        let mut outward = options.clone();
        outward.horizontal *= outer;
        outward.vertical *= outer;
        outward.sidebearing = false;
        let mut inward = options.clone();
        inward.horizontal *= inner;
        inward.vertical *= inner;
        inward.sidebearing = false;
        let mut shell = glyph.clone();
        let mut core = glyph.clone();
        offset_glyph(&mut shell, &outward)?;
        offset_glyph(&mut core, &inward)?;
        for contour in &mut core.contours {
            contour.points.reverse();
        }
        glyph.contours = shell.contours;
        glyph.contours.append(&mut core.contours);
        if options.sidebearing {
            shift_sidebearings(glyph, options.horizontal);
        }
    }
    Ok(names)
}

fn check_options(options: &OffsetOptions) -> Result<(), FoundryError> {
    let values = [options.horizontal, options.vertical, options.gap];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(FoundryError::NonFinite);
    }
    if options.gap < 0.0 {
        return Err(FoundryError::Edit(
            "offset gap must be zero or positive".into(),
        ));
    }
    if options.add_points && options.corner != Corner::Round {
        return Err(FoundryError::Edit(
            "offset add_points needs corner round".into(),
        ));
    }
    Ok(())
}

fn target_names(font: &Font, names: Option<&[String]>) -> Result<Vec<String>, FoundryError> {
    match names {
        None => Ok(font.glyphs.iter().map(|glyph| glyph.name.clone()).collect()),
        Some(names) => {
            for name in names {
                if font.glyph(name).is_none() {
                    return Err(FoundryError::MissingGlyph(name.clone()));
                }
            }
            Ok(names.to_vec())
        }
    }
}

fn offset_glyph(glyph: &mut Glyph, options: &OffsetOptions) -> Result<(), FoundryError> {
    if options.horizontal == 0.0 && options.vertical == 0.0 {
        return Ok(());
    }
    let holes = hole_flags(&glyph.contours);
    let mut proposed: Vec<Vec<Point>> = Vec::with_capacity(glyph.contours.len());
    for (index, contour) in glyph.contours.iter().enumerate() {
        proposed.push(offset_contour(contour, holes[index], options)?);
    }
    if options.gap > 0.0 {
        limit_gaps(&glyph.contours, &mut proposed, options.gap);
    }
    // After gap limiting, which compares points by index. Arcs change the count, so they go last.
    if options.add_points {
        for (index, contour) in glyph.contours.iter().enumerate() {
            proposed[index] = with_round_joins(contour, holes[index], options, &proposed[index]);
        }
    }
    for (contour, points) in glyph.contours.iter_mut().zip(proposed) {
        contour.points = points;
    }
    if options.sidebearing {
        shift_sidebearings(glyph, options.horizontal);
    }
    Ok(())
}

/// The moved points of one contour, with each open round join replaced by its four arc points.
fn with_round_joins(
    contour: &Contour,
    hole: bool,
    options: &OffsetOptions,
    moved: &[Point],
) -> Vec<Point> {
    let ccw = signed_area(&contour.points) >= 0.0;
    let mut out = Vec::with_capacity(moved.len() + 2 * contour.points.len());
    for (index, point) in moved.iter().enumerate() {
        match round_join(contour, index, hole, ccw, options) {
            Some(arc) => out.extend(arc),
            None => out.push(point.clone()),
        }
    }
    out
}

/// The arc that replaces the corner at `index`, or `None` when the corner stays sharp.
///
/// A join is drawn only where the offset opens a gap: on the outside of the turn when growing, or
/// the inside when shrinking. Both amounts must be non-zero, because a join that moves on one
/// axis only has no arc. With `keep_metrics`, a corner on a metric line stays sharp, as it does
/// for the moved point.
fn round_join(
    contour: &Contour,
    index: usize,
    hole: bool,
    ccw: bool,
    options: &OffsetOptions,
) -> Option<[Point; 4]> {
    let count = contour.points.len();
    let corner = &contour.points[index];
    if !contour.closed || count < 3 || corner.kind != PointKind::On || corner.smooth {
        return None;
    }
    if options.horizontal.abs() <= 1e-8 || options.vertical.abs() <= 1e-8 {
        return None;
    }
    if options.keep_metrics && vertical_extremum(&contour.points, index, true) {
        return None;
    }
    let prev = from_point(&contour.points[prev_index(index, count, true)]);
    let here = from_point(corner);
    let next = from_point(&contour.points[next_index(index, count, true)]);
    let in_unit = norm(sub(here, prev));
    let out_unit = norm(sub(next, here));
    if len(in_unit) < 1e-8 || len(out_unit) < 1e-8 {
        return None;
    }
    let turn = dot(in_unit, out_unit).clamp(-1.0, 1.0).acos();
    if turn < 1e-3 {
        return None;
    }
    // The grow side is right of travel for a counter-clockwise contour, flipped for a hole.
    // A left turn bends away from the right side, so the right side is the outside of that turn.
    let cross = in_unit.x * out_unit.y - in_unit.y * out_unit.x;
    let away_is_outside = (cross > 0.0) == (ccw != hole);
    let shrinking = options.horizontal < 0.0 || options.vertical < 0.0;
    if away_is_outside == shrinking {
        return None;
    }
    let in_away = edge_away(in_unit, ccw, hole);
    let out_away = edge_away(out_unit, ccw, hole);
    let start = v(
        here.x + in_away.x * options.horizontal,
        here.y + in_away.y * options.vertical,
    );
    let end = v(
        here.x + out_away.x * options.horizontal,
        here.y + out_away.y * options.vertical,
    );
    // A cubic approximates a circular arc of this angle when its handles are this long.
    let radius = (len(sub(start, here)) + len(sub(end, here))) / 2.0;
    let handle = 4.0 / 3.0 * (turn / 4.0).tan() * radius;
    let first_handle = add(start, mul(in_unit, handle));
    let second_handle = sub(end, mul(out_unit, handle));
    Some([
        arc_point(start, PointKind::On),
        arc_point(first_handle, PointKind::Off),
        arc_point(second_handle, PointKind::Off),
        arc_point(end, PointKind::On),
    ])
}

fn arc_point(at: V2, kind: PointKind) -> Point {
    Point {
        x: at.x,
        y: at.y,
        kind,
        smooth: false,
    }
}

fn shift_sidebearings(glyph: &mut Glyph, horizontal: f64) {
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.x += horizontal;
        }
    }
    glyph.advance += horizontal * 2.0;
}

#[derive(Clone, Copy)]
struct V2 {
    x: f64,
    y: f64,
}

fn v(x: f64, y: f64) -> V2 {
    V2 { x, y }
}

fn add(a: V2, b: V2) -> V2 {
    v(a.x + b.x, a.y + b.y)
}

fn sub(a: V2, b: V2) -> V2 {
    v(a.x - b.x, a.y - b.y)
}

fn mul(a: V2, scale: f64) -> V2 {
    v(a.x * scale, a.y * scale)
}

fn dot(a: V2, b: V2) -> f64 {
    a.x * b.x + a.y * b.y
}

fn len(a: V2) -> f64 {
    dot(a, a).sqrt()
}

fn norm(a: V2) -> V2 {
    let length = len(a);
    if length < 1e-8 {
        v(0.0, 0.0)
    } else {
        mul(a, 1.0 / length)
    }
}

fn from_point(point: &Point) -> V2 {
    v(point.x, point.y)
}

fn offset_contour(
    contour: &Contour,
    hole: bool,
    options: &OffsetOptions,
) -> Result<Vec<Point>, FoundryError> {
    let count = contour.points.len();
    if count == 0 {
        return Ok(Vec::new());
    }
    let area = signed_area(&contour.points);
    let ccw = area >= 0.0;
    let ax = options.horizontal.abs().max(1e-6);
    let ay = options.vertical.abs().max(1e-6);
    // A zero axis is "do not move that way", not a tiny scale. Mixed signs use font space too.
    let anisotropic = options.horizontal.abs() > 1e-8
        && options.vertical.abs() > 1e-8
        && options.horizontal.signum() == options.vertical.signum();
    let sign = if options.horizontal < 0.0 || options.vertical < 0.0 {
        -1.0
    } else {
        1.0
    };
    let mut normals = Vec::with_capacity(count);
    for index in 0..count {
        let prev = from_point(&contour.points[prev_index(index, count, contour.closed)]);
        let here = from_point(&contour.points[index]);
        let next = from_point(&contour.points[next_index(index, count, contour.closed)]);
        let (p0, p1, p2) = if anisotropic {
            (
                v(prev.x / ax, prev.y / ay),
                v(here.x / ax, here.y / ay),
                v(next.x / ax, next.y / ay),
            )
        } else {
            (prev, here, next)
        };
        normals.push(grow_normal(sub(p1, p0), sub(p2, p1), ccw, hole));
    }
    let mut moved = contour.points.clone();
    for index in 0..count {
        let here = from_point(&contour.points[index]);
        let outgoing = normals[index];
        let mut bisector = outgoing;
        if len(bisector) < 1e-8 {
            bisector = normals[prev_index(index, count, contour.closed)];
        }
        let support = dot(bisector, outgoing).abs().max(0.25);
        let miter = match options.corner {
            Corner::Miter => 1.0 / support,
            Corner::Round | Corner::Angle => 1.0,
        };
        let delta = if anisotropic {
            v(
                bisector.x * sign * miter * ax,
                bisector.y * sign * miter * ay,
            )
        } else {
            v(
                bisector.x * options.horizontal * miter,
                bisector.y * options.vertical * miter,
            )
        };
        let mut next = add(here, delta);
        if options.keep_metrics && vertical_extremum(&contour.points, index, contour.closed) {
            next.y = here.y;
        }
        moved[index].x = next.x;
        moved[index].y = next.y;
    }
    Ok(moved)
}

/// Normal that points into the white, so a positive offset makes the letter bolder.
fn grow_normal(incoming: V2, outgoing: V2, ccw: bool, hole: bool) -> V2 {
    let direction = if len(outgoing) >= len(incoming) {
        outgoing
    } else {
        incoming
    };
    let unit = norm(direction);
    if len(unit) < 1e-8 {
        return v(0.0, 0.0);
    }
    let left = v(-unit.y, unit.x);
    let right = v(unit.y, -unit.x);
    let away = if ccw { right } else { left };
    let grow = if hole { mul(away, -1.0) } else { away };
    // The unit bisector of the two edges. The caller uses this normal as it is.
    if len(incoming) > 1e-8 && len(outgoing) > 1e-8 {
        let in_unit = norm(incoming);
        let out_unit = norm(outgoing);
        let in_away = edge_away(in_unit, ccw, hole);
        let out_away = edge_away(out_unit, ccw, hole);
        let mixed = norm(add(in_away, out_away));
        if len(mixed) > 1e-8 {
            return mixed;
        }
    }
    grow
}

fn edge_away(unit: V2, ccw: bool, hole: bool) -> V2 {
    let left = v(-unit.y, unit.x);
    let right = v(unit.y, -unit.x);
    let away = if ccw { right } else { left };
    if hole { mul(away, -1.0) } else { away }
}

fn prev_index(index: usize, count: usize, closed: bool) -> usize {
    if index > 0 {
        index - 1
    } else if closed {
        count - 1
    } else {
        index
    }
}

fn next_index(index: usize, count: usize, closed: bool) -> usize {
    if index + 1 < count {
        index + 1
    } else if closed {
        0
    } else {
        index
    }
}

fn signed_area(points: &[Point]) -> f64 {
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

fn vertical_extremum(points: &[Point], index: usize, closed: bool) -> bool {
    let count = points.len();
    let y = points[index].y;
    let prev = points[prev_index(index, count, closed)].y;
    let next = points[next_index(index, count, closed)].y;
    (y >= prev && y >= next) || (y <= prev && y <= next)
}

fn hole_flags(contours: &[Contour]) -> Vec<bool> {
    let centroids: Vec<V2> = contours
        .iter()
        .map(|contour| {
            if contour.points.is_empty() {
                return v(0.0, 0.0);
            }
            let mut sum = v(0.0, 0.0);
            for point in &contour.points {
                sum = add(sum, from_point(point));
            }
            mul(sum, 1.0 / contour.points.len() as f64)
        })
        .collect();
    contours
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let depth = contours
                .iter()
                .enumerate()
                .filter(|(other, contour)| {
                    *other != index && point_in_polygon(centroids[index], &contour.points)
                })
                .count();
            depth % 2 == 1
        })
        .collect()
}

fn point_in_polygon(point: V2, polygon: &[Point]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for index in 0..polygon.len() {
        let a = &polygon[previous];
        let b = &polygon[index];
        let crosses = (a.y > point.y) != (b.y > point.y);
        if crosses {
            let x = (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x;
            if point.x < x {
                inside = !inside;
            }
        }
        previous = index;
    }
    inside
}

fn limit_gaps(original: &[Contour], proposed: &mut [Vec<Point>], gap: f64) {
    let segments = segments_of(original);
    for (contour_index, contour) in original.iter().enumerate() {
        for (point_index, point) in contour.points.iter().enumerate() {
            let next = &proposed[contour_index][point_index];
            let delta = v(next.x - point.x, next.y - point.y);
            let distance = len(delta);
            if distance < 1e-6 {
                continue;
            }
            let direction = mul(delta, 1.0 / distance);
            let mut allowed = distance;
            for segment in &segments {
                if segment.contour == contour_index
                    && near_point(segment.start, point_index, contour.points.len())
                {
                    continue;
                }
                let Some(hit) = distance_toward(from_point(point), direction, segment) else {
                    continue;
                };
                let share = if segment.opposes { 2.0 } else { 1.0 };
                let room = ((hit - gap) / share).max(0.0);
                if room < allowed {
                    allowed = room;
                }
            }
            proposed[contour_index][point_index].x = point.x + direction.x * allowed;
            proposed[contour_index][point_index].y = point.y + direction.y * allowed;
        }
    }
}

struct Segment {
    contour: usize,
    start: usize,
    a: V2,
    b: V2,
    opposes: bool,
}

fn segments_of(contours: &[Contour]) -> Vec<Segment> {
    let mut segments = Vec::new();
    for (contour_index, contour) in contours.iter().enumerate() {
        let count = contour.points.len();
        if count < 2 {
            continue;
        }
        let last = if contour.closed { count } else { count - 1 };
        for start in 0..last {
            let end = (start + 1) % count;
            let a = from_point(&contour.points[start]);
            let b = from_point(&contour.points[end]);
            let direction = norm(sub(b, a));
            let normal = v(direction.y, -direction.x);
            segments.push(Segment {
                contour: contour_index,
                start,
                a,
                b,
                opposes: len(normal) > 0.0,
            });
        }
    }
    // `opposes` is refined per query. The flag stored here means the segment has a normal.
    segments
}

fn near_point(segment_start: usize, point: usize, count: usize) -> bool {
    if count == 0 {
        return true;
    }
    let incoming = if point == 0 { count - 1 } else { point - 1 };
    segment_start == point || segment_start == incoming
}

/// Distance along `direction` from `origin` to `segment`, when the segment lies ahead and the
/// point is moving toward it. `None` when the segment is beside or behind the move.
fn distance_toward(origin: V2, direction: V2, segment: &Segment) -> Option<f64> {
    let ab = sub(segment.b, segment.a);
    let length = len(ab);
    if length < 1e-8 {
        return None;
    }
    let closest = closest_point(origin, segment.a, segment.b);
    let toward = sub(closest, origin);
    let ahead = dot(toward, direction);
    if ahead <= 1e-4 {
        return None;
    }
    // Ignore a segment we are sliding along rather than approaching.
    let side = len(sub(toward, mul(direction, ahead)));
    if side > 1.5 && ahead > side * 4.0 {
        return None;
    }
    Some(ahead)
}

fn closest_point(point: V2, a: V2, b: V2) -> V2 {
    let ab = sub(b, a);
    let length_sq = dot(ab, ab);
    if length_sq < 1e-12 {
        return a;
    }
    let t = dot(sub(point, a), ab) / length_sq;
    add(a, mul(ab, t.clamp(0.0, 1.0)))
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

    fn square_font() -> Font {
        let mut font = Font::new("Square", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "H".into(),
            unicode: Some(u32::from('H')),
            advance: 400.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    on(100.0, 0.0),
                    on(200.0, 0.0),
                    on(200.0, 700.0),
                    on(100.0, 700.0),
                ],
            }],
        })
        .unwrap();
        font
    }

    #[test]
    fn offset_keeps_points_height_and_grows_sideways() {
        let mut font = square_font();
        let names = offset_font(
            &mut font,
            &OffsetOptions {
                horizontal: 20.0,
                vertical: 4.0,
                keep_metrics: true,
                ..OffsetOptions::default()
            },
        )
        .unwrap();
        assert_eq!(names, vec!["H"]);
        let points = &font.glyph("H").unwrap().contours[0].points;
        assert_eq!(points.len(), 4);
        assert!(points.iter().any(|point| point.y == 0.0));
        assert!(points.iter().any(|point| point.y == 700.0));
        let min_x = points
            .iter()
            .map(|point| point.x)
            .fold(f64::INFINITY, f64::min);
        let max_x = points
            .iter()
            .map(|point| point.x)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(min_x < 100.0, "{min_x}");
        assert!(max_x > 200.0, "{max_x}");
        assert!((max_x - min_x) > 100.0);
    }

    #[test]
    fn round_joins_add_arc_points_on_the_outside_only() {
        // Square (100,0)-(200,0)-(200,700)-(100,700), counter-clockwise. Keep metrics off so the
        // corners on the baseline and cap height may move.
        let round = |horizontal: f64, vertical: f64, keep_metrics: bool| OffsetOptions {
            horizontal,
            vertical,
            keep_metrics,
            corner: Corner::Round,
            add_points: true,
            ..OffsetOptions::default()
        };
        let mut font = square_font();
        offset_font(&mut font, &round(5.0, 5.0, false)).unwrap();
        let points = &font.glyph("H").unwrap().contours[0].points;
        // Four corners, each replaced by two on-curve ends and two off-curve handles.
        assert_eq!(points.len(), 16);
        assert_eq!(
            points.iter().filter(|p| p.kind == PointKind::Off).count(),
            8
        );
        let ends: Vec<(f64, f64)> = points
            .iter()
            .filter(|p| p.kind == PointKind::On)
            .map(|p| (p.x, p.y))
            .collect();
        for expected in [
            (95.0, 0.0),
            (100.0, -5.0),
            (200.0, -5.0),
            (205.0, 0.0),
            (205.0, 700.0),
            (200.0, 705.0),
            (100.0, 705.0),
            (95.0, 700.0),
        ] {
            assert!(ends.contains(&expected), "missing {expected:?} in {ends:?}");
        }

        // Shrinking the same square opens no gap, so every corner stays sharp.
        let mut shrunk = square_font();
        offset_font(&mut shrunk, &round(-5.0, -5.0, false)).unwrap();
        assert_eq!(shrunk.glyph("H").unwrap().contours[0].points.len(), 4);

        // With keep_metrics, every corner of this box sits on a metric line, so none is rounded.
        let mut kept = square_font();
        offset_font(&mut kept, &round(5.0, 5.0, true)).unwrap();
        assert_eq!(kept.glyph("H").unwrap().contours[0].points.len(), 4);

        // A join that moves on one axis only has no arc, and add_points needs corner round.
        let mut one_axis = square_font();
        offset_font(&mut one_axis, &round(5.0, 0.0, false)).unwrap();
        assert_eq!(one_axis.glyph("H").unwrap().contours[0].points.len(), 4);
        let mismatch = OffsetOptions {
            horizontal: 5.0,
            vertical: 5.0,
            add_points: true,
            keep_metrics: false,
            ..OffsetOptions::default()
        };
        assert!(offset_font(&mut square_font(), &mismatch).is_err());
    }

    #[test]
    fn collision_keeps_a_counter_open() {
        let mut font = Font::new("Counter", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "o".into(),
            unicode: Some(u32::from('o')),
            advance: 400.0,
            contours: vec![
                Contour {
                    closed: true,
                    points: vec![
                        on(0.0, 0.0),
                        on(200.0, 0.0),
                        on(200.0, 200.0),
                        on(0.0, 200.0),
                    ],
                },
                Contour {
                    closed: true,
                    points: vec![
                        on(80.0, 40.0),
                        on(80.0, 100.0),
                        on(80.0, 160.0),
                        on(120.0, 160.0),
                        on(160.0, 160.0),
                        on(160.0, 100.0),
                        on(160.0, 40.0),
                        on(120.0, 40.0),
                    ],
                },
            ],
        })
        .unwrap();
        offset_font(
            &mut font,
            &OffsetOptions {
                horizontal: 40.0,
                vertical: 0.0,
                gap: 10.0,
                keep_metrics: false,
                ..OffsetOptions::default()
            },
        )
        .unwrap();
        let inner = &font.glyph("o").unwrap().contours[1].points;
        assert_eq!(inner.len(), 8);
        let left = inner
            .iter()
            .find(|point| (point.y - 100.0).abs() < 1.0 && point.x < 120.0)
            .unwrap();
        let right = inner
            .iter()
            .find(|point| (point.y - 100.0).abs() < 1.0 && point.x > 120.0)
            .unwrap();
        assert!(
            right.x - left.x >= 9.0,
            "counter closed: {} to {}",
            left.x,
            right.x
        );
        assert!(left.x > 80.0, "left side did not move: {}", left.x);
    }

    #[test]
    fn sidebearing_grows_with_the_offset() {
        let mut font = square_font();
        offset_font(
            &mut font,
            &OffsetOptions {
                horizontal: 15.0,
                vertical: 0.0,
                sidebearing: true,
                ..OffsetOptions::default()
            },
        )
        .unwrap();
        let glyph = font.glyph("H").unwrap();
        assert!((glyph.advance - 430.0).abs() < 1e-6, "{}", glyph.advance);
        let min_x = glyph.contours[0]
            .points
            .iter()
            .map(|point| point.x)
            .fold(f64::INFINITY, f64::min);
        assert!(min_x > 100.0, "{min_x}");
    }
}
