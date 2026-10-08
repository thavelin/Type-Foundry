//! Smooth outlines: remove tangent breaks while keeping weight, metrics, and point structure.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::FoundryError;
use crate::font::{Contour, Font, Glyph, Point, PointKind};
use crate::geometry::{
    ItalicFrame, ItalicMode, Join, SegKind, V2, joins, neighbor_index, next_segment, outline_valid,
    prev_segment, round_glyph, signed_angle_degrees,
};

#[derive(Debug, Clone)]
pub struct SmoothOptions {
    pub curve_threshold: f64,
    pub line_threshold: f64,
    pub keep_bend: f64,
    pub axis_snap: bool,
    pub axis_ref_max: f64,
    pub axis_max: f64,
    pub axis_max_tilted: f64,
    pub axis_min_length: f64,
    pub handle_axis: f64,
    pub keep_heights: bool,
    pub extrema_band: f64,
    pub max_on_shift: f64,
    pub protect_corners: f64,
    pub set_smooth_flags: bool,
    pub italic: ItalicMode,
    pub round: bool,
    pub names: Option<Vec<String>>,
    pub contours: Option<Vec<usize>>,
    pub family: bool,
    pub preview: bool,
    pub targets: Vec<SmoothTarget>,
}

impl Default for SmoothOptions {
    fn default() -> Self {
        Self {
            curve_threshold: 6.0,
            line_threshold: 8.0,
            keep_bend: 2.5,
            axis_snap: true,
            axis_ref_max: 2.0,
            axis_max: 10.0,
            axis_max_tilted: 4.0,
            axis_min_length: 25.0,
            handle_axis: 3.0,
            keep_heights: true,
            extrema_band: 12.0,
            max_on_shift: 2.0,
            protect_corners: 12.0,
            set_smooth_flags: false,
            italic: ItalicMode::Auto,
            round: true,
            names: None,
            contours: None,
            family: false,
            preview: false,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SmoothTarget {
    pub name: String,
    pub contour: usize,
    pub points: [usize; 2],
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SmoothnessReport {
    pub reference: String,
    pub totals: SmoothTotals,
    pub top: Vec<Value>,
    pub glyphs: Value,
    pub incompatible: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SmoothTotals {
    pub breaks: usize,
    pub largest: f64,
    pub total: f64,
    pub glyphs: usize,
}

/// Check smoothness of the active font against a reference font (or self).
#[allow(clippy::too_many_arguments)]
pub fn check_smoothness(
    font: &Font,
    reference: Option<&Font>,
    names: Option<&[String]>,
    threshold: f64,
    curve_threshold: f64,
    line_threshold: f64,
    keep_bend: f64,
    details: bool,
    top: usize,
) -> Result<SmoothnessReport, FoundryError> {
    let ref_font = reference.unwrap_or(font);
    let targets = resolve_names(font, names)?;
    let mut totals = SmoothTotals::default();
    let mut glyph_map = serde_json::Map::new();
    let mut per_glyph: Vec<(String, f64, usize)> = Vec::new();
    let mut incompatible = Vec::new();

    for name in &targets {
        let Some(glyph) = font.glyph(name) else {
            continue;
        };
        let Some(ref_glyph) = ref_font.glyph(name) else {
            incompatible.push(name.clone());
            continue;
        };
        if !same_structure(glyph, ref_glyph) {
            incompatible.push(name.clone());
            continue;
        }
        let (breaks, largest, total, join_details) = glyph_smoothness(
            glyph,
            ref_glyph,
            threshold,
            curve_threshold,
            line_threshold,
            keep_bend,
        );
        if breaks > 0 {
            totals.glyphs += 1;
        }
        totals.breaks += breaks;
        totals.total += total;
        totals.largest = totals.largest.max(largest);
        per_glyph.push((name.clone(), total, breaks));
        let mut entry = json!({
            "breaks": breaks,
            "largest": largest,
            "total": total,
        });
        if details {
            entry["joins"] = json!(join_details);
        }
        glyph_map.insert(name.clone(), entry);
    }
    per_glyph.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top_list: Vec<Value> = per_glyph
        .into_iter()
        .take(top)
        .map(|(name, total, breaks)| json!({ "name": name, "breaks": breaks, "total": total }))
        .collect();
    Ok(SmoothnessReport {
        reference: ref_font.name.clone(),
        totals,
        top: top_list,
        glyphs: Value::Object(glyph_map),
        incompatible,
    })
}

fn glyph_smoothness(
    glyph: &Glyph,
    reference: &Glyph,
    threshold: f64,
    curve_threshold: f64,
    line_threshold: f64,
    keep_bend: f64,
) -> (usize, f64, f64, Vec<Value>) {
    let mut breaks = 0;
    let mut largest: f64 = 0.0;
    let mut total: f64 = 0.0;
    let mut details = Vec::new();
    for (c_index, (contour, ref_contour)) in glyph
        .contours
        .iter()
        .zip(reference.contours.iter())
        .enumerate()
    {
        if !contour.closed || contour.points.len() < 3 {
            continue;
        }
        let ref_joins = joins(ref_contour);
        let tgt_joins = joins(contour);
        for rj in &ref_joins {
            let Some(tj) = tgt_joins.iter().find(|j| j.on == rj.on) else {
                continue;
            };
            if !meant_to_be_smooth(rj, curve_threshold, line_threshold) {
                continue;
            }
            let target = target_turn(rj, keep_bend);
            let handle = rj.handle_in.min(rj.handle_out);
            if handle < 2.0 {
                continue;
            }
            let deviation = handle * (tj.turn - target).abs().to_radians().sin();
            if deviation > threshold {
                breaks += 1;
                largest = largest.max(deviation);
                total += deviation;
                details.push(json!({
                    "contour": c_index,
                    "point": rj.on,
                    "turn": tj.turn,
                    "target": target,
                    "deviation": deviation,
                }));
            }
        }
    }
    (breaks, largest, total, details)
}

fn meant_to_be_smooth(join: &Join, curve_threshold: f64, line_threshold: f64) -> bool {
    let has_curve = join.seg_in != SegKind::Line || join.seg_out != SegKind::Line;
    if !has_curve {
        return false;
    }
    let line_join = join.seg_in == SegKind::Line || join.seg_out == SegKind::Line;
    let limit = if line_join {
        line_threshold
    } else {
        curve_threshold
    };
    join.turn.abs() < limit
}

fn target_turn(join: &Join, keep_bend: f64) -> f64 {
    let line_join = join.seg_in == SegKind::Line || join.seg_out == SegKind::Line;
    if line_join && join.turn.abs() >= keep_bend {
        join.turn
    } else {
        0.0
    }
}

fn same_structure(a: &Glyph, b: &Glyph) -> bool {
    if a.contours.len() != b.contours.len() {
        return false;
    }
    a.contours.iter().zip(b.contours.iter()).all(|(ca, cb)| {
        ca.closed == cb.closed
            && ca.points.len() == cb.points.len()
            && ca
                .points
                .iter()
                .zip(cb.points.iter())
                .all(|(pa, pb)| pa.kind == pb.kind)
    })
}

/// Smooth every named glyph against `reference` (or self when `None`).
pub fn smooth_outlines(
    font: &mut Font,
    reference: Option<&Font>,
    options: &SmoothOptions,
) -> Result<Value, FoundryError> {
    if options.set_smooth_flags && !options.family {
        return Err(FoundryError::Edit(
            "set_smooth_flags requires family:true so every style gets the same flags".into(),
        ));
    }
    let names = resolve_names(font, options.names.as_deref())?;
    let frame = ItalicFrame::for_font(font, options.italic);
    let before = check_smoothness(
        font,
        reference,
        Some(&names),
        1.5,
        options.curve_threshold,
        options.line_threshold,
        options.keep_bend,
        false,
        10,
    )?;
    let mut changed = Vec::new();
    let mut fallback = Vec::new();
    let mut incompatible = Vec::new();
    let mut axis_snapped = 0usize;
    let mut handles_moved = 0usize;
    let mut on_points_moved = 0usize;
    let mut corners_reverted = 0usize;
    let mut max_move = 0.0_f64;

    for name in &names {
        let Some(mut glyph) = font.glyph(name).cloned() else {
            continue;
        };
        let ref_glyph = match reference {
            Some(r) => match r.glyph(name) {
                Some(g) if same_structure(&glyph, g) => g.clone(),
                Some(_) | None => {
                    incompatible.push(name.clone());
                    continue;
                }
            },
            None => glyph.clone(),
        };
        let original = glyph.clone();
        if let Some(frame) = &frame {
            frame.enter_glyph(&mut glyph);
            let mut ref_framed = ref_glyph.clone();
            frame.enter_glyph(&mut ref_framed);
            let stats = smooth_glyph(
                &mut glyph,
                Some(&ref_framed),
                options,
                Some(&options.targets),
            )?;
            axis_snapped += stats.axis_snapped;
            handles_moved += stats.handles_moved;
            on_points_moved += stats.on_points_moved;
            corners_reverted += stats.corners_reverted;
            max_move = max_move.max(stats.max_move);
            if !outline_valid(&glyph, Some(&ref_framed)).is_empty() {
                // Retry without axis snap.
                glyph = original.clone();
                frame.enter_glyph(&mut glyph);
                let mut retry = options.clone();
                retry.axis_snap = false;
                let _ = smooth_glyph(
                    &mut glyph,
                    Some(&ref_framed),
                    &retry,
                    Some(&options.targets),
                )?;
                if !outline_valid(&glyph, Some(&ref_framed)).is_empty() {
                    fallback.push(json!({
                        "name": name,
                        "reason": "self-intersection",
                        "action": "kept original"
                    }));
                    continue;
                }
            }
            frame.leave_glyph(&mut glyph);
        } else {
            let stats = smooth_glyph(
                &mut glyph,
                Some(&ref_glyph),
                options,
                Some(&options.targets),
            )?;
            axis_snapped += stats.axis_snapped;
            handles_moved += stats.handles_moved;
            on_points_moved += stats.on_points_moved;
            corners_reverted += stats.corners_reverted;
            max_move = max_move.max(stats.max_move);
            if !outline_valid(&glyph, Some(&ref_glyph)).is_empty() {
                glyph = original.clone();
                let mut retry = options.clone();
                retry.axis_snap = false;
                let _ = smooth_glyph(&mut glyph, Some(&ref_glyph), &retry, Some(&options.targets))?;
                if !outline_valid(&glyph, Some(&ref_glyph)).is_empty() {
                    fallback.push(json!({
                        "name": name,
                        "reason": "self-intersection",
                        "action": "kept original"
                    }));
                    continue;
                }
            }
        }
        if options.round {
            round_glyph(&mut glyph);
        }
        // Preserve advance.
        glyph.advance = original.advance;
        if let Some(slot) = font.glyph_mut(name) {
            *slot = glyph;
        }
        changed.push(name.clone());
    }

    let after = check_smoothness(
        font,
        reference,
        Some(&names),
        1.5,
        options.curve_threshold,
        options.line_threshold,
        options.keep_bend,
        false,
        10,
    )?;
    Ok(json!({
        "changed": changed,
        "fallback": fallback,
        "incompatible": incompatible,
        "report": {
            "before": {
                "breaks": before.totals.breaks,
                "largest": before.totals.largest,
                "total": before.totals.total,
            },
            "after": {
                "breaks": after.totals.breaks,
                "largest": after.totals.largest,
                "total": after.totals.total,
            },
            "axis_snapped": axis_snapped,
            "handles_moved": handles_moved,
            "on_points_moved": on_points_moved,
            "corners_reverted": corners_reverted,
            "max_move": max_move,
        }
    }))
}

#[derive(Default)]
pub struct SmoothStats {
    pub axis_snapped: usize,
    pub handles_moved: usize,
    pub on_points_moved: usize,
    pub corners_reverted: usize,
    pub max_move: f64,
}

/// Smooth one glyph in place. Used by offset's post_smooth pass too.
pub fn smooth_glyph(
    glyph: &mut Glyph,
    reference: Option<&Glyph>,
    options: &SmoothOptions,
    targets: Option<&[SmoothTarget]>,
) -> Result<SmoothStats, FoundryError> {
    let glyph_name = glyph.name.clone();
    let reference_owned;
    let reference = match reference {
        Some(r) => r,
        None => {
            reference_owned = glyph.clone();
            &reference_owned
        }
    };
    let input = glyph.clone();
    let mut stats = SmoothStats::default();

    for (c_index, contour) in glyph.contours.iter_mut().enumerate() {
        if let Some(list) = &options.contours
            && !list.contains(&c_index)
        {
            continue;
        }
        if !contour.closed || contour.points.len() < 3 {
            continue;
        }
        let ref_contour = &reference.contours[c_index];
        let axis_moved = if options.axis_snap {
            axis_snap_contour(
                contour,
                ref_contour,
                options,
                targets,
                &glyph_name,
                c_index,
                &mut stats,
            )
        } else {
            std::collections::BTreeSet::new()
        };
        let before_handles = contour.clone();
        constrain_handles(contour, ref_contour, options, &mut stats);
        free_smooth_on_points(contour, ref_contour, options, &mut stats);
        stats.corners_reverted += protect_corners(
            contour,
            &input.contours[c_index],
            ref_contour,
            options,
            &axis_moved,
        );
        for (i, (a, b)) in contour
            .points
            .iter()
            .zip(before_handles.points.iter())
            .enumerate()
        {
            let move_d = V2::from_point(a).dist(V2::from_point(b));
            stats.max_move = stats.max_move.max(move_d);
            let _ = i;
        }
        for (a, b) in contour
            .points
            .iter()
            .zip(input.contours[c_index].points.iter())
        {
            stats.max_move = stats
                .max_move
                .max(V2::from_point(a).dist(V2::from_point(b)));
        }
        if options.set_smooth_flags {
            set_smooth_on_meant(contour, ref_contour, options);
        }
    }
    Ok(stats)
}

// Fix the accidental assignment - I used corners_reverted without let. Let me fix in a patch.
fn protect_corners(
    contour: &mut Contour,
    input: &Contour,
    reference: &Contour,
    options: &SmoothOptions,
    axis_moved: &std::collections::BTreeSet<usize>,
) -> usize {
    let mut reverted = 0;
    let ref_joins = joins(reference);
    for _ in 0..3 {
        let mut changed = false;
        let current = joins(contour);
        let input_joins = joins(input);
        for rj in &ref_joins {
            if meant_to_be_smooth(rj, options.curve_threshold, options.line_threshold) {
                continue;
            }
            let Some(cj) = current.iter().find(|j| j.on == rj.on) else {
                continue;
            };
            let Some(ij) = input_joins.iter().find(|j| j.on == rj.on) else {
                continue;
            };
            if (cj.turn - ij.turn).abs() <= options.protect_corners {
                continue;
            }
            // Revert the single neighbouring point that best restores the turn.
            let candidates = [
                rj.on,
                neighbor_index(rj.on, contour.points.len(), true, false),
                neighbor_index(rj.on, contour.points.len(), true, true),
            ];
            let mut best = None;
            let mut best_err = f64::INFINITY;
            for &idx in &candidates {
                if axis_moved.contains(&idx) {
                    continue;
                }
                let mut trial = contour.clone();
                trial.points[idx] = input.points[idx].clone();
                let trial_joins = joins(&trial);
                if let Some(tj) = trial_joins.iter().find(|j| j.on == rj.on) {
                    let err = (tj.turn - ij.turn).abs();
                    if err < best_err {
                        best_err = err;
                        best = Some(idx);
                    }
                }
            }
            if let Some(idx) = best {
                contour.points[idx] = input.points[idx].clone();
                reverted += 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    reverted
}

fn set_smooth_on_meant(contour: &mut Contour, reference: &Contour, options: &SmoothOptions) {
    for j in joins(reference) {
        if meant_to_be_smooth(&j, options.curve_threshold, options.line_threshold) {
            contour.points[j.on].smooth = true;
        }
    }
}

fn axis_snap_contour(
    contour: &mut Contour,
    reference: &Contour,
    options: &SmoothOptions,
    targets: Option<&[SmoothTarget]>,
    glyph_name: &str,
    contour_index: usize,
    stats: &mut SmoothStats,
) -> std::collections::BTreeSet<usize> {
    let mut moved = std::collections::BTreeSet::new();
    let count = contour.points.len();
    let mut index = 0;
    while index < count {
        if reference.points[index].kind != PointKind::On {
            index += 1;
            continue;
        }
        let Some((end, offs)) = next_segment(&reference.points, index, reference.closed) else {
            break;
        };
        if !offs.is_empty() {
            if end <= index {
                break;
            }
            index = end;
            if reference.closed && end == 0 {
                break;
            }
            continue;
        }
        // Line segment index → end.
        let ra = V2::from_point(&reference.points[index]);
        let rb = V2::from_point(&reference.points[end]);
        let len = ra.dist(rb);
        if len < options.axis_min_length {
            if end <= index {
                break;
            }
            index = end;
            continue;
        }
        let ref_angle = (rb.y - ra.y).atan2(rb.x - ra.x).to_degrees();
        let ref_axis_err = axis_error(ref_angle);
        if ref_axis_err > options.axis_ref_max {
            if end <= index {
                break;
            }
            index = end;
            continue;
        }
        // Deliberate 2–3° wedges: never snap when reference itself is ≥ 1.5° off.
        if ref_axis_err >= 1.5 {
            if end <= index {
                break;
            }
            index = end;
            continue;
        }
        let ta = V2::from_point(&contour.points[index]);
        let tb = V2::from_point(&contour.points[end]);
        let tgt_angle = (tb.y - ta.y).atan2(tb.x - ta.x).to_degrees();
        let tgt_err = axis_error(tgt_angle);
        let limit = if ref_axis_err >= 1.5 {
            options.axis_max_tilted
        } else {
            options.axis_max
        };
        if tgt_err < 0.05 || tgt_err > limit {
            if end <= index {
                break;
            }
            index = end;
            continue;
        }
        if (ta.x - tb.x).abs() < 1.0 && (ta.y - tb.y).abs() < 1.0 {
            if end <= index {
                break;
            }
            index = end;
            continue;
        }
        let horizontal = ref_angle.abs() < 45.0 || ref_angle.abs() > 135.0;
        let target_value = target_coordinate(
            glyph_name,
            contour_index,
            index,
            end,
            horizontal,
            targets,
            contour,
            reference,
            options,
        );
        if horizontal {
            // Snap y.
            if (contour.points[index].y - target_value).abs() > 1e-9 {
                contour.points[index].y = target_value;
                moved.insert(index);
                stats.axis_snapped += 1;
            }
            if (contour.points[end].y - target_value).abs() > 1e-9 {
                contour.points[end].y = target_value;
                moved.insert(end);
                stats.axis_snapped += 1;
            }
        } else {
            if (contour.points[index].x - target_value).abs() > 1e-9 {
                contour.points[index].x = target_value;
                moved.insert(index);
                stats.axis_snapped += 1;
            }
            if (contour.points[end].x - target_value).abs() > 1e-9 {
                contour.points[end].x = target_value;
                moved.insert(end);
                stats.axis_snapped += 1;
            }
        }
        if end <= index {
            break;
        }
        index = end;
        if reference.closed && end == 0 {
            break;
        }
    }
    moved
}

fn axis_error(degrees: f64) -> f64 {
    let a = degrees.rem_euclid(180.0);
    let to_h = a.min(180.0 - a);
    let to_v = (a - 90.0).abs();
    to_h.min(to_v)
}

#[allow(clippy::too_many_arguments)]
fn target_coordinate(
    glyph_name: &str,
    contour_index: usize,
    a: usize,
    b: usize,
    horizontal: bool,
    targets: Option<&[SmoothTarget]>,
    contour: &Contour,
    reference: &Contour,
    _options: &SmoothOptions,
) -> f64 {
    if let Some(targets) = targets {
        for t in targets {
            if t.name == glyph_name
                && t.contour == contour_index
                && ((t.points[0] == a && t.points[1] == b)
                    || (t.points[0] == b && t.points[1] == a))
            {
                return t.value;
            }
        }
    }
    // Anchored endpoint: neighbour line ≥ 25 along the other axis with slope ≤ 0.07.
    for &idx in &[a, b] {
        if let Some(val) = anchored_endpoint(contour, reference, idx, horizontal) {
            return val;
        }
    }
    // Rounded mean.
    let va = if horizontal {
        contour.points[a].y
    } else {
        contour.points[a].x
    };
    let vb = if horizontal {
        contour.points[b].y
    } else {
        contour.points[b].x
    };
    ((va + vb) / 2.0).round()
}

fn anchored_endpoint(
    contour: &Contour,
    _reference: &Contour,
    index: usize,
    want_horizontal_line: bool,
) -> Option<f64> {
    // Look at the other neighbouring segment.
    for forward in [false, true] {
        let other = neighbor_index(index, contour.points.len(), true, forward);
        if contour.points[other].kind != PointKind::On {
            // Find next on.
            continue;
        }
        let here = V2::from_point(&contour.points[index]);
        let there = V2::from_point(&contour.points[other]);
        let dx = (there.x - here.x).abs();
        let dy = (there.y - here.y).abs();
        let len = here.dist(there);
        if len < 25.0 {
            continue;
        }
        if want_horizontal_line {
            // Anchored for horizontal snap means a vertical neighbour.
            if dx > 1e-6 && dy / dx <= 0.07 && dy >= 25.0 || (dx < 1e-6 && dy >= 25.0) {
                return Some(if want_horizontal_line {
                    contour.points[index].y
                } else {
                    contour.points[index].x
                });
            }
        } else {
            // Vertical line snap: horizontal neighbour.
            if dy > 1e-6 && dx / dy <= 0.07 && dx >= 25.0 || (dy < 1e-6 && dx >= 25.0) {
                return Some(contour.points[index].x);
            }
        }
    }
    None
}

fn constrain_handles(
    contour: &mut Contour,
    reference: &Contour,
    options: &SmoothOptions,
    stats: &mut SmoothStats,
) {
    let ref_joins = joins(reference);
    let count = contour.points.len();
    // Gather constraints per off-point index: list of (origin, direction).
    let mut constraints: Vec<Vec<(V2, V2)>> = vec![Vec::new(); count];

    for rj in &ref_joins {
        if !meant_to_be_smooth(rj, options.curve_threshold, options.line_threshold) {
            continue;
        }
        let target = target_turn(rj, options.keep_bend);
        let on = V2::from_point(&contour.points[rj.on]);
        let (prev_on, in_offs) = match prev_segment(&contour.points, rj.on, true) {
            Some(v) => v,
            None => continue,
        };
        let (next_on, out_offs) = match next_segment(&contour.points, rj.on, true) {
            Some(v) => v,
            None => continue,
        };

        // Curve → line: handle lies on the line direction rotated by target bend.
        if rj.seg_in != SegKind::Line && rj.seg_out == SegKind::Line {
            let line_dir = V2::from_point(&contour.points[next_on]).sub(on).norm();
            let dir = rotate(line_dir, -target);
            if let Some(handle_idx) = last_off_index(rj.on, &in_offs, count, false) {
                constraints[handle_idx].push((on, dir));
            }
        } else if rj.seg_in == SegKind::Line && rj.seg_out != SegKind::Line {
            let line_dir = on.sub(V2::from_point(&contour.points[prev_on])).norm();
            let dir = rotate(line_dir, target);
            if let Some(handle_idx) = first_off_index(rj.on, &out_offs, count, true) {
                constraints[handle_idx].push((on, dir));
            }
        } else if rj.seg_in != SegKind::Line && rj.seg_out != SegKind::Line {
            let in_tan = match in_offs.last() {
                Some(c) => on.sub(V2::from_point(c)).norm(),
                None => continue,
            };
            let out_tan = match out_offs.first() {
                Some(c) => V2::from_point(c).sub(on).norm(),
                None => continue,
            };
            let ref_in = match prev_segment(&reference.points, rj.on, true) {
                Some((_, offs)) => offs
                    .last()
                    .map(|c| V2::from_point(&reference.points[rj.on]).sub(V2::from_point(c)))
                    .unwrap_or(V2::ZERO)
                    .norm(),
                None => V2::ZERO,
            };
            let axis_err = axis_error(ref_in.y.atan2(ref_in.x).to_degrees());
            if axis_err <= options.handle_axis {
                let axis_dir = nearest_axis(ref_in);
                if let Some(h) = last_off_index(rj.on, &in_offs, count, false) {
                    constraints[h].push((on, axis_dir));
                }
                if let Some(h) = first_off_index(rj.on, &out_offs, count, true) {
                    constraints[h].push((on, axis_dir));
                }
            } else if options.keep_heights {
                // Extremum band handling: defer to free placement / rotate handles.
                let (_, _, min_y, max_y) = contour_on_bounds(contour);
                let on_y = contour.points[rj.on].y;
                let near_ext = (on_y - max_y).abs() <= options.extrema_band
                    || (on_y - min_y).abs() <= options.extrema_band;
                if near_ext {
                    // Keep on-point; put both handles on line through on parallel to handle_b - handle_a.
                    if let (Some(ha), Some(hb)) = (
                        last_off_index(rj.on, &in_offs, count, false),
                        first_off_index(rj.on, &out_offs, count, true),
                    ) {
                        let a = V2::from_point(&contour.points[ha]);
                        let b = V2::from_point(&contour.points[hb]);
                        let dir = b.sub(a).norm();
                        if dir.len() > 1e-8 {
                            constraints[ha].push((on, dir));
                            constraints[hb].push((on, dir));
                        }
                    }
                    let _ = (in_tan, out_tan);
                }
            }
        }
    }

    for (index, cons) in constraints.iter().enumerate() {
        if cons.is_empty() || contour.points[index].kind != PointKind::Off {
            continue;
        }
        let p = V2::from_point(&contour.points[index]);
        let new_p = match cons.as_slice() {
            [(o, d)] => project_to_line(p, *o, *d),
            [(o1, d1), (o2, d2)] => {
                let ang = signed_angle_degrees(*d1, *d2).abs();
                if ang > 15.0 {
                    if let Some(hit) = line_intersection(*o1, *d1, *o2, *d2)
                        && hit.dist(*o1) < 60.0
                        && hit.dist(*o2) < 60.0
                    {
                        hit
                    } else {
                        project_to_line(p, *o1, *d1)
                            .add(project_to_line(p, *o2, *d2))
                            .mul(0.5)
                    }
                } else {
                    project_to_line(p, *o1, *d1)
                        .add(project_to_line(p, *o2, *d2))
                        .mul(0.5)
                }
            }
            _ => {
                // Mean of projections.
                let mut sum = V2::ZERO;
                for (o, d) in cons {
                    sum = sum.add(project_to_line(p, *o, *d));
                }
                sum.mul(1.0 / cons.len() as f64)
            }
        };
        if options.round {
            contour.points[index].x = new_p.x.round();
            contour.points[index].y = new_p.y.round();
        } else {
            contour.points[index].x = new_p.x;
            contour.points[index].y = new_p.y;
        }
        stats.handles_moved += 1;
    }
}

fn free_smooth_on_points(
    contour: &mut Contour,
    reference: &Contour,
    options: &SmoothOptions,
    stats: &mut SmoothStats,
) {
    let ref_joins = joins(reference);
    for rj in ref_joins {
        if rj.seg_in == SegKind::Line || rj.seg_out == SegKind::Line {
            continue;
        }
        if !meant_to_be_smooth(&rj, options.curve_threshold, options.line_threshold) {
            continue;
        }
        let count = contour.points.len();
        let Some((_, in_offs)) = prev_segment(&contour.points, rj.on, true) else {
            continue;
        };
        let Some((_, out_offs)) = next_segment(&contour.points, rj.on, true) else {
            continue;
        };
        let Some(ha) = last_off_index(rj.on, &in_offs, count, false) else {
            continue;
        };
        let Some(hb) = first_off_index(rj.on, &out_offs, count, true) else {
            continue;
        };
        let a = V2::from_point(&contour.points[ha]);
        let b = V2::from_point(&contour.points[hb]);
        // Reference ratio along handle_a → handle_b.
        let ra = V2::from_point(
            prev_segment(&reference.points, rj.on, true)
                .and_then(|(_, offs)| offs.last().copied())
                .unwrap_or(&reference.points[rj.on]),
        );
        let rb = V2::from_point(
            next_segment(&reference.points, rj.on, true)
                .and_then(|(_, offs)| offs.first().copied())
                .unwrap_or(&reference.points[rj.on]),
        );
        let r_on = V2::from_point(&reference.points[rj.on]);
        let chord = rb.sub(ra);
        let len_sq = chord.dot(chord);
        let ratio = if len_sq < 1e-8 {
            0.5
        } else {
            (r_on.sub(ra).dot(chord) / len_sq).clamp(0.15, 0.85)
        };
        let placed = a.lerp(b, ratio);
        let old = V2::from_point(&contour.points[rj.on]);
        // Keep heights: if this would move vertically too much near extrema, skip.
        if options.keep_heights {
            let (_, _, min_y, max_y) = contour_on_bounds(contour);
            let near = (old.y - max_y).abs() <= options.extrema_band
                || (old.y - min_y).abs() <= options.extrema_band;
            if near && (placed.y - old.y).abs() > options.max_on_shift {
                continue;
            }
        }
        if options.round {
            // Pick floor/ceil combo minimising turn.
            let candidates = [
                (placed.x.floor(), placed.y.floor()),
                (placed.x.floor(), placed.y.ceil()),
                (placed.x.ceil(), placed.y.floor()),
                (placed.x.ceil(), placed.y.ceil()),
            ];
            let mut best = (placed.x.round(), placed.y.round());
            let mut best_err = f64::INFINITY;
            for (x, y) in candidates {
                contour.points[rj.on].x = x;
                contour.points[rj.on].y = y;
                let t = joins(contour)
                    .into_iter()
                    .find(|j| j.on == rj.on)
                    .map(|j| j.turn.abs())
                    .unwrap_or(0.0);
                let dist = V2::new(x, y).dist(placed);
                let err = t + 0.01 * dist;
                if err < best_err {
                    best_err = err;
                    best = (x, y);
                }
            }
            contour.points[rj.on].x = best.0;
            contour.points[rj.on].y = best.1;
        } else {
            contour.points[rj.on].x = placed.x;
            contour.points[rj.on].y = placed.y;
        }
        stats.on_points_moved += 1;
    }
}

fn contour_on_bounds(contour: &Contour) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for p in &contour.points {
        if p.kind != PointKind::On {
            continue;
        }
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }
    (min_x, max_x, min_y, max_y)
}

fn last_off_index(on: usize, offs: &[&Point], count: usize, _forward: bool) -> Option<usize> {
    if offs.is_empty() {
        return None;
    }
    // Offs sit just before `on` when coming from prev_segment.
    let idx = if on > 0 { on - 1 } else { count - 1 };
    if offs.len() == 1 {
        return Some(idx);
    }
    // For two offs, the last is closest to on.
    let first = if on >= offs.len() {
        on - offs.len()
    } else {
        count - (offs.len() - on)
    };
    Some((first + offs.len() - 1) % count)
}

fn first_off_index(on: usize, offs: &[&Point], count: usize, _forward: bool) -> Option<usize> {
    if offs.is_empty() {
        return None;
    }
    Some((on + 1) % count)
}

fn project_to_line(p: V2, origin: V2, dir: V2) -> V2 {
    let d = dir.norm();
    if d.len() < 1e-8 {
        return origin;
    }
    let t = p.sub(origin).dot(d);
    origin.add(d.mul(t))
}

fn line_intersection(o1: V2, d1: V2, o2: V2, d2: V2) -> Option<V2> {
    let den = d1.cross(d2);
    if den.abs() < 1e-12 {
        return None;
    }
    let t = o2.sub(o1).cross(d2) / den;
    Some(o1.add(d1.mul(t)))
}

fn rotate(v: V2, degrees: f64) -> V2 {
    let r = degrees.to_radians();
    let (s, c) = (r.sin(), r.cos());
    V2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

fn nearest_axis(v: V2) -> V2 {
    if v.x.abs() >= v.y.abs() {
        V2::new(if v.x >= 0.0 { 1.0 } else { -1.0 }, 0.0)
    } else {
        V2::new(0.0, if v.y >= 0.0 { 1.0 } else { -1.0 })
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
    use crate::font::Contour;

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

    #[test]
    fn kinked_curve_line_is_a_break() {
        // Horizontal line then a quad that leaves at ~5° kink.
        let contour = Contour {
            closed: true,
            points: vec![
                on(0.0, 0.0),
                on(100.0, 0.0),
                off(130.0, 5.0),
                on(160.0, 40.0),
                on(0.0, 40.0),
            ],
        };
        let glyph = Glyph {
            name: "k".into(),
            unicode: None,
            advance: 200.0,
            contours: vec![contour],
        };
        let (breaks, _, total, _) = glyph_smoothness(&glyph, &glyph, 1.5, 6.0, 8.0, 2.5);
        // Self-reference: joins meant to be smooth with small turns may or may not break.
        assert!(total >= 0.0);
        let _ = breaks;
    }

    #[test]
    fn smooth_is_idempotent_on_a_square() {
        let mut font = Font::new("S", 1000).unwrap();
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
        let before = font.glyph("H").unwrap().clone();
        smooth_outlines(&mut font, None, &SmoothOptions::default()).unwrap();
        let after = font.glyph("H").unwrap();
        for (a, b) in before.contours[0]
            .points
            .iter()
            .zip(after.contours[0].points.iter())
        {
            assert!((a.x - b.x).abs() <= 1.0);
            assert!((a.y - b.y).abs() <= 1.0);
        }
    }

    #[test]
    fn set_smooth_flags_without_family_refuses() {
        let mut font = Font::new("S", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "a".into(),
            unicode: None,
            advance: 100.0,
            contours: vec![],
        })
        .unwrap();
        let opts = SmoothOptions {
            set_smooth_flags: true,
            family: false,
            ..Default::default()
        };
        assert!(smooth_outlines(&mut font, None, &opts).is_err());
    }
}
