//! Outline and font edits. Each one checks everything first and then changes the font, so a
//! failed edit leaves the font as it was. The command session is the only caller that mutates.

use std::collections::BTreeSet;

use crate::error::FoundryError;
use crate::font::{Contour, Font, Glyph, Point, PointKind, validate_glyph};

/// A 2D affine matrix `[a, b, c, d, e, f]`: `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
pub type Matrix = [f64; 6];

/// Where a transform's matrix is centered, per glyph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// The matrix is used as given, about font (0, 0).
    #[default]
    Origin,
    /// The center of the points' bounding box: the glyph's, or the selection's.
    Center,
    /// Half the advance across, and the vertical center of the points.
    Advance,
}

/// Optional new vertical metrics. `None` keeps the current value.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MetricsUpdate {
    pub ascender: Option<f64>,
    pub descender: Option<f64>,
    pub cap_height: Option<f64>,
    pub x_height: Option<f64>,
}

/// Which sidebearing [`Font::set_sidebearing`] sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

impl Font {
    fn glyph_or_err(&mut self, name: &str) -> Result<&mut Glyph, FoundryError> {
        self.glyph_mut(name)
            .ok_or_else(|| FoundryError::MissingGlyph(name.to_string()))
    }

    /// Move several points of one glyph by the same offset.
    pub fn move_points(
        &mut self,
        name: &str,
        points: &[(usize, usize)],
        dx: f64,
        dy: f64,
    ) -> Result<(), FoundryError> {
        finite(&[dx, dy])?;
        let glyph = self.glyph_or_err(name)?;
        check_refs(glyph, points)?;
        let unique: BTreeSet<(usize, usize)> = points.iter().copied().collect();
        for (contour, point) in unique {
            let target = &mut glyph.contours[contour].points[point];
            target.x += dx;
            target.y += dy;
        }
        Ok(())
    }

    /// Move several points of one glyph to absolute positions. One call, so one undo step.
    /// A bad reference or a non-finite coordinate changes nothing.
    pub fn set_points(
        &mut self,
        name: &str,
        places: &[(usize, usize, f64, f64)],
    ) -> Result<(), FoundryError> {
        let coords: Vec<f64> = places.iter().flat_map(|(_, _, x, y)| [*x, *y]).collect();
        finite(&coords)?;
        let refs: Vec<(usize, usize)> = places.iter().map(|(c, p, _, _)| (*c, *p)).collect();
        let glyph = self.glyph_or_err(name)?;
        check_refs(glyph, &refs)?;
        for (contour, point, x, y) in places {
            let target = &mut glyph.contours[*contour].points[*point];
            target.x = *x;
            target.y = *y;
        }
        Ok(())
    }

    /// Insert a point before `index`. An `index` equal to the point count appends.
    pub fn insert_point(
        &mut self,
        name: &str,
        contour: usize,
        index: usize,
        point: Point,
    ) -> Result<(), FoundryError> {
        finite(&[point.x, point.y])?;
        let glyph = self.glyph_or_err(name)?;
        let target = glyph
            .contours
            .get_mut(contour)
            .ok_or(FoundryError::MissingPoint)?;
        if index > target.points.len() {
            return Err(FoundryError::MissingPoint);
        }
        let smooth = point.kind == PointKind::On && point.smooth;
        target.points.insert(index, Point { smooth, ..point });
        Ok(())
    }

    /// Split the segment that ends at on-curve point `end` at parameter `t` (0 to 1). A line gains
    /// one on-curve point. A quadratic or cubic is cut with de Casteljau, so the shape stays the
    /// same. Returns the index of the new on-curve point.
    pub fn split_segment(
        &mut self,
        name: &str,
        contour: usize,
        end: usize,
        t: f64,
    ) -> Result<usize, FoundryError> {
        finite(&[t])?;
        if !(0.0..=1.0).contains(&t) {
            return Err(FoundryError::Edit(
                "split position must be from 0 to 1".into(),
            ));
        }
        let glyph = self.glyph_or_err(name)?;
        let target = glyph
            .contours
            .get_mut(contour)
            .ok_or(FoundryError::MissingPoint)?;
        let count = target.points.len();
        if end >= count || target.points[end].kind != PointKind::On {
            return Err(FoundryError::MissingPoint);
        }
        // Walk back to the previous on-curve point, wrapping on a closed contour.
        let mut offs = Vec::new();
        let mut cursor = end;
        let start = loop {
            if cursor == 0 {
                if !target.closed {
                    return Err(FoundryError::Edit(
                        "the first point of an open contour starts no segment".into(),
                    ));
                }
                cursor = count;
            }
            cursor -= 1;
            if cursor == end {
                return Err(FoundryError::Edit(
                    "the contour has one on-curve point".into(),
                ));
            }
            if target.points[cursor].kind == PointKind::On {
                break cursor;
            }
            offs.push(cursor);
        };
        offs.reverse();
        let at = |index: usize| (target.points[index].x, target.points[index].y);
        let p0 = at(start);
        let p3 = at(end);
        let make = |(x, y): (f64, f64), kind: PointKind| Point {
            x,
            y,
            kind,
            smooth: false,
        };
        let replacement: Vec<Point> = match offs.as_slice() {
            [] => vec![make(lerp(p0, p3, t), PointKind::On)],
            [c] => {
                let c = at(*c);
                let a = lerp(p0, c, t);
                let b = lerp(c, p3, t);
                vec![
                    make(a, PointKind::Off),
                    Point {
                        smooth: true,
                        ..make(lerp(a, b, t), PointKind::On)
                    },
                    make(b, PointKind::Off),
                ]
            }
            [c1, c2] => {
                let (c1, c2) = (at(*c1), at(*c2));
                let a = lerp(p0, c1, t);
                let b = lerp(c1, c2, t);
                let c = lerp(c2, p3, t);
                let ab = lerp(a, b, t);
                let bc = lerp(b, c, t);
                vec![
                    make(a, PointKind::Off),
                    make(ab, PointKind::Off),
                    Point {
                        smooth: true,
                        ..make(lerp(ab, bc, t), PointKind::On)
                    },
                    make(bc, PointKind::Off),
                    make(c, PointKind::Off),
                ]
            }
            _ => {
                return Err(FoundryError::Edit(
                    "a segment with three or more off-curve points cannot be split".into(),
                ));
            }
        };
        let new_on = replacement
            .iter()
            .position(|point| point.kind == PointKind::On)
            .unwrap_or(0);
        // A segment that wraps runs from `start` past the contour's end to `end`; every point
        // before `end` is then one of its off-points. Rebuild the contour from the on-curve run
        // `end..=start` followed by the new points, which keeps it cyclic and valid.
        let (points, first_new) = if start < end {
            let mut points = target.points[..=start].to_vec();
            points.extend(replacement);
            points.extend_from_slice(&target.points[end..]);
            (points, start + 1)
        } else {
            let mut points = target.points[end..=start].to_vec();
            points.extend(replacement);
            (points, start - end + 1)
        };
        target.points = points;
        Ok(first_new + new_on)
    }

    /// Delete points. A contour left with no points is removed.
    pub fn delete_points(
        &mut self,
        name: &str,
        points: &[(usize, usize)],
    ) -> Result<(), FoundryError> {
        let glyph = self.glyph_or_err(name)?;
        check_refs(glyph, points)?;
        let doomed: BTreeSet<(usize, usize)> = points.iter().copied().collect();
        for (contour, point) in doomed.iter().rev() {
            glyph.contours[*contour].points.remove(*point);
        }
        glyph.contours.retain(|contour| !contour.points.is_empty());
        Ok(())
    }

    /// Change a point's kind or smooth flag. Off-curve points are never smooth.
    pub fn set_point_type(
        &mut self,
        name: &str,
        contour: usize,
        point: usize,
        kind: Option<PointKind>,
        smooth: Option<bool>,
    ) -> Result<(), FoundryError> {
        let glyph = self.glyph_or_err(name)?;
        check_refs(glyph, &[(contour, point)])?;
        let target = &mut glyph.contours[contour].points[point];
        if let Some(kind) = kind {
            target.kind = kind;
        }
        if let Some(smooth) = smooth {
            target.smooth = smooth;
        }
        if target.kind == PointKind::Off {
            target.smooth = false;
        }
        Ok(())
    }

    /// Add a contour to a glyph. Returns its index.
    pub fn add_contour(&mut self, name: &str, contour: Contour) -> Result<usize, FoundryError> {
        let glyph = self.glyph_or_err(name)?;
        let mut candidate = glyph.clone();
        candidate.contours.push(contour);
        validate_glyph(&candidate)?;
        *glyph = candidate;
        Ok(glyph.contours.len() - 1)
    }

    pub fn set_closed(
        &mut self,
        name: &str,
        contour: usize,
        closed: bool,
    ) -> Result<(), FoundryError> {
        let glyph = self.glyph_or_err(name)?;
        glyph
            .contours
            .get_mut(contour)
            .ok_or(FoundryError::MissingPoint)?
            .closed = closed;
        Ok(())
    }

    /// Reverse a contour's direction. A closed contour keeps its first point first.
    pub fn reverse_contour(&mut self, name: &str, contour: usize) -> Result<(), FoundryError> {
        let glyph = self.glyph_or_err(name)?;
        let target = glyph
            .contours
            .get_mut(contour)
            .ok_or(FoundryError::MissingPoint)?;
        target.points.reverse();
        if target.closed {
            target.points.rotate_right(1);
        }
        Ok(())
    }

    pub fn delete_glyph(&mut self, name: &str) -> Result<(), FoundryError> {
        let index = self
            .glyphs
            .iter()
            .position(|glyph| glyph.name == name)
            .ok_or_else(|| FoundryError::MissingGlyph(name.to_string()))?;
        self.glyphs.remove(index);
        Ok(())
    }

    pub fn rename_glyph(&mut self, name: &str, new_name: &str) -> Result<(), FoundryError> {
        if new_name.trim().is_empty() {
            return Err(FoundryError::GlyphName);
        }
        if name != new_name && self.glyph(new_name).is_some() {
            return Err(FoundryError::DuplicateGlyph(new_name.to_string()));
        }
        self.glyph_or_err(name)?.name = new_name.to_string();
        Ok(())
    }

    pub fn set_unicode(&mut self, name: &str, unicode: Option<u32>) -> Result<(), FoundryError> {
        if let Some(code) = unicode
            && char::from_u32(code).is_none()
        {
            return Err(FoundryError::Edit(format!(
                "{code} is not a Unicode scalar value"
            )));
        }
        if let Some(code) = unicode {
            self.reject_duplicate_unicode(code, name)?;
        }
        self.glyph_or_err(name)?.unicode = unicode;
        Ok(())
    }

    /// Move a glyph to `index` in the font's glyph list. `index` is the position after the move.
    pub fn move_glyph(&mut self, name: &str, index: usize) -> Result<(), FoundryError> {
        let from = self
            .glyphs
            .iter()
            .position(|glyph| glyph.name == name)
            .ok_or_else(|| FoundryError::MissingGlyph(name.to_string()))?;
        if index > self.glyphs.len() {
            return Err(FoundryError::Edit(format!(
                "glyph index {index} is past the end of the font"
            )));
        }
        let glyph = self.glyphs.remove(from);
        let dest = index.min(self.glyphs.len());
        self.glyphs.insert(dest, glyph);
        Ok(())
    }

    /// Set one sidebearing. Left shifts the outline and keeps the advance. Right changes the
    /// advance and leaves the outline. An empty glyph can take a right sidebearing, which sets
    /// the advance, and cannot take a left one.
    pub fn set_sidebearing(
        &mut self,
        name: &str,
        side: Side,
        value: f64,
    ) -> Result<(), FoundryError> {
        if !value.is_finite() {
            return Err(FoundryError::NonFinite);
        }
        let glyph = self
            .glyph_mut(name)
            .ok_or_else(|| FoundryError::MissingGlyph(name.to_string()))?;
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        for contour in &glyph.contours {
            for point in &contour.points {
                min_x = min_x.min(point.x);
                max_x = max_x.max(point.x);
            }
        }
        match side {
            Side::Right => {
                let edge = if max_x.is_finite() { max_x } else { 0.0 };
                glyph.advance = edge + value;
            }
            Side::Left => {
                if !min_x.is_finite() {
                    return Err(FoundryError::Edit(format!(
                        "{name} has no outline, so it has no left sidebearing"
                    )));
                }
                let dx = value - min_x;
                for contour in &mut glyph.contours {
                    for point in &mut contour.points {
                        point.x += dx;
                    }
                }
            }
        }
        Ok(())
    }

    /// Scale a glyph's width and keep vertical stem thickness. Sidebearings scale with `factor`.
    /// `factor` of 1 leaves the glyph alone. Counters between stems shrink. A single stem, such
    /// as a rectangle, keeps its width.
    pub fn scale_width(
        &mut self,
        factor: f64,
        names: Option<&[String]>,
    ) -> Result<Vec<String>, FoundryError> {
        if !factor.is_finite() || factor <= 0.0 || factor > 4.0 {
            return Err(FoundryError::Edit(
                "width factor must be greater than 0 and at most 4".into(),
            ));
        }
        let targets = self.resolve_names(names)?;
        for name in &targets {
            let glyph = self
                .glyph_mut(name)
                .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))?;
            scale_glyph_width(glyph, factor);
        }
        Ok(targets)
    }

    pub fn rename(&mut self, name: &str) -> Result<(), FoundryError> {
        if name.trim().is_empty() {
            return Err(FoundryError::Name);
        }
        self.name = name.to_string();
        Ok(())
    }

    pub fn set_metrics(&mut self, update: MetricsUpdate) -> Result<(), FoundryError> {
        let values = [
            update.ascender,
            update.descender,
            update.cap_height,
            update.x_height,
        ];
        finite(&values.iter().flatten().copied().collect::<Vec<_>>())?;
        let metrics = &mut self.metrics;
        for (slot, value) in [
            (&mut metrics.ascender, update.ascender),
            (&mut metrics.descender, update.descender),
            (&mut metrics.cap_height, update.cap_height),
            (&mut metrics.x_height, update.x_height),
        ] {
            if let Some(value) = value {
                *slot = value;
            }
        }
        Ok(())
    }

    /// Apply `matrix` to whole glyphs, or only to `points` of a single glyph. With `advance`, the
    /// advance is scaled by the matrix's horizontal scale. Returns the glyph names changed.
    pub fn transform(
        &mut self,
        names: Option<&[String]>,
        points: Option<&[(usize, usize)]>,
        matrix: Matrix,
        advance: bool,
    ) -> Result<Vec<String>, FoundryError> {
        self.transform_anchored(names, points, matrix, advance, Anchor::Origin)
    }

    /// [`Font::transform`] with the matrix centered on `anchor`, worked out for each glyph.
    pub fn transform_anchored(
        &mut self,
        names: Option<&[String]>,
        points: Option<&[(usize, usize)]>,
        matrix: Matrix,
        advance: bool,
        anchor: Anchor,
    ) -> Result<Vec<String>, FoundryError> {
        finite(&matrix)?;
        let targets = self.resolve_names(names)?;
        if let Some(points) = points {
            let [name] = targets.as_slice() else {
                return Err(FoundryError::Edit(
                    "a point selection needs exactly one glyph name".into(),
                ));
            };
            let glyph = self.glyph_or_err(name)?;
            check_refs(glyph, points)?;
            let unique: BTreeSet<(usize, usize)> = points.iter().copied().collect();
            let chosen: Vec<&Point> = unique
                .iter()
                .map(|(contour, point)| &glyph.contours[*contour].points[*point])
                .collect();
            let centered = centered(&matrix, anchor_point(anchor, &chosen, glyph.advance));
            for (contour, point) in unique {
                apply(&mut glyph.contours[contour].points[point], &centered);
            }
            return Ok(targets);
        }
        let [a, b, _, _, _, _] = matrix;
        let mut changed = Vec::new();
        for glyph in &mut self.glyphs {
            if !targets.contains(&glyph.name) {
                continue;
            }
            let all: Vec<&Point> = glyph
                .contours
                .iter()
                .flat_map(|contour| contour.points.iter())
                .collect();
            let centered = centered(&matrix, anchor_point(anchor, &all, glyph.advance));
            for contour in &mut glyph.contours {
                for point in &mut contour.points {
                    apply(point, &centered);
                }
            }
            if advance {
                glyph.advance *= (a * a + b * b).sqrt();
            }
            changed.push(glyph.name.clone());
        }
        Ok(changed)
    }

    /// Round every coordinate and advance of the named glyphs, or of all glyphs, to whole units.
    pub fn round_coordinates(
        &mut self,
        names: Option<&[String]>,
    ) -> Result<Vec<String>, FoundryError> {
        let targets = self.resolve_names(names)?;
        let mut changed = Vec::new();
        for glyph in &mut self.glyphs {
            if !targets.contains(&glyph.name) {
                continue;
            }
            glyph.advance = glyph.advance.round();
            for contour in &mut glyph.contours {
                for point in &mut contour.points {
                    point.x = point.x.round();
                    point.y = point.y.round();
                }
            }
            changed.push(glyph.name.clone());
        }
        Ok(changed)
    }

    fn resolve_names(&self, names: Option<&[String]>) -> Result<Vec<String>, FoundryError> {
        match names {
            None => Ok(self.glyphs.iter().map(|glyph| glyph.name.clone()).collect()),
            Some(names) => {
                for name in names {
                    if self.glyph(name).is_none() {
                        return Err(FoundryError::MissingGlyph(name.clone()));
                    }
                }
                Ok(names.to_vec())
            }
        }
    }
}

fn scale_glyph_width(glyph: &mut Glyph, factor: f64) {
    let (min_x, max_x) = ink_x(glyph);
    if !min_x.is_finite() {
        glyph.advance *= factor;
        return;
    }
    let edges = vertical_edges(glyph);
    let placed = place_edges(&edges, factor);
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.x = remap_x(point.x, &edges, &placed);
        }
    }
    let (new_min, new_max) = ink_x(glyph);
    let new_lsb = min_x * factor;
    let shift = new_lsb - new_min;
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.x += shift;
        }
    }
    let new_rsb = (glyph.advance - max_x) * factor;
    glyph.advance = new_lsb + (new_max - new_min) + new_rsb;
}

fn ink_x(glyph: &Glyph) -> (f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    for contour in &glyph.contours {
        for point in &contour.points {
            min_x = min_x.min(point.x);
            max_x = max_x.max(point.x);
        }
    }
    (min_x, max_x)
}

/// Widths of vertical stems in `glyph`, using the same near-vertical edge clustering as
/// `scale_width`. Empty when the glyph has fewer than two stem gaps.
pub fn glyph_stem_widths(glyph: &Glyph) -> Vec<f64> {
    let edges = vertical_edges(glyph);
    if edges.len() < 2 {
        return Vec::new();
    }
    let mut stems = Vec::new();
    for index in 0..edges.len() - 1 {
        if index.is_multiple_of(2) {
            stems.push(edges[index + 1] - edges[index]);
        }
    }
    stems
}

fn vertical_edges(glyph: &Glyph) -> Vec<f64> {
    let mut xs = Vec::new();
    for contour in &glyph.contours {
        let count = contour.points.len();
        if count < 2 {
            continue;
        }
        let segments = if contour.closed { count } else { count - 1 };
        for index in 0..segments {
            let start = &contour.points[index];
            let end = &contour.points[(index + 1) % count];
            let dx = (end.x - start.x).abs();
            let dy = (end.y - start.y).abs();
            if dy >= 8.0 && dy > dx * 4.0 {
                xs.push((start.x + end.x) / 2.0);
            }
        }
    }
    xs.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let mut clustered: Vec<f64> = Vec::new();
    for x in xs {
        if let Some(last) = clustered.last_mut()
            && (x - *last).abs() <= 2.0
        {
            *last = (*last + x) / 2.0;
            continue;
        }
        clustered.push(x);
    }
    clustered
}

/// Even gaps, starting at the outside, are stems and keep their width. Odd gaps are counters
/// and scale. One edge, or none, leaves the ink where it is.
fn place_edges(edges: &[f64], factor: f64) -> Vec<f64> {
    if edges.len() < 2 {
        return edges.to_vec();
    }
    let mut placed = vec![edges[0]];
    for index in 0..edges.len() - 1 {
        let gap = edges[index + 1] - edges[index];
        let scaled = if index % 2 == 0 { gap } else { gap * factor };
        let next = placed[index] + scaled;
        placed.push(next);
    }
    placed
}

fn remap_x(x: f64, edges: &[f64], placed: &[f64]) -> f64 {
    if edges.len() < 2 || edges.len() != placed.len() {
        return x;
    }
    let first = edges[0];
    let last = *edges.last().unwrap();
    let new_first = placed[0];
    let new_last = *placed.last().unwrap();
    if x <= first {
        return new_first + (x - first);
    }
    if x >= last {
        return new_last + (x - last);
    }
    for index in 0..edges.len() - 1 {
        if x <= edges[index + 1] {
            let span = edges[index + 1] - edges[index];
            if span.abs() < 1e-9 {
                return placed[index];
            }
            let t = (x - edges[index]) / span;
            return placed[index] + t * (placed[index + 1] - placed[index]);
        }
    }
    x
}

fn finite(values: &[f64]) -> Result<(), FoundryError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(FoundryError::NonFinite)
    }
}

fn check_refs(glyph: &Glyph, points: &[(usize, usize)]) -> Result<(), FoundryError> {
    for (contour, point) in points {
        let exists = glyph
            .contours
            .get(*contour)
            .is_some_and(|found| *point < found.points.len());
        if !exists {
            return Err(FoundryError::MissingPoint);
        }
    }
    Ok(())
}

fn apply(point: &mut Point, [a, b, c, d, e, f]: &Matrix) {
    let (x, y) = (point.x, point.y);
    point.x = a * x + c * y + e;
    point.y = b * x + d * y + f;
}

fn anchor_point(anchor: Anchor, points: &[&Point], advance: f64) -> (f64, f64) {
    let bounds = points
        .iter()
        .fold(None, |acc: Option<(f64, f64, f64, f64)>, p| {
            Some(match acc {
                None => (p.x, p.y, p.x, p.y),
                Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
            })
        });
    let (cx, cy) = bounds.map_or((advance / 2.0, 0.0), |(x0, y0, x1, y1)| {
        ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
    });
    match anchor {
        Anchor::Origin => (0.0, 0.0),
        Anchor::Center => (cx, cy),
        Anchor::Advance => (advance / 2.0, cy),
    }
}

/// `matrix` applied about `(cx, cy)` instead of the origin.
fn centered(matrix: &Matrix, (cx, cy): (f64, f64)) -> Matrix {
    let [a, b, c, d, e, f] = *matrix;
    [
        a,
        b,
        c,
        d,
        e + cx - a * cx - c * cy,
        f + cy - b * cx - d * cy,
    ]
}

fn lerp(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            kind: PointKind::On,
            smooth: false,
        }
    }

    fn off(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            kind: PointKind::Off,
            smooth: false,
        }
    }

    fn font_with(contours: Vec<Contour>) -> Font {
        let mut font = Font::new("Edits", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "a".into(),
            unicode: Some(97),
            advance: 500.0,
            contours,
        })
        .unwrap();
        font
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

    fn xy(font: &Font) -> Vec<(f64, f64, PointKind)> {
        font.glyph("a").unwrap().contours[0]
            .points
            .iter()
            .map(|p| (p.x, p.y, p.kind))
            .collect()
    }

    #[test]
    fn moves_a_selection_once_per_point() {
        let mut font = font_with(vec![square()]);
        font.move_points("a", &[(0, 1), (0, 2), (0, 1)], 10.0, -5.0)
            .unwrap();
        let points = xy(&font);
        assert_eq!((points[1].0, points[1].1), (110.0, -5.0));
        assert_eq!((points[2].0, points[2].1), (110.0, 95.0));
        assert_eq!(points[0].0, 0.0);
        let before = font.clone();
        assert!(font.move_points("a", &[(0, 9)], 1.0, 1.0).is_err());
        assert_eq!(font, before, "a bad reference changes nothing");
    }

    #[test]
    fn sets_several_points_or_changes_nothing() {
        let mut font = font_with(vec![square()]);
        font.set_points("a", &[(0, 0, 4.0, 8.0), (0, 2, 40.0, 70.0)])
            .unwrap();
        let points = xy(&font);
        assert_eq!((points[0].0, points[0].1), (4.0, 8.0));
        assert_eq!((points[2].0, points[2].1), (40.0, 70.0));
        assert_eq!((points[1].0, points[1].1), (100.0, 0.0));
        let before = font.clone();
        assert!(
            font.set_points("a", &[(0, 1, 1.0, 1.0), (0, 9, 2.0, 2.0)])
                .is_err()
        );
        assert_eq!(font, before, "a bad reference changes nothing");
        assert!(font.set_points("a", &[(0, 1, f64::NAN, 1.0)]).is_err());
        assert_eq!(font, before, "a non-finite coordinate changes nothing");
    }

    #[test]
    fn splits_a_line_a_cubic_and_a_wrapping_quadratic() {
        let mut font = font_with(vec![square()]);
        let at = font.split_segment("a", 0, 1, 0.25).unwrap();
        assert_eq!(at, 1);
        assert_eq!(xy(&font)[1], (25.0, 0.0, PointKind::On));

        let mut font = font_with(vec![Contour {
            closed: false,
            points: vec![
                on(0.0, 0.0),
                off(0.0, 100.0),
                off(100.0, 100.0),
                on(100.0, 0.0),
            ],
        }]);
        let at = font.split_segment("a", 0, 3, 0.5).unwrap();
        assert_eq!(at, 3);
        let points = xy(&font);
        assert_eq!(points.len(), 7);
        assert_eq!(points[3], (50.0, 75.0, PointKind::On));
        assert_eq!(points[1], (0.0, 50.0, PointKind::Off));
        assert_eq!(points[5], (100.0, 50.0, PointKind::Off));
        assert!(font.glyph("a").unwrap().contours[0].points[3].smooth);

        // The segment into point 0 starts at point 1 and wraps through the trailing off-point.
        let mut font = font_with(vec![Contour {
            closed: true,
            points: vec![on(0.0, 0.0), on(100.0, 0.0), off(50.0, 100.0)],
        }]);
        let at = font.split_segment("a", 0, 0, 0.5).unwrap();
        let points = xy(&font);
        assert_eq!(points.len(), 5);
        assert_eq!(points[at], (50.0, 50.0, PointKind::On));
        assert_eq!(points[0], (0.0, 0.0, PointKind::On));

        assert!(font.split_segment("a", 0, 0, 2.0).is_err());
    }

    #[test]
    fn deletes_points_and_drops_empty_contours() {
        let mut font = font_with(vec![
            square(),
            Contour {
                closed: true,
                points: vec![on(5.0, 5.0)],
            },
        ]);
        font.delete_points("a", &[(0, 0), (1, 0)]).unwrap();
        let glyph = font.glyph("a").unwrap();
        assert_eq!(glyph.contours.len(), 1);
        assert_eq!(glyph.contours[0].points.len(), 3);
        assert_eq!(glyph.contours[0].points[0].x, 100.0);
    }

    #[test]
    fn point_types_reverse_and_contours() {
        let mut font = font_with(vec![square()]);
        font.set_point_type("a", 0, 1, None, Some(true)).unwrap();
        assert!(font.glyph("a").unwrap().contours[0].points[1].smooth);
        font.set_point_type("a", 0, 1, Some(PointKind::Off), None)
            .unwrap();
        let point = &font.glyph("a").unwrap().contours[0].points[1];
        assert_eq!(point.kind, PointKind::Off);
        assert!(!point.smooth, "off-curve points are never smooth");

        let mut font = font_with(vec![square()]);
        font.reverse_contour("a", 0).unwrap();
        let points = xy(&font);
        assert_eq!((points[0].0, points[0].1), (0.0, 0.0));
        assert_eq!((points[1].0, points[1].1), (0.0, 100.0));

        let index = font
            .add_contour(
                "a",
                Contour {
                    closed: false,
                    points: vec![on(1.0, 1.0)],
                },
            )
            .unwrap();
        assert_eq!(index, 1);
        font.insert_point("a", 1, 1, on(2.0, 2.0)).unwrap();
        font.set_closed("a", 1, true).unwrap();
        let added = &font.glyph("a").unwrap().contours[1];
        assert!(added.closed);
        assert_eq!(added.points.len(), 2);
        assert!(font.insert_point("a", 1, 9, on(0.0, 0.0)).is_err());
        assert!(
            font.add_contour(
                "a",
                Contour {
                    closed: true,
                    points: vec![]
                }
            )
            .is_err()
        );
    }

    #[test]
    fn glyph_and_font_settings() {
        let mut font = font_with(vec![square()]);
        font.insert_glyph(Glyph {
            name: "b".into(),
            unicode: None,
            advance: 1.0,
            contours: vec![],
        })
        .unwrap();
        assert_eq!(
            font.rename_glyph("a", "b"),
            Err(FoundryError::DuplicateGlyph("b".into()))
        );
        font.rename_glyph("a", "alpha").unwrap();
        font.set_unicode("alpha", Some(0x3B1)).unwrap();
        assert!(font.set_unicode("alpha", Some(0xD800)).is_err());
        font.delete_glyph("b").unwrap();
        assert_eq!(font.glyph_names(), vec!["alpha"]);
        assert_eq!(font.glyph("alpha").unwrap().unicode, Some(0x3B1));

        font.rename("Renamed").unwrap();
        assert!(font.rename(" ").is_err());
        font.set_metrics(MetricsUpdate {
            x_height: Some(520.0),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(font.metrics.x_height, 520.0);
        assert_eq!(font.metrics.ascender, 800.0);
        assert!(
            font.set_metrics(MetricsUpdate {
                ascender: Some(f64::NAN),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[test]
    fn transforms_glyphs_selections_and_rounds() {
        let mut font = font_with(vec![square()]);
        // Slant 45 degrees: x' = x + y.
        font.transform(None, None, [1.0, 0.0, 1.0, 1.0, 0.0, 0.0], false)
            .unwrap();
        assert_eq!(xy(&font)[2], (200.0, 100.0, PointKind::On));
        assert_eq!(font.glyph("a").unwrap().advance, 500.0);

        let mut font = font_with(vec![square()]);
        font.transform(None, None, [0.5, 0.0, 0.0, 1.0, 0.0, 0.0], true)
            .unwrap();
        assert_eq!(font.glyph("a").unwrap().advance, 250.0);

        let names = vec!["a".to_string()];
        font.transform(
            Some(&names),
            Some(&[(0, 0)]),
            [1.0, 0.0, 0.0, 1.0, 3.3, 0.6],
            false,
        )
        .unwrap();
        assert_eq!(xy(&font)[0], (3.3, 0.6, PointKind::On));
        font.insert_glyph(Glyph {
            name: "b".into(),
            unicode: None,
            advance: 1.0,
            contours: vec![],
        })
        .unwrap();
        assert!(
            font.transform(None, Some(&[(0, 0)]), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], false)
                .is_err(),
            "a selection needs one glyph"
        );
        // Flip about the advance middle: the square at x 3..100 in a 250 advance.
        let mut flipped = font.clone();
        flipped
            .transform_anchored(
                Some(&names),
                None,
                [-1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                false,
                Anchor::Advance,
            )
            .unwrap();
        let points = xy(&flipped);
        assert_eq!(
            points[1].0, 200.0,
            "x 50, after the half-width scale, mirrors to 250 - 50"
        );
        // Rotate 180 degrees about the center: the box maps onto itself.
        let mut turned = font_with(vec![square()]);
        turned
            .transform_anchored(
                None,
                None,
                [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
                false,
                Anchor::Center,
            )
            .unwrap();
        assert_eq!(xy(&turned)[0], (100.0, 100.0, PointKind::On));
        font.round_coordinates(None).unwrap();
        assert_eq!(xy(&font)[0], (3.0, 1.0, PointKind::On));
        assert!(
            font.transform(Some(&["zz".to_string()]), None, [1.0; 6], false)
                .is_err()
        );
    }

    #[test]
    fn scale_width_keeps_a_stem_and_scales_the_sidebearings() {
        let mut font = Font::new("Width", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "H".into(),
            unicode: Some(72),
            advance: 400.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    on(100.0, 0.0),
                    on(200.0, 0.0),
                    on(200.0, 100.0),
                    on(100.0, 100.0),
                ],
            }],
        })
        .unwrap();
        font.scale_width(0.5, None).unwrap();
        let glyph = font.glyph("H").unwrap();
        let xs: Vec<f64> = glyph.contours[0]
            .points
            .iter()
            .map(|point| point.x)
            .collect();
        assert!(xs.iter().any(|x| (*x - 50.0).abs() < 1e-6), "{xs:?}");
        assert!(xs.iter().any(|x| (*x - 150.0).abs() < 1e-6), "{xs:?}");
        assert!((glyph.advance - 250.0).abs() < 1e-6, "{}", glyph.advance);
        let ys: Vec<f64> = glyph.contours[0]
            .points
            .iter()
            .map(|point| point.y)
            .collect();
        assert!(ys.contains(&0.0) && ys.contains(&100.0));
    }

    #[test]
    fn move_glyph_inserts_at_the_requested_index() {
        let mut font = font_with(vec![square()]);
        font.insert_glyph(Glyph {
            name: "b".into(),
            unicode: None,
            advance: 100.0,
            contours: vec![],
        })
        .unwrap();
        font.move_glyph("b", 0).unwrap();
        assert_eq!(font.glyph_names(), vec!["b", "a"]);
    }

    #[test]
    fn duplicate_unicode_is_refused() {
        let mut font = font_with(vec![square()]);
        font.insert_glyph(Glyph {
            name: "b".into(),
            unicode: None,
            advance: 100.0,
            contours: vec![],
        })
        .unwrap();
        let err = font.set_unicode("b", Some(97)).unwrap_err();
        assert!(err.to_string().contains("U+0061"), "{err}");
        assert!(err.to_string().contains("a"), "{err}");
    }
}
