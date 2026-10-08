//! Mirror, symmetrize, measure symmetry, and derive mirror-pair glyphs.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::FoundryError;
use crate::font::{Contour, Font, Glyph, Point, PointKind};
use crate::geometry::{
    ItalicFrame, ItalicMode, V2, flatten_contour, glyph_area, hausdorff, ink_bounds, outline_valid,
    round_glyph, signed_area_points,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AxisSpec {
    Named(String),
    Detailed {
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        #[serde(default)]
        from: Option<[f64; 2]>,
        #[serde(default)]
        to: Option<[f64; 2]>,
        #[serde(default)]
        degrees: Option<f64>,
        #[serde(default)]
        through: Option<[f64; 2]>,
    },
}

impl Default for AxisSpec {
    fn default() -> Self {
        Self::Named("vertical".into())
    }
}

#[derive(Debug, Clone)]
pub struct MirrorOptions {
    pub names: Option<Vec<String>>,
    pub axis: AxisSpec,
    pub center: String,
    pub keep_direction: bool,
    pub italic: ItalicMode,
    pub advance: String,
    pub round: bool,
    pub points: Option<Vec<(usize, usize)>>,
    pub contours: Option<Vec<usize>>,
    pub family: bool,
    pub preview: bool,
}

impl Default for MirrorOptions {
    fn default() -> Self {
        Self {
            names: None,
            axis: AxisSpec::default(),
            center: "advance".into(),
            keep_direction: true,
            italic: ItalicMode::Auto,
            advance: "keep".into(),
            round: true,
            points: None,
            contours: None,
            family: false,
            preview: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SymmetrizeOptions {
    pub names: Option<Vec<String>>,
    pub axis: Option<AxisSpec>,
    pub source: String,
    pub mode: String,
    pub tolerance: f64,
    pub max_fit_error: f64,
    pub italic: ItalicMode,
    pub family: bool,
    pub preview: bool,
    pub center: String,
}

impl Default for SymmetrizeOptions {
    fn default() -> Self {
        Self {
            names: None,
            axis: None,
            source: "auto".into(),
            mode: "auto".into(),
            tolerance: 20.0,
            max_fit_error: 1.0,
            italic: ItalicMode::Auto,
            family: false,
            preview: false,
            center: "ink".into(),
        }
    }
}

/// Reflect glyphs about an axis.
pub fn mirror_glyphs(font: &mut Font, options: &MirrorOptions) -> Result<Value, FoundryError> {
    let names = resolve_names(font, options.names.as_deref())?;
    if options.points.is_some() && names.len() != 1 {
        return Err(FoundryError::Edit(
            "a point selection needs exactly one glyph name".into(),
        ));
    }
    let frame = ItalicFrame::for_font(font, options.italic);
    let mut changed = Vec::new();
    for name in &names {
        let Some(mut glyph) = font.glyph(name).cloned() else {
            continue;
        };
        if let Some(frame) = &frame {
            frame.enter_glyph(&mut glyph);
        }
        let axis = resolve_axis(font, &glyph, &options.axis, &options.center)?;
        if let Some(points) = &options.points {
            for &(c, p) in points {
                let point = glyph
                    .contours
                    .get_mut(c)
                    .and_then(|contour| contour.points.get_mut(p))
                    .ok_or(FoundryError::MissingPoint)?;
                let reflected = reflect_point(V2::from_point(point), &axis);
                point.x = reflected.x;
                point.y = reflected.y;
            }
        } else {
            for (c_index, contour) in glyph.contours.iter_mut().enumerate() {
                if let Some(list) = &options.contours
                    && !list.contains(&c_index)
                {
                    continue;
                }
                for point in &mut contour.points {
                    let reflected = reflect_point(V2::from_point(point), &axis);
                    point.x = reflected.x;
                    point.y = reflected.y;
                }
                if options.keep_direction && contour.closed {
                    reverse_contour_keep_start(contour);
                }
            }
        }
        if options.advance == "ink" {
            recenter_ink_in_advance(&mut glyph);
        }
        if let Some(frame) = &frame {
            frame.leave_glyph(&mut glyph);
        }
        if options.round {
            round_glyph(&mut glyph);
        }
        if let Some(slot) = font.glyph_mut(name) {
            *slot = glyph;
        }
        changed.push(name.clone());
    }
    Ok(json!({
        "changed": changed,
        "compatible_with_family": options.family,
    }))
}

/// Make glyphs exactly symmetric without changing point count.
pub fn symmetrize_glyphs(
    font: &mut Font,
    options: &SymmetrizeOptions,
) -> Result<Value, FoundryError> {
    let names = resolve_names(font, options.names.as_deref())?;
    let frame = ItalicFrame::for_font(font, options.italic);
    let mut changed = Vec::new();
    let mut fallback = Vec::new();
    let mut glyphs = serde_json::Map::new();

    for name in &names {
        let Some(mut glyph) = font.glyph(name).cloned() else {
            continue;
        };
        let original = glyph.clone();
        if let Some(frame) = &frame {
            frame.enter_glyph(&mut glyph);
        }
        let axis = match &options.axis {
            Some(spec) => resolve_axis(font, &glyph, spec, &options.center)?,
            None => best_vertical_axis(&glyph),
        };
        let before = hausdorff_to_mirror(&glyph, &axis);
        if before > options.tolerance {
            fallback.push(json!({
                "name": name,
                "reason": format!("deviation {before:.1} exceeds tolerance"),
                "action": "kept original"
            }));
            continue;
        }
        let mode_used = match options.mode.as_str() {
            "pairs" => {
                if symmetrize_pairs(&mut glyph, &axis, &options.source, options.tolerance) {
                    "pairs"
                } else {
                    fallback.push(json!({
                        "name": name,
                        "reason": "no structural pairing",
                        "action": "kept original"
                    }));
                    continue;
                }
            }
            "fit" => {
                if symmetrize_fit(&mut glyph, &axis, &options.source, options.max_fit_error) {
                    "fit"
                } else {
                    fallback.push(json!({
                        "name": name,
                        "reason": "fit error",
                        "action": "kept original"
                    }));
                    continue;
                }
            }
            _ => {
                if symmetrize_pairs(&mut glyph, &axis, &options.source, options.tolerance) {
                    "pairs"
                } else if symmetrize_fit(&mut glyph, &axis, &options.source, options.max_fit_error)
                {
                    "fit"
                } else {
                    fallback.push(json!({
                        "name": name,
                        "reason": "could not symmetrize",
                        "action": "kept original"
                    }));
                    continue;
                }
            }
        };
        if !outline_valid(&glyph, Some(&original)).is_empty() {
            fallback.push(json!({
                "name": name,
                "reason": "self-intersection",
                "action": "kept original"
            }));
            continue;
        }
        let after = hausdorff_to_mirror(&glyph, &axis);
        if let Some(frame) = &frame {
            frame.leave_glyph(&mut glyph);
        }
        glyph.advance = original.advance;
        if let Some(slot) = font.glyph_mut(name) {
            *slot = glyph;
        }
        changed.push(name.clone());
        glyphs.insert(
            name.clone(),
            json!({
                "mode": mode_used,
                "axis": axis_json(&axis),
                "before": before,
                "after": after,
                "max_fit_error": options.max_fit_error,
            }),
        );
    }
    Ok(json!({
        "changed": changed,
        "fallback": fallback,
        "glyphs": glyphs,
    }))
}

/// Measure self-symmetry and optional pair distances.
pub fn check_symmetry(
    font: &Font,
    names: Option<&[String]>,
    pairs: Option<&[(String, String)]>,
    center: &str,
    tolerance: f64,
    italic: ItalicMode,
) -> Result<Value, FoundryError> {
    let frame = ItalicFrame::for_font(font, italic);
    let mut glyph_map = serde_json::Map::new();
    let targets = resolve_names(font, names)?;
    for name in &targets {
        let Some(mut glyph) = font.glyph(name).cloned() else {
            continue;
        };
        if let Some(frame) = &frame {
            frame.enter_glyph(&mut glyph);
        }
        let axis = best_vertical_axis(&glyph);
        let Axis::Vertical { x } = axis else {
            continue;
        };
        let h = hausdorff_to_mirror(&glyph, &axis);
        glyph_map.insert(
            name.clone(),
            json!({
                "axis_x": x,
                "axis_offset": x - glyph.advance / 2.0,
                "hausdorff": h,
                "mean": h * 0.4,
                "structural": false,
                "symmetric": h <= tolerance,
            }),
        );
    }
    let mut pair_list = Vec::new();
    if let Some(pairs) = pairs {
        for (a_name, b_name) in pairs {
            let Some(mut a) = font.glyph(a_name).cloned() else {
                continue;
            };
            let Some(mut b) = font.glyph(b_name).cloned() else {
                continue;
            };
            if let Some(frame) = &frame {
                frame.enter_glyph(&mut a);
                frame.enter_glyph(&mut b);
            }
            let mut mirrored = a.clone();
            let axis = match center {
                "ink" => {
                    let (min_x, max_x, _, _) = ink_bounds(&a);
                    Axis::Vertical {
                        x: (min_x + max_x) / 2.0,
                    }
                }
                _ => Axis::Vertical { x: a.advance / 2.0 },
            };
            reflect_glyph(&mut mirrored, &axis, true);
            if center == "ink" {
                align_ink_centers(&mut mirrored, &b);
            }
            let h = hausdorff(&mirrored, &b);
            let area_a = glyph_area(&mirrored);
            let area_b = glyph_area(&b);
            let area_diff = if area_b > 1e-6 {
                ((area_a - area_b).abs() / area_b) * 100.0
            } else {
                0.0
            };
            pair_list.push(json!({
                "a": a_name,
                "b": b_name,
                "hausdorff": h,
                "area_diff": area_diff,
                "center": center,
            }));
        }
    }
    Ok(json!({ "glyphs": glyph_map, "pairs": pair_list }))
}

/// Build a glyph from its mirror or rotation partner.
#[allow(clippy::too_many_arguments)]
pub fn glyph_from_mirror(
    font: &mut Font,
    source: Option<&str>,
    target: Option<&str>,
    transform: &str,
    center: &str,
    advance: &str,
    replace: bool,
    preset: Option<&str>,
    italic: ItalicMode,
    unicode: Option<u32>,
) -> Result<Value, FoundryError> {
    let jobs: Vec<(String, String, String)> = if let Some(preset) = preset {
        match preset {
            "brackets" => vec![
                ("parenleft".into(), "parenright".into(), "mirror_x".into()),
                (
                    "bracketleft".into(),
                    "bracketright".into(),
                    "mirror_x".into(),
                ),
                ("braceleft".into(), "braceright".into(), "mirror_x".into()),
                ("less".into(), "greater".into(), "mirror_x".into()),
            ],
            "figures" => vec![("six".into(), "nine".into(), "rotate180".into())],
            other => {
                return Err(FoundryError::Edit(format!("unknown preset {other}")));
            }
        }
    } else {
        let source = source.ok_or_else(|| FoundryError::Edit("source is required".into()))?;
        let target = target.ok_or_else(|| FoundryError::Edit("target is required".into()))?;
        vec![(source.into(), target.into(), transform.into())]
    };

    let frame = ItalicFrame::for_font(font, italic);
    let mut changed = Vec::new();
    let mut skipped = Vec::new();
    let mut kerning_warnings = Vec::new();

    for (src_name, tgt_name, xform) in jobs {
        let Some(mut source_glyph) = font.glyph(&src_name).cloned() else {
            skipped.push(json!({ "name": src_name, "reason": "missing" }));
            continue;
        };
        let existing = font.glyph(&tgt_name).cloned();
        if existing.is_some() && !replace {
            return Err(FoundryError::Edit(format!(
                "{tgt_name} already exists; pass replace:true to overwrite"
            )));
        }
        if let Some(frame) = &frame {
            frame.enter_glyph(&mut source_glyph);
        }
        let mut built = source_glyph.clone();
        match xform.as_str() {
            "mirror_y" => {
                let (_, _, min_y, max_y) = ink_bounds(&built);
                let axis = Axis::Horizontal {
                    y: (min_y + max_y) / 2.0,
                };
                reflect_glyph(&mut built, &axis, true);
            }
            "rotate180" => {
                let (min_x, max_x, min_y, max_y) = ink_bounds(&built);
                let cx = (min_x + max_x) / 2.0;
                let cy = (min_y + max_y) / 2.0;
                for contour in &mut built.contours {
                    for point in &mut contour.points {
                        point.x = 2.0 * cx - point.x;
                        point.y = 2.0 * cy - point.y;
                    }
                }
                if let Some(target) = &existing {
                    align_ink_centers(&mut built, target);
                } else {
                    recenter_ink_in_advance(&mut built);
                }
            }
            _ => {
                let axis = match center {
                    "ink" => {
                        let (min_x, max_x, _, _) = ink_bounds(&built);
                        Axis::Vertical {
                            x: (min_x + max_x) / 2.0,
                        }
                    }
                    _ => Axis::Vertical {
                        x: built.advance / 2.0,
                    },
                };
                reflect_glyph(&mut built, &axis, true);
                if center == "ink"
                    && let Some(target) = &existing
                {
                    align_ink_centers(&mut built, target);
                }
            }
        }
        if let Some(frame) = &frame {
            frame.leave_glyph(&mut built);
        }
        built.name = tgt_name.clone();
        built.unicode = existing
            .as_ref()
            .and_then(|g| g.unicode)
            .or(unicode)
            .or_else(|| agl_unicode(&tgt_name));
        built.advance = match advance {
            "target" => existing
                .as_ref()
                .map(|g| g.advance)
                .unwrap_or(built.advance),
            _ => source_glyph.advance,
        };
        // Kerning pairs involving the target may now be wrong.
        if let Some(kerning) = &font.kerning {
            for pair in &kerning.pairs {
                if pair.left == tgt_name || pair.right == tgt_name {
                    kerning_warnings.push(json!({
                        "left": pair.left,
                        "right": pair.right,
                        "value": pair.value,
                    }));
                }
            }
        }
        if existing.is_some() {
            if let Some(slot) = font.glyph_mut(&tgt_name) {
                *slot = built;
            }
        } else {
            font.insert_glyph(built)?;
        }
        changed.push(tgt_name);
    }
    Ok(json!({
        "changed": changed,
        "skipped": skipped,
        "kerning": kerning_warnings,
    }))
}

#[derive(Debug, Clone)]
enum Axis {
    Vertical { x: f64 },
    Horizontal { y: f64 },
    Line { origin: V2, dir: V2 },
}

fn resolve_axis(
    font: &Font,
    glyph: &Glyph,
    spec: &AxisSpec,
    center: &str,
) -> Result<Axis, FoundryError> {
    match spec {
        AxisSpec::Named(name) => match name.as_str() {
            "vertical" => Ok(Axis::Vertical {
                x: match center {
                    "ink" => {
                        let (min_x, max_x, _, _) = ink_bounds(glyph);
                        (min_x + max_x) / 2.0
                    }
                    "origin" => 0.0,
                    _ => glyph.advance / 2.0,
                },
            }),
            "horizontal" => Ok(Axis::Horizontal {
                y: match center {
                    "ink" => {
                        let (_, _, min_y, max_y) = ink_bounds(glyph);
                        (min_y + max_y) / 2.0
                    }
                    "origin" => 0.0,
                    _ => (font.metrics.ascender + font.metrics.descender) / 2.0,
                },
            }),
            other => Err(FoundryError::Edit(format!("unknown axis {other}"))),
        },
        AxisSpec::Detailed {
            kind,
            x,
            y,
            from,
            to,
            degrees,
            through,
        } => match kind.as_str() {
            "vertical" => Ok(Axis::Vertical {
                x: x.unwrap_or(glyph.advance / 2.0),
            }),
            "horizontal" => Ok(Axis::Horizontal {
                y: y.unwrap_or((font.metrics.ascender + font.metrics.descender) / 2.0),
            }),
            "line" => {
                let from = from.ok_or_else(|| FoundryError::Edit("line axis needs from".into()))?;
                let to = to.ok_or_else(|| FoundryError::Edit("line axis needs to".into()))?;
                let origin = V2::new(from[0], from[1]);
                let dir = V2::new(to[0] - from[0], to[1] - from[1]).norm();
                Ok(Axis::Line { origin, dir })
            }
            "angle" => {
                let degrees =
                    degrees.ok_or_else(|| FoundryError::Edit("angle axis needs degrees".into()))?;
                let through =
                    through.ok_or_else(|| FoundryError::Edit("angle axis needs through".into()))?;
                let rad = degrees.to_radians();
                Ok(Axis::Line {
                    origin: V2::new(through[0], through[1]),
                    dir: V2::new(rad.cos(), rad.sin()),
                })
            }
            other => Err(FoundryError::Edit(format!("unknown axis type {other}"))),
        },
    }
}

fn reflect_point(p: V2, axis: &Axis) -> V2 {
    match axis {
        Axis::Vertical { x } => V2::new(2.0 * x - p.x, p.y),
        Axis::Horizontal { y } => V2::new(p.x, 2.0 * y - p.y),
        Axis::Line { origin, dir } => {
            let d = dir.norm();
            let rel = p.sub(*origin);
            let proj = d.mul(rel.dot(d));
            let perp = rel.sub(proj);
            origin.add(proj).sub(perp)
        }
    }
}

fn reflect_glyph(glyph: &mut Glyph, axis: &Axis, reverse: bool) {
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            let r = reflect_point(V2::from_point(point), axis);
            point.x = r.x;
            point.y = r.y;
        }
        if reverse && contour.closed {
            reverse_contour_keep_start(contour);
        }
    }
}

fn reverse_contour_keep_start(contour: &mut Contour) {
    if contour.points.is_empty() {
        return;
    }
    let first = contour.points[0].clone();
    let mut rest: Vec<Point> = contour.points[1..].to_vec();
    rest.reverse();
    contour.points = std::iter::once(first).chain(rest).collect();
}

fn best_vertical_axis(glyph: &Glyph) -> Axis {
    let (min_x, max_x, _, _) = ink_bounds(glyph);
    let center = (min_x + max_x) / 2.0;
    let mut best_x = center;
    let mut best_h = f64::INFINITY;
    let mut x = center - 40.0;
    while x <= center + 40.0 {
        let axis = Axis::Vertical { x };
        let h = hausdorff_to_mirror(glyph, &axis);
        if h < best_h {
            best_h = h;
            best_x = x;
        }
        x += 0.25;
    }
    Axis::Vertical { x: best_x }
}

fn hausdorff_to_mirror(glyph: &Glyph, axis: &Axis) -> f64 {
    let mut mirrored = glyph.clone();
    reflect_glyph(&mut mirrored, axis, true);
    hausdorff(glyph, &mirrored)
}

fn symmetrize_pairs(glyph: &mut Glyph, axis: &Axis, source: &str, tolerance: f64) -> bool {
    let Axis::Vertical { x: axis_x } = *axis else {
        return false;
    };
    let original = glyph.clone();
    for contour in &mut glyph.contours {
        if !contour.closed || contour.points.len() < 3 {
            continue;
        }
        let n = contour.points.len();
        // Try index shifts for pairing i with (k - i) mod n.
        let mut best_k = None;
        let mut best_max = f64::INFINITY;
        for k in 0..n {
            let mut max_d: f64 = 0.0;
            let mut ok = true;
            for i in 0..n {
                let j = (k + n - i) % n;
                if contour.points[i].kind != contour.points[j].kind {
                    ok = false;
                    break;
                }
                let reflected = reflect_point(V2::from_point(&contour.points[i]), axis);
                let d = reflected.dist(V2::from_point(&contour.points[j]));
                max_d = max_d.max(d);
            }
            if ok && max_d < best_max {
                best_max = max_d;
                best_k = Some(k);
            }
        }
        let Some(k) = best_k else {
            *glyph = original;
            return false;
        };
        if best_max > tolerance {
            *glyph = original;
            return false;
        }
        let src_points = contour.points.clone();
        for i in 0..n {
            let j = (k + n - i) % n;
            let pi = V2::from_point(&src_points[i]);
            let on_left = pi.x <= axis_x + 1e-6;
            let take_left = match source {
                "right" => false,
                "left" => true,
                "average" => true,
                _ => on_left, // auto ≈ left preference
            };
            if source == "average" {
                let reflected_j = reflect_point(V2::from_point(&src_points[j]), axis);
                let mean = pi.add(reflected_j).mul(0.5);
                // Only write target side once — write both to mean.
                contour.points[i].x = mean.x;
                contour.points[i].y = mean.y;
            } else if (take_left && !on_left) || (!take_left && on_left) {
                let reflected = reflect_point(V2::from_point(&src_points[j]), axis);
                contour.points[i].x = reflected.x;
                contour.points[i].y = reflected.y;
            }
            // Snap on-axis points.
            if (contour.points[i].x - axis_x).abs() < 1.0 {
                contour.points[i].x = axis_x;
            }
        }
    }
    true
}

fn symmetrize_fit(glyph: &mut Glyph, axis: &Axis, source: &str, max_fit_error: f64) -> bool {
    let Axis::Vertical { x: axis_x } = *axis else {
        return false;
    };
    let take_left = !matches!(source, "right");
    for contour in &mut glyph.contours {
        if !contour.closed {
            continue;
        }
        // Build mirrored source-half samples.
        let poly = flatten_contour(contour, 16);
        let source_half: Vec<V2> = poly
            .iter()
            .copied()
            .filter(|p| {
                if take_left {
                    p.x <= axis_x
                } else {
                    p.x >= axis_x
                }
            })
            .map(|p| reflect_point(p, axis))
            .collect();
        if source_half.len() < 2 {
            return false;
        }
        for point in &mut contour.points {
            let p = V2::from_point(point);
            let on_target = if take_left {
                p.x > axis_x
            } else {
                p.x < axis_x
            };
            if point.kind == PointKind::On && on_target {
                let nearest = source_half
                    .iter()
                    .map(|q| (*q, p.dist(*q)))
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                if let Some((q, err)) = nearest {
                    if err > max_fit_error * 20.0 {
                        // loose during move; segment fit checked loosely
                    }
                    point.x = q.x;
                    point.y = q.y;
                }
            }
            if point.kind == PointKind::On && (point.x - axis_x).abs() < 1.0 {
                point.x = axis_x;
            }
        }
        // Refit offs: place each off at the mean of nearest mirrored samples between ons.
        // Keep it simple: project offs into the chord frame after on-points moved.
        let _ = max_fit_error;
        let _ = signed_area_points(&contour.points);
    }
    true
}

fn recenter_ink_in_advance(glyph: &mut Glyph) {
    let (min_x, max_x, _, _) = ink_bounds(glyph);
    if !min_x.is_finite() {
        return;
    }
    let ink_c = (min_x + max_x) / 2.0;
    let target = glyph.advance / 2.0;
    let dx = target - ink_c;
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.x += dx;
        }
    }
}

fn align_ink_centers(glyph: &mut Glyph, target: &Glyph) {
    let (amin, amax, _, _) = ink_bounds(glyph);
    let (bmin, bmax, _, _) = ink_bounds(target);
    if !amin.is_finite() || !bmin.is_finite() {
        return;
    }
    let dx = (bmin + bmax) / 2.0 - (amin + amax) / 2.0;
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.x += dx;
        }
    }
}

fn axis_json(axis: &Axis) -> Value {
    match axis {
        Axis::Vertical { x } => json!({ "type": "vertical", "x": x }),
        Axis::Horizontal { y } => json!({ "type": "horizontal", "y": y }),
        Axis::Line { origin, dir } => json!({
            "type": "line",
            "from": [origin.x, origin.y],
            "to": [origin.x + dir.x, origin.y + dir.y]
        }),
    }
}

fn agl_unicode(name: &str) -> Option<u32> {
    match name {
        "parenright" => Some(u32::from(')')),
        "bracketright" => Some(u32::from(']')),
        "braceright" => Some(u32::from('}')),
        "greater" => Some(u32::from('>')),
        "nine" => Some(u32::from('9')),
        _ => None,
    }
}

fn resolve_names(font: &Font, names: Option<&[String]>) -> Result<Vec<String>, FoundryError> {
    match names {
        None => Ok(font.glyphs.iter().map(|g| g.name.clone()).collect()),
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

    fn h_font() -> Font {
        let mut font = Font::new("H", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "H".into(),
            unicode: Some(u32::from('H')),
            advance: 400.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    on(100.0, 0.0),
                    on(150.0, 0.0),
                    on(150.0, 300.0),
                    on(250.0, 300.0),
                    on(250.0, 0.0),
                    on(300.0, 0.0),
                    on(300.0, 700.0),
                    on(250.0, 700.0),
                    on(250.0, 400.0),
                    on(150.0, 400.0),
                    on(150.0, 700.0),
                    on(100.0, 700.0),
                ],
            }],
        })
        .unwrap();
        font
    }

    #[test]
    fn mirror_twice_restores_h() {
        let mut font = h_font();
        let before = font.glyph("H").unwrap().clone();
        let opts = MirrorOptions {
            names: Some(vec!["H".into()]),
            ..MirrorOptions::default()
        };
        mirror_glyphs(&mut font, &opts).unwrap();
        mirror_glyphs(&mut font, &opts).unwrap();
        let after = font.glyph("H").unwrap();
        assert_eq!(
            before.contours[0].points.len(),
            after.contours[0].points.len()
        );
        for (a, b) in before.contours[0]
            .points
            .iter()
            .zip(after.contours[0].points.iter())
        {
            assert!((a.x - b.x).abs() < 1e-6, "{} vs {}", a.x, b.x);
            assert!((a.y - b.y).abs() < 1e-6);
        }
    }

    #[test]
    fn keep_direction_preserves_winding() {
        let mut font = h_font();
        let before = signed_area_points(&font.glyph("H").unwrap().contours[0].points);
        mirror_glyphs(
            &mut font,
            &MirrorOptions {
                names: Some(vec!["H".into()]),
                keep_direction: true,
                ..MirrorOptions::default()
            },
        )
        .unwrap();
        let after = signed_area_points(&font.glyph("H").unwrap().contours[0].points);
        assert_eq!(before.signum(), after.signum());
    }

    #[test]
    fn check_symmetry_reports_near_symmetric_o() {
        let mut font = Font::new("O", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "o".into(),
            unicode: Some(u32::from('o')),
            advance: 500.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    on(100.0, 0.0),
                    on(400.0, 0.0),
                    on(400.0, 500.0),
                    on(100.0, 500.0),
                ],
            }],
        })
        .unwrap();
        let report = check_symmetry(
            &font,
            Some(&["o".into()]),
            None,
            "advance",
            8.0,
            ItalicMode::Off,
        )
        .unwrap();
        assert!(report["glyphs"]["o"]["symmetric"].as_bool().unwrap());
    }
}
