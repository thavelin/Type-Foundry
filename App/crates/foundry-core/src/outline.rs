//! Outline and spacing checks. Each issue names the glyph, and the contour and point when it has them.

use serde::Serialize;

use crate::font::{Contour, Font, Glyph, Point, PointKind};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutlineIssue {
    pub code: &'static str,
    pub glyph: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contour: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point: Option<usize>,
    pub detail: String,
}

/// Self-intersections, smooth kinks, reversed holes, missing overshoot, and glyphs that float.
pub fn check_outlines(font: &Font) -> Vec<OutlineIssue> {
    let mut issues = Vec::new();
    for glyph in &font.glyphs {
        issues.extend(glyph_issues(font, glyph));
    }
    issues
}

fn glyph_issues(font: &Font, glyph: &Glyph) -> Vec<OutlineIssue> {
    let mut issues = Vec::new();
    let mut areas = Vec::new();
    for (index, contour) in glyph.contours.iter().enumerate() {
        let area = signed_area(contour);
        areas.push(area);
        issues.extend(kinks(glyph, index, contour));
        issues.extend(crossings(glyph, index, contour));
    }
    if let Some((outer, _)) = areas
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
    {
        let outer_ccw = areas[outer] >= 0.0;
        for (index, area) in areas.iter().enumerate() {
            if index == outer || area.abs() < 1.0 {
                continue;
            }
            let inside = glyph.contours[index]
                .points
                .first()
                .is_some_and(|point| point_in_polygon(point, &glyph.contours[outer].points));
            if inside && (*area >= 0.0) == outer_ccw {
                issues.push(issue(
                    glyph,
                    Some(index),
                    None,
                    "direction",
                    format!(
                        "{} contour {index} winds the same way as the outer contour",
                        glyph.name
                    ),
                ));
            }
        }
        if !outer_ccw {
            issues.push(issue(
                glyph,
                Some(outer),
                None,
                "direction",
                format!(
                    "{} contour {outer} is clockwise; outer contours are counter-clockwise",
                    glyph.name
                ),
            ));
        }
    }
    issues.extend(overshoot(font, glyph));
    issues.extend(floating(font, glyph));
    issues
}

fn kinks(glyph: &Glyph, index: usize, contour: &Contour) -> Vec<OutlineIssue> {
    let mut issues = Vec::new();
    for (point_index, point) in contour.points.iter().enumerate() {
        if point.kind != PointKind::On || !point.smooth {
            continue;
        }
        let prev =
            &contour.points[neighbor(point_index, contour.points.len(), contour.closed, false)];
        let next =
            &contour.points[neighbor(point_index, contour.points.len(), contour.closed, true)];
        let incoming = (point.x - prev.x, point.y - prev.y);
        let outgoing = (next.x - point.x, next.y - point.y);
        let in_len = (incoming.0 * incoming.0 + incoming.1 * incoming.1).sqrt();
        let out_len = (outgoing.0 * outgoing.0 + outgoing.1 * outgoing.1).sqrt();
        if in_len < 1e-6 || out_len < 1e-6 {
            continue;
        }
        let cosine = ((incoming.0 * outgoing.0 + incoming.1 * outgoing.1) / (in_len * out_len))
            .clamp(-1.0, 1.0);
        let degrees = cosine.acos().to_degrees();
        if degrees > 12.0 {
            issues.push(issue(
                glyph,
                Some(index),
                Some(point_index),
                "kink",
                format!(
                    "{} contour {index} point {point_index} is smooth but turns {degrees:.0}°",
                    glyph.name
                ),
            ));
        }
    }
    issues
}

fn crossings(glyph: &Glyph, index: usize, contour: &Contour) -> Vec<OutlineIssue> {
    let samples = sample_contour(contour);
    let mut issues = Vec::new();
    if samples.len() < 4 {
        return issues;
    }
    let segments = samples.len() - 1;
    for left in 0..segments {
        for right in (left + 2)..segments {
            if contour.closed && left == 0 && right + 1 == segments {
                continue;
            }
            if segments_cross(
                samples[left],
                samples[left + 1],
                samples[right],
                samples[right + 1],
            ) {
                issues.push(issue(
                    glyph,
                    Some(index),
                    None,
                    "intersection",
                    format!("{} contour {index} crosses itself", glyph.name),
                ));
                return issues;
            }
        }
    }
    issues
}

fn overshoot(font: &Font, glyph: &Glyph) -> Vec<OutlineIssue> {
    let mut issues = Vec::new();
    let Some(top) = glyph
        .contours
        .iter()
        .flat_map(|contour| contour.points.iter())
        .map(|point| point.y)
        .max_by(f64::total_cmp)
    else {
        return issues;
    };
    let curved = glyph.contours.iter().any(|contour| {
        contour
            .points
            .iter()
            .any(|point| point.kind == PointKind::Off)
    });
    if !curved {
        return issues;
    }
    for metric in [font.metrics.x_height, font.metrics.cap_height] {
        if (top - metric).abs() <= 0.5 {
            issues.push(issue(
                glyph,
                None,
                None,
                "overshoot",
                format!(
                    "{} reaches {top:.0}, on a metric line, and its top is a curve",
                    glyph.name
                ),
            ));
            break;
        }
    }
    issues
}

fn floating(font: &Font, glyph: &Glyph) -> Vec<OutlineIssue> {
    let letter = glyph
        .unicode
        .and_then(char::from_u32)
        .is_some_and(|ch| ch.is_ascii_alphanumeric());
    if !letter {
        return Vec::new();
    }
    let Some(bottom) = glyph
        .contours
        .iter()
        .flat_map(|contour| contour.points.iter())
        .map(|point| point.y)
        .min_by(f64::total_cmp)
    else {
        return Vec::new();
    };
    if bottom > 20.0 && bottom < font.metrics.x_height {
        vec![issue(
            glyph,
            None,
            None,
            "baseline",
            format!("{} sits at {bottom:.0}, above the baseline", glyph.name),
        )]
    } else {
        Vec::new()
    }
}

fn issue(
    glyph: &Glyph,
    contour: Option<usize>,
    point: Option<usize>,
    code: &'static str,
    detail: String,
) -> OutlineIssue {
    OutlineIssue {
        code,
        glyph: glyph.name.clone(),
        contour,
        point,
        detail,
    }
}

fn neighbor(index: usize, count: usize, closed: bool, forward: bool) -> usize {
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
        0
    }
}

fn signed_area(contour: &Contour) -> f64 {
    let points = &contour.points;
    if points.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    let last = if contour.closed {
        points.len()
    } else {
        points.len() - 1
    };
    for index in 0..last {
        let here = &points[index];
        let next = &points[(index + 1) % points.len()];
        area += here.x * next.y - next.x * here.y;
    }
    area * 0.5
}

fn point_in_polygon(point: &Point, polygon: &[Point]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for index in 0..polygon.len() {
        let a = &polygon[previous];
        let b = &polygon[index];
        if (a.y > point.y) != (b.y > point.y) {
            let x = (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x;
            if point.x < x {
                inside = !inside;
            }
        }
        previous = index;
    }
    inside
}

fn sample_contour(contour: &Contour) -> Vec<(f64, f64)> {
    let points = &contour.points;
    if points.is_empty() {
        return Vec::new();
    }
    let mut samples = vec![(points[0].x, points[0].y)];
    let last = if contour.closed {
        points.len()
    } else {
        points.len() - 1
    };
    for index in 0..last {
        let end = (index + 1) % points.len();
        samples.push((points[end].x, points[end].y));
    }
    samples
}

fn segments_cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    fn side(p: (f64, f64), q: (f64, f64), r: (f64, f64)) -> f64 {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    }
    let ab_c = side(a, b, c);
    let ab_d = side(a, b, d);
    let cd_a = side(c, d, a);
    let cd_b = side(c, d, b);
    ab_c * ab_d < 0.0 && cd_a * cd_b < 0.0
}

/// Sidebearings, and pairs whose ink comes closer than `min_gap`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpacingIssue {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
    pub detail: String,
}

pub fn check_spacing(
    font: &Font,
    min_gap: f64,
    pairs: Option<&[(String, String)]>,
) -> Result<Vec<SpacingIssue>, crate::FoundryError> {
    if !min_gap.is_finite() || min_gap < 0.0 {
        return Err(crate::FoundryError::Edit(
            "spacing gap must be zero or a positive number".into(),
        ));
    }
    let mut issues = Vec::new();
    let shear = font.style.italic_angle.to_radians().tan();
    for glyph in &font.glyphs {
        if glyph.contours.is_empty() {
            continue;
        }
        let (lsb, rsb) = slant_bearings(glyph, shear);
        if lsb < min_gap || rsb < min_gap {
            issues.push(SpacingIssue {
                code: "bearing",
                left: Some(glyph.name.clone()),
                right: None,
                detail: format!(
                    "{} sidebearings along the italic are {lsb:.0} and {rsb:.0}",
                    glyph.name
                ),
            });
        }
    }
    let wanted: Vec<(String, String)> = match pairs {
        Some(pairs) => pairs.to_vec(),
        None => candidate_pairs(font, min_gap),
    };
    let kerning = crate::kerning::resolved_pairs(font).unwrap_or_default();
    for (left_name, right_name) in wanted {
        let (Some(left), Some(right)) = (font.glyph(&left_name), font.glyph(&right_name)) else {
            issues.push(SpacingIssue {
                code: "missing",
                left: Some(left_name.clone()),
                right: Some(right_name.clone()),
                detail: format!("spacing pair {left_name} {right_name} is missing a glyph"),
            });
            continue;
        };
        let kern = kerning
            .iter()
            .find(|(left, right, _)| left == &left_name && right == &right_name)
            .map(|(_, _, value)| *value)
            .unwrap_or(0.0);
        if pair_gap(left, right, kern) < min_gap {
            issues.push(SpacingIssue {
                code: "collision",
                left: Some(left_name.clone()),
                right: Some(right_name),
                detail: format!(
                    "{left_name} and the following glyph come closer than {min_gap:.0}"
                ),
            });
        }
    }
    Ok(issues)
}

fn slant_bearings(glyph: &Glyph, shear: f64) -> (f64, f64) {
    let mut min_proj = f64::INFINITY;
    let mut max_proj = f64::NEG_INFINITY;
    for point in glyph
        .contours
        .iter()
        .flat_map(|contour| contour.points.iter())
    {
        let projected = point.x + point.y * shear;
        min_proj = min_proj.min(projected);
        max_proj = max_proj.max(projected);
    }
    let right_edge = glyph.advance;
    (min_proj, right_edge - max_proj)
}

fn candidate_pairs(font: &Font, min_gap: f64) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let encoded: Vec<&Glyph> = font
        .glyphs
        .iter()
        .filter(|glyph| glyph.unicode.is_some() && !glyph.contours.is_empty())
        .collect();
    for left in &encoded {
        let left_max = ink_max_x(left);
        for right in &encoded {
            let right_min = ink_min_x(right);
            let gap = left.advance + right_min - left_max;
            if gap < min_gap + 40.0 {
                pairs.push((left.name.clone(), right.name.clone()));
            }
        }
    }
    pairs
}

fn pair_gap(left: &Glyph, right: &Glyph, kern: f64) -> f64 {
    let shift = left.advance + kern;
    let left_segments = glyph_segments(left, 0.0);
    let right_segments = glyph_segments(right, shift);
    let mut best = f64::INFINITY;
    for left_segment in &left_segments {
        for right_segment in &right_segments {
            best = best.min(segment_distance(*left_segment, *right_segment));
        }
    }
    best
}

fn glyph_segments(glyph: &Glyph, shift: f64) -> Vec<((f64, f64), (f64, f64))> {
    let mut segments = Vec::new();
    for contour in &glyph.contours {
        let count = contour.points.len();
        if count < 2 {
            continue;
        }
        let last = if contour.closed { count } else { count - 1 };
        for index in 0..last {
            let a = &contour.points[index];
            let b = &contour.points[(index + 1) % count];
            segments.push(((a.x + shift, a.y), (b.x + shift, b.y)));
        }
    }
    segments
}

fn segment_distance(left: ((f64, f64), (f64, f64)), right: ((f64, f64), (f64, f64))) -> f64 {
    let (a, b) = left;
    let (c, d) = right;
    if segments_cross(a, b, c, d) {
        return 0.0;
    }
    point_segment_distance(a, c, d)
        .min(point_segment_distance(b, c, d))
        .min(point_segment_distance(c, a, b))
        .min(point_segment_distance(d, a, b))
}

fn point_segment_distance(point: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let ab = (b.0 - a.0, b.1 - a.1);
    let length_sq = ab.0 * ab.0 + ab.1 * ab.1;
    if length_sq < 1e-12 {
        let dx = point.0 - a.0;
        let dy = point.1 - a.1;
        return (dx * dx + dy * dy).sqrt();
    }
    let t = (((point.0 - a.0) * ab.0 + (point.1 - a.1) * ab.1) / length_sq).clamp(0.0, 1.0);
    let closest = (a.0 + ab.0 * t, a.1 + ab.1 * t);
    let dx = point.0 - closest.0;
    let dy = point.1 - closest.1;
    (dx * dx + dy * dy).sqrt()
}

fn ink_min_x(glyph: &Glyph) -> f64 {
    glyph
        .contours
        .iter()
        .flat_map(|contour| contour.points.iter())
        .map(|point| point.x)
        .fold(f64::INFINITY, f64::min)
}

fn ink_max_x(glyph: &Glyph) -> f64 {
    glyph
        .contours
        .iter()
        .flat_map(|contour| contour.points.iter())
        .map(|point| point.x)
        .fold(f64::NEG_INFINITY, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::PointKind;

    fn on(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            kind: PointKind::On,
            smooth: false,
        }
    }

    #[test]
    fn flags_a_bowtie_and_a_floating_letter() {
        let mut font = Font::new("Check", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "X".into(),
            unicode: Some(u32::from('X')),
            advance: 200.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    on(0.0, 0.0),
                    on(100.0, 100.0),
                    on(0.0, 100.0),
                    on(100.0, 0.0),
                ],
            }],
        })
        .unwrap();
        font.insert_glyph(Glyph {
            name: "H".into(),
            unicode: Some(u32::from('H')),
            advance: 200.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    on(10.0, 100.0),
                    on(40.0, 100.0),
                    on(40.0, 200.0),
                    on(10.0, 200.0),
                ],
            }],
        })
        .unwrap();
        let issues = check_outlines(&font);
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == "intersection" && issue.glyph == "X")
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == "baseline" && issue.glyph == "H")
        );
    }
}
