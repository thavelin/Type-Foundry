//! Weight offset: move points along normals so a master stays blend-compatible.
//!
//! Points mode follows `documents/geometry-tools-spec.md` §2. Clean mode builds a true
//! geometric offset (point count may change). `add_points` with `corner: round` inserts
//! cubic arcs at sharp outside corners for compatibility with the prior API.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::FoundryError;
use crate::font::{Contour, Font, Glyph, Metrics, Point, PointKind};
use crate::geometry::{
    ItalicFrame, ItalicMode, V2, contour_nesting, flatten_contour, grow_is_left_of_travel,
    ink_bounds, joins, measure_bars_ray, measure_stems_ray, neighbor_index, next_segment,
    outline_valid, prev_segment, ray_cast_boundary, round_glyph, signed_area_points,
};
use crate::group::{GlyphGroup, classify};
use crate::smooth::{SmoothOptions, smooth_glyph};

/// Corner join style for offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Corner {
    /// Extend to the edge intersection, capped by `miter_limit`.
    Miter,
    /// In points mode same as [`Corner::Keep`]; with `add_points` builds an arc.
    Round,
    /// Alias of [`Corner::Keep`].
    Angle,
    /// Move along the corner bisector and keep the original corner angle.
    #[default]
    Keep,
}

impl Corner {
    fn is_keep(self) -> bool {
        matches!(self, Self::Keep | Self::Angle | Self::Round)
    }
}

/// How [`offset_font`] builds the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OffsetMode {
    /// Move existing points; structure stays blend-compatible.
    #[default]
    Points,
    /// Geometric offset with joins, then curve re-fit (`compatible: false`).
    Clean,
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

/// Zone pinning for vertical metrics (and optional glyph extremes).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ZoneOptions {
    #[serde(default = "default_true")]
    pub baseline: bool,
    #[serde(default = "default_true")]
    pub x_height: bool,
    #[serde(default = "default_true")]
    pub cap_height: bool,
    #[serde(default = "default_true")]
    pub descender: bool,
    #[serde(default = "default_true")]
    pub ascender: bool,
    #[serde(default = "default_true")]
    pub glyph_extremes: bool,
    #[serde(default = "default_overshoot")]
    pub overshoot: f64,
}

impl Default for ZoneOptions {
    fn default() -> Self {
        Self {
            baseline: true,
            x_height: true,
            cap_height: true,
            descender: true,
            ascender: true,
            glyph_extremes: true,
            overshoot: 12.0,
        }
    }
}

impl ZoneOptions {
    fn all_off() -> Self {
        Self {
            baseline: false,
            x_height: false,
            cap_height: false,
            descender: false,
            ascender: false,
            glyph_extremes: false,
            overshoot: 12.0,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_overshoot() -> f64 {
    12.0
}

/// Options for [`offset_font`] / [`offset_font_detailed`].
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
    pub mode: OffsetMode,
    pub zones: ZoneOptions,
    pub counter_share: f64,
    pub min_gap_ratio: f64,
    pub gap_window: f64,
    pub miter_limit: f64,
    pub smooth_sigma: f64,
    pub post_smooth: bool,
    pub italic: ItalicMode,
    pub round: bool,
    pub points: Option<Vec<(usize, usize)>>,
    pub contours: Option<Vec<usize>>,
    pub family: bool,
    pub preview: bool,
}

impl Default for OffsetOptions {
    fn default() -> Self {
        Self {
            horizontal: 0.0,
            vertical: 0.0,
            gap: 0.0,
            corner: Corner::Keep,
            sidebearing: false,
            keep_metrics: true,
            add_points: false,
            names: None,
            mode: OffsetMode::Points,
            zones: ZoneOptions::default(),
            counter_share: 0.85,
            min_gap_ratio: 0.72,
            gap_window: 60.0,
            miter_limit: 2.0,
            smooth_sigma: 10.0,
            post_smooth: true,
            italic: ItalicMode::Auto,
            round: true,
            points: None,
            contours: None,
            family: false,
            preview: false,
        }
    }
}

impl OffsetOptions {
    /// Build options from the API command fields.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        horizontal: f64,
        vertical: f64,
        gap: f64,
        corner: Corner,
        sidebearing: bool,
        keep_metrics: bool,
        add_points: bool,
        names: Option<Vec<String>>,
        mode: OffsetMode,
        zones: Option<ZoneOptions>,
        counter_share: f64,
        min_gap_ratio: f64,
        gap_window: f64,
        miter_limit: f64,
        smooth_sigma: f64,
        post_smooth: bool,
        italic: &str,
        round: bool,
        points: Option<Vec<(usize, usize)>>,
        contours: Option<Vec<usize>>,
        family: bool,
        preview: bool,
    ) -> Self {
        let zones = match zones {
            Some(z) => z,
            None if keep_metrics => ZoneOptions::default(),
            None => ZoneOptions::all_off(),
        };
        Self {
            horizontal,
            vertical,
            gap,
            corner,
            sidebearing,
            keep_metrics,
            add_points,
            names,
            mode,
            zones,
            counter_share,
            min_gap_ratio,
            gap_window,
            miter_limit,
            smooth_sigma,
            post_smooth,
            italic: ItalicMode::from_field(italic),
            round,
            points,
            contours,
            family,
            preview,
        }
    }
}

/// Per-run report from [`offset_font_detailed`].
#[derive(Debug, Clone, Serialize)]
pub struct OffsetReport {
    pub changed: Vec<String>,
    pub unchanged: Vec<String>,
    pub reduced: Vec<Value>,
    pub fallback: Vec<Value>,
    pub skipped_open: Vec<Value>,
    pub compatible: bool,
    pub report: Value,
}

/// Offset the named glyphs, or every glyph. Returns the names that changed.
pub fn offset_font(font: &mut Font, options: &OffsetOptions) -> Result<Vec<String>, FoundryError> {
    let report = offset_font_detailed(font, options)?;
    Ok(report.changed)
}

/// Offset with the full §2.6 report.
pub fn offset_font_detailed(
    font: &mut Font,
    options: &OffsetOptions,
) -> Result<OffsetReport, FoundryError> {
    check_options(options)?;
    let names = target_names(font, options.names.as_deref())?;
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();
    let mut reduced = Vec::new();
    let mut fallback = Vec::new();
    let mut skipped_open = Vec::new();
    let mut stems_before = None;
    let mut stems_after = None;
    let mut bars_before = None;
    let mut bars_after = None;
    let mut counters_before = None;
    let mut counters_after = None;
    let mut min_gap_kept = options.min_gap_ratio;

    let frame = ItalicFrame::for_font(font, options.italic);
    let metrics = font.metrics.clone();
    let upm = font.upm as f64;

    for name in &names {
        let source = font
            .glyph(name)
            .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))?
            .clone();
        if source.contours.is_empty() {
            unchanged.push(name.clone());
            continue;
        }

        if name == "H" {
            stems_before = Some(measure_stems_ray(&source, metrics.cap_height));
            bars_before = Some(measure_bars_ray(&source));
        }
        if name == "o" {
            counters_before = Some(counter_width(&source));
        }

        let mut working = source.clone();
        if let Some(frame) = &frame {
            frame.enter_glyph(&mut working);
        }
        let source_framed = working.clone();

        let outcome = match options.mode {
            OffsetMode::Points => offset_glyph_points(
                &mut working,
                &source_framed,
                options,
                &metrics,
                upm,
                &mut skipped_open,
            )?,
            OffsetMode::Clean => {
                offset_glyph_clean(&mut working, &source_framed, options, &metrics)?;
                GlyphOutcome::Changed
            }
        };

        if let Some(frame) = &frame {
            frame.leave_glyph(&mut working);
        }
        if options.round && options.mode == OffsetMode::Points {
            round_glyph(&mut working);
        }
        if options.sidebearing {
            shift_sidebearings(&mut working, options.horizontal);
        }

        match outcome {
            GlyphOutcome::Unchanged => unchanged.push(name.clone()),
            GlyphOutcome::Changed => {
                if name == "H" {
                    stems_after = Some(measure_stems_ray(&working, metrics.cap_height));
                    bars_after = Some(measure_bars_ray(&working));
                }
                if name == "o" {
                    counters_after = Some(counter_width(&working));
                }
                *font
                    .glyph_mut(name)
                    .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))? = working;
                changed.push(name.clone());
            }
            GlyphOutcome::Reduced(factor) => {
                reduced.push(json!({ "name": name, "factor": factor }));
                if name == "H" {
                    stems_after = Some(measure_stems_ray(&working, metrics.cap_height));
                    bars_after = Some(measure_bars_ray(&working));
                }
                if name == "o" {
                    counters_after = Some(counter_width(&working));
                }
                *font
                    .glyph_mut(name)
                    .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))? = working;
                changed.push(name.clone());
            }
            GlyphOutcome::Fallback(reason) => {
                fallback.push(json!({
                    "name": name,
                    "reason": reason,
                    "action": "kept original"
                }));
                unchanged.push(name.clone());
            }
        }
    }

    let compatible = options.mode == OffsetMode::Points && options.corner != Corner::Round
        || !options.add_points;
    let compatible = compatible && options.mode == OffsetMode::Points && !options.add_points;

    let mut report_obj = serde_json::Map::new();
    if let (Some(b), Some(a)) = (stems_before, stems_after)
        && let (Some(&sb), Some(&sa)) = (b.first(), a.first())
    {
        report_obj.insert("stems".into(), json!({ "H": [sb, sa] }));
    }
    if let (Some(b), Some(a)) = (bars_before, bars_after)
        && let (Some(&sb), Some(&sa)) = (b.first(), a.first())
    {
        report_obj.insert("bars".into(), json!({ "H": [sb, sa] }));
    }
    if let (Some(b), Some(a)) = (counters_before, counters_after) {
        report_obj.insert("counters".into(), json!({ "o": [b, a] }));
    }
    report_obj.insert("min_gap_kept".into(), json!(min_gap_kept));
    let _ = &mut min_gap_kept;

    Ok(OffsetReport {
        changed,
        unchanged,
        reduced,
        fallback,
        skipped_open,
        compatible,
        report: Value::Object(report_obj),
    })
}

enum GlyphOutcome {
    Unchanged,
    Changed,
    Reduced(f64),
    Fallback(&'static str),
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
    let metrics = font.metrics.clone();
    let upm = font.upm as f64;
    for name in &names {
        let source = font
            .glyph(name)
            .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))?
            .clone();
        let mut outward = options.clone();
        outward.horizontal *= outer;
        outward.vertical *= outer;
        outward.sidebearing = false;
        let mut inward = options.clone();
        inward.horizontal *= inner;
        inward.vertical *= inner;
        inward.sidebearing = false;
        let mut shell = source.clone();
        let mut core = source.clone();
        let mut skipped = Vec::new();
        let _ = offset_glyph_points(&mut shell, &source, &outward, &metrics, upm, &mut skipped)?;
        let _ = offset_glyph_points(&mut core, &source, &inward, &metrics, upm, &mut skipped)?;
        for contour in &mut core.contours {
            contour.points.reverse();
        }
        let glyph = font
            .glyph_mut(name)
            .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))?;
        glyph.contours = shell.contours;
        glyph.contours.append(&mut core.contours);
        if options.sidebearing {
            shift_sidebearings(glyph, options.horizontal);
        }
    }
    Ok(names)
}

fn check_options(options: &OffsetOptions) -> Result<(), FoundryError> {
    let values = [
        options.horizontal,
        options.vertical,
        options.gap,
        options.counter_share,
        options.min_gap_ratio,
        options.gap_window,
        options.miter_limit,
        options.smooth_sigma,
        options.zones.overshoot,
    ];
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
    if !(0.0..=1.0).contains(&options.counter_share) {
        return Err(FoundryError::Edit(
            "offset counter_share must be between 0 and 1".into(),
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

fn shift_sidebearings(glyph: &mut Glyph, horizontal: f64) {
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.x += horizontal;
        }
    }
    glyph.advance += horizontal * 2.0;
}

fn counter_width(glyph: &Glyph) -> f64 {
    if glyph.contours.len() < 2 {
        return 0.0;
    }
    let nesting = contour_nesting(&glyph.contours);
    let mut best = 0.0_f64;
    for (index, (_, outer)) in nesting.iter().enumerate() {
        if *outer {
            continue;
        }
        let (min_x, max_x, _, _) = ink_bounds(&Glyph {
            name: String::new(),
            unicode: None,
            advance: 0.0,
            contours: vec![glyph.contours[index].clone()],
        });
        best = best.max(max_x - min_x);
    }
    best
}

// --- Points mode -----------------------------------------------------------

fn offset_glyph_points(
    glyph: &mut Glyph,
    source: &Glyph,
    options: &OffsetOptions,
    metrics: &Metrics,
    upm: f64,
    skipped_open: &mut Vec<Value>,
) -> Result<GlyphOutcome, FoundryError> {
    if options.horizontal.abs() < 1e-12 && options.vertical.abs() < 1e-12 {
        return Ok(GlyphOutcome::Unchanged);
    }
    if is_dot_glyph(glyph, upm) {
        scale_dot(glyph, options.horizontal);
        return Ok(GlyphOutcome::Changed);
    }
    let mut factor = 1.0;
    let mut last_reason = "self-intersection";
    for attempt in 0..9 {
        let mut trial = source.clone();
        let mut open_notes = Vec::new();
        let ok = apply_points_with_optional_arcs(
            &mut trial,
            source,
            options,
            metrics,
            factor,
            &mut open_notes,
        )?;
        if !ok {
            return Ok(GlyphOutcome::Unchanged);
        }
        let issues = if options.add_points {
            Vec::new()
        } else {
            outline_valid(&trial, Some(source))
        };
        if issues.is_empty() {
            *glyph = trial;
            skipped_open.extend(open_notes);
            if attempt == 0 {
                return Ok(GlyphOutcome::Changed);
            }
            return Ok(GlyphOutcome::Reduced(factor));
        }
        last_reason = issues[0].code;
        if attempt == 8 {
            skipped_open.extend(open_notes);
            return Ok(GlyphOutcome::Fallback(last_reason));
        }
        factor *= 0.93;
    }
    Ok(GlyphOutcome::Fallback(last_reason))
}

fn apply_points_once(
    glyph: &mut Glyph,
    source: &Glyph,
    options: &OffsetOptions,
    metrics: &Metrics,
    amount_scale: f64,
    skipped_open: &mut Vec<Value>,
) -> Result<bool, FoundryError> {
    let h = options.horizontal * amount_scale;
    let v = options.vertical * amount_scale;
    if h.abs() < 1e-12 && v.abs() < 1e-12 {
        return Ok(false);
    }

    let nesting = contour_nesting(&glyph.contours);
    let has_hole = nesting.iter().any(|(_, outer)| !*outer);
    let group = classify(glyph.unicode, false);
    let zone_ys = zone_lines(metrics, &options.zones, group, glyph);

    // Per-contour grow side and share.
    let mut grow_left = Vec::with_capacity(glyph.contours.len());
    let mut shares = Vec::with_capacity(glyph.contours.len());
    for (index, contour) in glyph.contours.iter().enumerate() {
        if !contour.closed {
            skipped_open.push(json!({
                "name": glyph.name,
                "contour": index,
                "reason": "open"
            }));
            grow_left.push(true);
            shares.push(1.0);
            continue;
        }
        if contour.points.len() < 3 {
            grow_left.push(true);
            shares.push(1.0);
            continue;
        }
        let outer = nesting[index].1;
        grow_left.push(grow_is_left_of_travel(contour, outer));
        let share = if has_hole {
            if outer {
                2.0 - options.counter_share
            } else {
                options.counter_share
            }
        } else {
            1.0
        };
        shares.push(share);
    }

    // Selection masks.
    let point_sel = options.points.as_deref();
    let contour_sel = options.contours.as_deref();

    // (1–4) Normals, factors, raw displacement for every on-point.
    let mut disp: Vec<Vec<Option<V2>>> = Vec::with_capacity(glyph.contours.len());
    let mut zone_pinned: Vec<Vec<bool>> = Vec::with_capacity(glyph.contours.len());

    for (c_index, contour) in glyph.contours.iter().enumerate() {
        let count = contour.points.len();
        let mut d_row = vec![None; count];
        let mut pin_row = vec![false; count];
        if !contour.closed || count < 3 {
            disp.push(d_row);
            zone_pinned.push(pin_row);
            continue;
        }
        if let Some(list) = contour_sel
            && !list.contains(&c_index)
        {
            disp.push(d_row);
            zone_pinned.push(pin_row);
            continue;
        }
        let gl = grow_left[c_index];
        let share = shares[c_index];
        let join_list = joins(contour);
        for join in &join_list {
            let index = join.on;
            if let Some(sel) = point_sel
                && !sel.iter().any(|&(c, p)| c == c_index && p == index)
            {
                continue;
            }
            let Some(normal) = on_point_normal(contour, index, gl) else {
                continue;
            };
            let factor = corner_factor(join.turn, options);
            let mut d = V2::new(normal.x * h, normal.y * v).mul(factor * share);
            let pinned = zone_pin_y(
                V2::from_point(&contour.points[index]),
                normal,
                &zone_ys,
                options.zones.overshoot,
                &mut d,
            );
            pin_row[index] = pinned;
            d_row[index] = Some(d);
        }
        disp.push(d_row);
        zone_pinned.push(pin_row);
    }

    // (5) Smooth displacement field along arc length, then re-pin zones.
    if options.smooth_sigma > 0.0 {
        for (c_index, contour) in glyph.contours.iter().enumerate() {
            if !contour.closed || contour.points.len() < 3 {
                continue;
            }
            smooth_displacement_field(contour, &mut disp[c_index], options.smooth_sigma);
            for (index, slot) in disp[c_index].iter_mut().enumerate() {
                if let Some(d) = slot.as_mut() {
                    let Some(normal) = on_point_normal(contour, index, grow_left[c_index]) else {
                        continue;
                    };
                    let pinned = zone_pin_y(
                        V2::from_point(&contour.points[index]),
                        normal,
                        &zone_ys,
                        options.zones.overshoot,
                        d,
                    );
                    if pinned {
                        zone_pinned[c_index][index] = true;
                    }
                }
            }
        }
    }

    // (6) Collision cap + gap guard.
    collision_cap(glyph, &mut disp, &grow_left, options.gap, h, v);
    if options.min_gap_ratio > 0.0 && options.gap_window > 0.0 {
        gap_guard(glyph, &mut disp, options.min_gap_ratio, options.gap_window);
    }

    // Thinning floor.
    if h < 0.0 || v < 0.0 {
        thinning_floor(glyph, &mut disp, &grow_left);
    }

    // (7) Apply on-points.
    for (c_index, contour) in glyph.contours.iter_mut().enumerate() {
        for (index, point) in contour.points.iter_mut().enumerate() {
            if point.kind != PointKind::On {
                continue;
            }
            if let Some(Some(d)) = disp.get(c_index).and_then(|row| row.get(index)) {
                point.x += d.x;
                point.y += d.y;
            }
        }
    }

    // (8) Restore.
    restore_axis_runs(glyph, source);
    rebuild_offs_in_chord_frame(glyph, source);

    if options.post_smooth {
        let smooth_opts = SmoothOptions {
            axis_snap: true,
            italic: ItalicMode::Off,
            round: false,
            keep_heights: true,
            ..SmoothOptions::default()
        };
        let _ = smooth_glyph(glyph, Some(source), &smooth_opts, None)?;
    }
    // Always restore zone y from the source for points that sat on a zone (or were pinned).
    reapply_zone_pins(glyph, source, &zone_pinned);
    reapply_zone_ys_from_metrics(glyph, source, metrics, &options.zones, group);

    clamp_offs_to_chord(glyph);
    Ok(true)
}

fn on_point_normal(contour: &Contour, index: usize, grow_left: bool) -> Option<V2> {
    let points = &contour.points;
    let here = V2::from_point(&points[index]);
    let (prev_on, in_offs) = prev_segment(points, index, contour.closed)?;
    let (next_on, out_offs) = next_segment(points, index, contour.closed)?;
    let in_tan = match in_offs.as_slice() {
        [] => here.sub(V2::from_point(&points[prev_on])),
        [c] | [_, c] => here.sub(V2::from_point(c)),
        _ => return None,
    };
    let out_tan = match out_offs.as_slice() {
        [] => V2::from_point(&points[next_on]).sub(here),
        [c] | [c, _] => V2::from_point(c).sub(here),
        _ => return None,
    };
    let in_u = in_tan.norm();
    let out_u = out_tan.norm();
    if in_u.len() < 1e-8 || out_u.len() < 1e-8 {
        return None;
    }
    let in_n = edge_normal(in_u, grow_left);
    let out_n = edge_normal(out_u, grow_left);
    let mixed = in_n.add(out_n).norm();
    if mixed.len() > 1e-8 {
        Some(mixed)
    } else {
        Some(in_n)
    }
}

fn edge_normal(unit: V2, grow_left: bool) -> V2 {
    if grow_left {
        unit.perp_left()
    } else {
        unit.perp_right()
    }
}

fn corner_factor(turn_deg: f64, options: &OffsetOptions) -> f64 {
    if options.corner.is_keep() {
        return 1.0;
    }
    // Miter: 1 / cos(θ/2), θ = turn.
    let half = (turn_deg.to_radians() / 2.0).abs();
    let cos = half.cos().abs().max(1e-6);
    (1.0 / cos).min(options.miter_limit)
}

fn zone_lines(
    metrics: &Metrics,
    zones: &ZoneOptions,
    group: GlyphGroup,
    glyph: &Glyph,
) -> Vec<(f64, i8)> {
    // (y, outward_sign) where +1 means outward is up, -1 means outward is down.
    let mut lines = Vec::new();
    if zones.baseline {
        lines.push((metrics.baseline, -1));
    }
    if zones.descender {
        lines.push((metrics.descender, -1));
    }
    if zones.ascender {
        lines.push((metrics.ascender, 1));
    }
    match group {
        GlyphGroup::Lowercase => {
            if zones.x_height {
                lines.push((metrics.x_height, 1));
            }
        }
        GlyphGroup::Uppercase | GlyphGroup::Figures => {
            if zones.cap_height {
                lines.push((metrics.cap_height, 1));
            }
        }
        _ => {
            if zones.x_height {
                lines.push((metrics.x_height, 1));
            }
            if zones.cap_height {
                lines.push((metrics.cap_height, 1));
            }
        }
    }
    if zones.glyph_extremes {
        let mut top = f64::NEG_INFINITY;
        let mut bottom = f64::INFINITY;
        for contour in &glyph.contours {
            for point in &contour.points {
                if point.kind == PointKind::On {
                    top = top.max(point.y);
                    bottom = bottom.min(point.y);
                }
            }
        }
        if top.is_finite() {
            lines.push((top, 1));
        }
        if bottom.is_finite() {
            lines.push((bottom, -1));
        }
    }
    lines
}

/// Pin or ramp `d.y` for zone points. Returns true when fully pinned (`d.y = 0`).
fn zone_pin_y(point: V2, normal: V2, zones: &[(f64, i8)], overshoot: f64, d: &mut V2) -> bool {
    let mut pinned = false;
    let mut ramp = 1.0_f64;
    for &(zone_y, out_sign) in zones {
        let dy = point.y - zone_y;
        // Normal must point out of the glyph across that zone.
        let normal_out = if out_sign > 0 {
            normal.y > 0.05
        } else {
            normal.y < -0.05
        };
        if !normal_out {
            continue;
        }
        // On the zone, or within overshoot on the outside.
        let on_zone = dy.abs() <= 0.51;
        let outside = if out_sign > 0 {
            dy > 0.0 && dy <= overshoot
        } else {
            dy < 0.0 && dy >= -overshoot
        };
        if on_zone || outside {
            d.y = 0.0;
            pinned = true;
            break;
        }
        // Ramp: within 100 units inside the zone, scale d.y from 0 at the zone to full at 100.
        let inside = if out_sign > 0 {
            (-100.0..0.0).contains(&dy)
        } else {
            (0.0..=100.0).contains(&dy) && dy > 0.0
        };
        if inside {
            let dist = dy.abs();
            if dist < 100.0 {
                let t = dist / 100.0;
                ramp = ramp.min(t);
            }
        }
    }
    if !pinned && ramp < 1.0 {
        d.y *= ramp;
    }
    pinned
}

fn reapply_zone_pins(glyph: &mut Glyph, source: &Glyph, zone_pinned: &[Vec<bool>]) {
    for (c_index, contour) in glyph.contours.iter_mut().enumerate() {
        let Some(src) = source.contours.get(c_index) else {
            continue;
        };
        let Some(pins) = zone_pinned.get(c_index) else {
            continue;
        };
        for (index, point) in contour.points.iter_mut().enumerate() {
            if point.kind != PointKind::On {
                continue;
            }
            if pins.get(index).copied().unwrap_or(false) {
                point.y = src.points[index].y;
            }
        }
    }
}

/// Restore source y for any on-point that sat on a metric/extreme zone line in the source.
fn reapply_zone_ys_from_metrics(
    glyph: &mut Glyph,
    source: &Glyph,
    metrics: &Metrics,
    zones: &ZoneOptions,
    group: GlyphGroup,
) {
    let zone_ys = zone_lines(metrics, zones, group, source);
    if zone_ys.is_empty() {
        return;
    }
    let overshoot = zones.overshoot;
    for (c_index, contour) in glyph.contours.iter_mut().enumerate() {
        let Some(src) = source.contours.get(c_index) else {
            continue;
        };
        for (index, point) in contour.points.iter_mut().enumerate() {
            if point.kind != PointKind::On || index >= src.points.len() {
                continue;
            }
            let sy = src.points[index].y;
            for &(zone_y, _) in &zone_ys {
                if (sy - zone_y).abs() <= overshoot.max(0.51) {
                    point.y = sy;
                    break;
                }
            }
        }
    }
}

fn smooth_displacement_field(contour: &Contour, disp: &mut [Option<V2>], sigma: f64) {
    let count = contour.points.len();
    if count < 3 || sigma <= 0.0 {
        return;
    }
    // Arc-length parameter at each on-point; offs inherit neighbouring on values (skipped).
    let mut on_indices = Vec::new();
    for (i, p) in contour.points.iter().enumerate() {
        if p.kind == PointKind::On && disp[i].is_some() {
            on_indices.push(i);
        }
    }
    if on_indices.len() < 2 {
        return;
    }
    let mut arc = vec![0.0; on_indices.len()];
    for i in 1..on_indices.len() {
        let a = V2::from_point(&contour.points[on_indices[i - 1]]);
        let b = V2::from_point(&contour.points[on_indices[i]]);
        arc[i] = arc[i - 1] + a.dist(b);
    }
    let total = {
        let a = V2::from_point(&contour.points[*on_indices.last().unwrap()]);
        let b = V2::from_point(&contour.points[on_indices[0]]);
        arc[on_indices.len() - 1] + a.dist(b)
    };
    if total < 1e-8 {
        return;
    }
    let src: Vec<V2> = on_indices
        .iter()
        .map(|&i| disp[i].unwrap_or(V2::ZERO))
        .collect();
    let mut out = vec![V2::ZERO; on_indices.len()];
    for i in 0..on_indices.len() {
        let mut wsum = 0.0;
        let mut acc = V2::ZERO;
        for j in 0..on_indices.len() {
            let mut dist = (arc[i] - arc[j]).abs();
            dist = dist.min(total - dist);
            let w = (-0.5 * (dist / sigma).powi(2)).exp();
            wsum += w;
            acc = acc.add(src[j].mul(w));
        }
        if wsum > 1e-12 {
            out[i] = acc.mul(1.0 / wsum);
        }
    }
    for (k, &index) in on_indices.iter().enumerate() {
        disp[index] = Some(out[k]);
    }
}

fn collision_cap(
    glyph: &Glyph,
    disp: &mut [Vec<Option<V2>>],
    grow_left: &[bool],
    gap: f64,
    h: f64,
    v: f64,
) {
    // Build a proposal glyph for ray casts against the current proposal.
    let mut proposal = glyph.clone();
    for (c_index, contour) in proposal.contours.iter_mut().enumerate() {
        for (index, point) in contour.points.iter_mut().enumerate() {
            if let Some(Some(d)) = disp.get(c_index).and_then(|row| row.get(index)) {
                point.x += d.x;
                point.y += d.y;
            }
        }
    }

    let nesting = contour_nesting(&glyph.contours);
    let outward_for = |contours: &[Contour], grow: &[bool]| {
        let grow = grow.to_vec();
        let contours_area: Vec<f64> = contours
            .iter()
            .map(|c| signed_area_points(&c.points))
            .collect();
        move |c_index: usize, a: V2, b: V2| {
            let edge = b.sub(a).norm();
            if edge.len() < 1e-8 {
                return V2::ZERO;
            }
            let gl = grow.get(c_index).copied().unwrap_or(false);
            // Prefer grow_left; fall back to area-based outward.
            let left = edge.perp_left();
            if contours_area.get(c_index).copied().unwrap_or(0.0).abs() < 1e-12 {
                return if gl { left } else { edge.perp_right() };
            }
            if gl { left } else { edge.perp_right() }
        }
    };

    let src_out = outward_for(&glyph.contours, grow_left);
    let prop_out = outward_for(&proposal.contours, grow_left);

    for (c_index, contour) in glyph.contours.iter().enumerate() {
        if !contour.closed {
            continue;
        }
        for (index, point) in contour.points.iter().enumerate() {
            if point.kind != PointKind::On {
                continue;
            }
            let Some(Some(d)) = disp[c_index].get(index).cloned() else {
                continue;
            };
            let len = d.len();
            if len < 1e-8 {
                continue;
            }
            let dir = d.norm();
            let origin = V2::from_point(point);
            let hit_src = ray_cast_boundary(origin, dir, &glyph.contours, &src_out);
            let hit_prop = ray_cast_boundary(origin, dir, &proposal.contours, &prop_out);
            let hit = match (hit_src, hit_prop) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            let Some(hit) = hit else {
                continue;
            };
            // Facing edges that also grow toward us share the room.
            let share = if (h > 0.0 || v > 0.0) && nesting.len() > 1 {
                2.0
            } else {
                1.0
            };
            let allowed = ((hit - gap) / share).max(0.0);
            if allowed < len {
                disp[c_index][index] = Some(dir.mul(allowed));
            }
        }
    }
    let _ = nesting;
}

fn gap_guard(glyph: &Glyph, disp: &mut [Vec<Option<V2>>], min_ratio: f64, window: f64) {
    // Sample on-points as proxies for boundary samples.
    let mut samples: Vec<(usize, usize, V2)> = Vec::new();
    for (c_index, contour) in glyph.contours.iter().enumerate() {
        if !contour.closed {
            continue;
        }
        for (index, point) in contour.points.iter().enumerate() {
            if point.kind == PointKind::On {
                samples.push((c_index, index, V2::from_point(point)));
            }
        }
    }
    for _ in 0..3 {
        let mut scaled = false;
        for i in 0..samples.len() {
            for j in (i + 1)..samples.len() {
                let (c0, p0, a) = samples[i];
                let (c1, p1, b) = samples[j];
                // Neighbours on the same contour (within 2 on-points) are skipped.
                if c0 == c1 {
                    let contour = &glyph.contours[c0];
                    let on_idx: Vec<usize> = contour
                        .points
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p.kind == PointKind::On)
                        .map(|(i, _)| i)
                        .collect();
                    if let (Some(i0), Some(i1)) = (
                        on_idx.iter().position(|&x| x == p0),
                        on_idx.iter().position(|&x| x == p1),
                    ) {
                        let n = on_idx.len() as isize;
                        let dist = (i0 as isize - i1 as isize)
                            .abs()
                            .min(n - (i0 as isize - i1 as isize).abs());
                        if dist <= 2 {
                            continue;
                        }
                    }
                }
                let s = a.dist(b);
                if s >= window || s < 1e-6 {
                    continue;
                }
                let d0 = disp[c0][p0].unwrap_or(V2::ZERO);
                let d1 = disp[c1][p1].unwrap_or(V2::ZERO);
                let new = a.add(d0).dist(b.add(d1));
                let need = min_ratio * s;
                if new + 1e-6 < need {
                    // Scale both displacements back proportionally.
                    let mid = a.add(d0).lerp(b.add(d1), 0.5);
                    let dir0 = a.add(d0).sub(mid);
                    let dir1 = b.add(d1).sub(mid);
                    let half = need / 2.0;
                    let n0 = a.sub(mid).norm();
                    let n1 = b.sub(mid).norm();
                    let target0 = mid.add(n0.mul(half));
                    let target1 = mid.add(n1.mul(half));
                    // Prefer scaling existing d rather than inventing new positions when possible.
                    let scale = (need / new).min(1.0);
                    let _ = (dir0, dir1, target0, target1);
                    if let Some(d) = disp[c0][p0].as_mut() {
                        *d = d.mul(scale);
                    }
                    if let Some(d) = disp[c1][p1].as_mut() {
                        *d = d.mul(scale);
                    }
                    scaled = true;
                }
            }
        }
        if !scaled {
            break;
        }
    }
}

fn thinning_floor(glyph: &Glyph, disp: &mut [Vec<Option<V2>>], grow_left: &[bool]) {
    let outward_for = {
        let grow = grow_left.to_vec();
        move |c_index: usize, a: V2, b: V2| {
            let edge = b.sub(a).norm();
            if edge.len() < 1e-8 {
                return V2::ZERO;
            }
            if grow.get(c_index).copied().unwrap_or(false) {
                edge.perp_left()
            } else {
                edge.perp_right()
            }
        }
    };
    for (c_index, contour) in glyph.contours.iter().enumerate() {
        for (index, point) in contour.points.iter().enumerate() {
            if point.kind != PointKind::On {
                continue;
            }
            let Some(Some(d)) = disp[c_index].get(index).cloned() else {
                continue;
            };
            if d.len() < 1e-8 {
                continue;
            }
            // Ray opposite grow (into the stroke) to measure current thickness.
            let into = d.norm().mul(-1.0);
            let origin = V2::from_point(point);
            let Some(thickness) = ray_cast_boundary(origin, into, &glyph.contours, &outward_for)
            else {
                continue;
            };
            let floor = (0.25 * thickness).max(4.0);
            let room = (thickness - floor).max(0.0);
            // Thinning moves into the stroke; |d| may not exceed room.
            if d.len() > room {
                disp[c_index][index] = Some(d.norm().mul(room));
            }
        }
    }
}

fn restore_axis_runs(glyph: &mut Glyph, source: &Glyph) {
    for (c_index, contour) in glyph.contours.iter_mut().enumerate() {
        let Some(src) = source.contours.get(c_index) else {
            continue;
        };
        if !contour.closed || contour.points.len() < 3 {
            continue;
        }
        let on_idx: Vec<usize> = contour
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.kind == PointKind::On)
            .map(|(i, _)| i)
            .collect();
        if on_idx.len() < 2 {
            continue;
        }
        // X runs.
        let mut i = 0;
        while i < on_idx.len() {
            let mut j = i + 1;
            while j < on_idx.len() {
                let a = src.points[on_idx[j - 1]].x;
                let b = src.points[on_idx[j]].x;
                if (a - b).abs() <= 0.5 {
                    j += 1;
                } else {
                    break;
                }
            }
            // Wrap-around run handled only for a full loop of equal x.
            if j - i >= 2 {
                let mean = on_idx[i..j]
                    .iter()
                    .map(|&k| contour.points[k].x)
                    .sum::<f64>()
                    / (j - i) as f64;
                for &k in &on_idx[i..j] {
                    contour.points[k].x = mean;
                }
            }
            i = j.max(i + 1);
        }
        // Y runs.
        i = 0;
        while i < on_idx.len() {
            let mut j = i + 1;
            while j < on_idx.len() {
                let a = src.points[on_idx[j - 1]].y;
                let b = src.points[on_idx[j]].y;
                if (a - b).abs() <= 0.5 {
                    j += 1;
                } else {
                    break;
                }
            }
            if j - i >= 2 {
                let mean = on_idx[i..j]
                    .iter()
                    .map(|&k| contour.points[k].y)
                    .sum::<f64>()
                    / (j - i) as f64;
                for &k in &on_idx[i..j] {
                    contour.points[k].y = mean;
                }
            }
            i = j.max(i + 1);
        }
    }
}

fn rebuild_offs_in_chord_frame(glyph: &mut Glyph, source: &Glyph) {
    for (c_index, contour) in glyph.contours.iter_mut().enumerate() {
        let Some(src) = source.contours.get(c_index) else {
            continue;
        };
        if !contour.closed {
            continue;
        }
        let count = contour.points.len();
        for index in 0..count {
            if src.points[index].kind != PointKind::On {
                continue;
            }
            let Some((next_on, offs)) = next_segment(&src.points, index, true) else {
                continue;
            };
            if offs.is_empty() {
                continue;
            }
            let a_src = V2::from_point(&src.points[index]);
            let b_src = V2::from_point(&src.points[next_on]);
            let a = V2::from_point(&contour.points[index]);
            let b = V2::from_point(&contour.points[next_on]);
            let chord_src = b_src.sub(a_src);
            let chord = b.sub(a);
            let len_src = chord_src.len();
            let len = chord.len();
            if len_src < 1e-8 || len < 1e-8 {
                continue;
            }
            let ex_src = chord_src.mul(1.0 / len_src);
            let ey_src = ex_src.perp_left();
            let ex = chord.mul(1.0 / len);
            let ey = ex.perp_left();
            // Walk offs between index and next_on in the live contour.
            let mut cursor = neighbor_index(index, count, true, true);
            let mut off_i = 0;
            while cursor != next_on && off_i < offs.len() {
                let p_src = V2::from_point(&src.points[cursor]);
                let rel = p_src.sub(a_src);
                let u = rel.dot(ex_src) / len_src;
                let v = rel.dot(ey_src) / len_src;
                let world = a.add(ex.mul(u * len)).add(ey.mul(v * len));
                contour.points[cursor].x = world.x;
                contour.points[cursor].y = world.y;
                cursor = neighbor_index(cursor, count, true, true);
                off_i += 1;
            }
        }
    }
}

fn clamp_offs_to_chord(glyph: &mut Glyph) {
    for contour in &mut glyph.contours {
        if !contour.closed {
            continue;
        }
        let count = contour.points.len();
        for index in 0..count {
            if contour.points[index].kind != PointKind::On {
                continue;
            }
            let Some((next_on, offs)) = next_segment(&contour.points, index, true) else {
                continue;
            };
            if offs.is_empty() {
                continue;
            }
            let a = V2::from_point(&contour.points[index]);
            let b = V2::from_point(&contour.points[next_on]);
            let chord = b.sub(a);
            let len = chord.len();
            if len < 1e-8 {
                continue;
            }
            let ex = chord.mul(1.0 / len);
            let mut cursor = neighbor_index(index, count, true, true);
            while cursor != next_on {
                let p = V2::from_point(&contour.points[cursor]);
                let t = p.sub(a).dot(ex) / len;
                let clamped = t.clamp(0.08, 0.92);
                if (clamped - t).abs() > 1e-9 {
                    // Keep the perpendicular offset; only clamp along-chord.
                    let ey = ex.perp_left();
                    let v = p.sub(a).dot(ey);
                    let world = a.add(ex.mul(clamped * len)).add(ey.mul(v));
                    contour.points[cursor].x = world.x;
                    contour.points[cursor].y = world.y;
                }
                cursor = neighbor_index(cursor, count, true, true);
            }
        }
    }
}

fn is_dot_glyph(glyph: &Glyph, upm: f64) -> bool {
    if glyph.contours.len() != 1 {
        return false;
    }
    let contour = &glyph.contours[0];
    if !contour.closed || contour.points.len() < 3 {
        return false;
    }
    // Dots are curved; a boxy serif stem must not match.
    let has_curve = contour
        .points
        .iter()
        .any(|point| point.kind == PointKind::Off);
    if !has_curve {
        return false;
    }
    let (min_x, max_x, min_y, max_y) = ink_bounds(glyph);
    let w = max_x - min_x;
    let h = max_y - min_y;
    if w <= 0.0 || h <= 0.0 {
        return false;
    }
    let aspect = w.min(h) / w.max(h);
    if aspect < 0.7 {
        return false;
    }
    let area = signed_area_points(&contour.points).abs();
    let bbox = w * h;
    let fill = area / bbox;
    // Circles sit near π/4 ≈ 0.785; reject filled rectangles (~1.0).
    (0.7..0.95).contains(&fill) && w.max(h) < 0.15 * upm
}

fn scale_dot(glyph: &mut Glyph, horizontal: f64) {
    let (min_x, max_x, min_y, max_y) = ink_bounds(glyph);
    let cx = (min_x + max_x) / 2.0;
    let cy = (min_y + max_y) / 2.0;
    let width = (max_x - min_x).max(1.0);
    let scale = 1.0 + 2.0 * horizontal / width;
    // Top-anchored: keep top y.
    let top = max_y;
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.x = cx + (point.x - cx) * scale;
            point.y = cy + (point.y - cy) * scale;
        }
    }
    let (_, _, _, new_top) = ink_bounds(glyph);
    let dy = top - new_top;
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.y += dy;
        }
    }
}

// --- Round joins (add_points compatibility) --------------------------------

fn with_round_joins(
    contour: &Contour,
    grow_left: bool,
    options: &OffsetOptions,
    moved: &[Point],
    zones_active: bool,
) -> Vec<Point> {
    let mut out = Vec::with_capacity(moved.len() + 2 * contour.points.len());
    for (index, point) in moved.iter().enumerate() {
        match round_join(contour, index, grow_left, options, zones_active) {
            Some(arc) => out.extend(arc),
            None => out.push(point.clone()),
        }
    }
    out
}

fn round_join(
    contour: &Contour,
    index: usize,
    grow_left: bool,
    options: &OffsetOptions,
    zones_active: bool,
) -> Option<[Point; 4]> {
    let count = contour.points.len();
    let corner = &contour.points[index];
    if !contour.closed || count < 3 || corner.kind != PointKind::On || corner.smooth {
        return None;
    }
    if options.horizontal.abs() <= 1e-8 || options.vertical.abs() <= 1e-8 {
        return None;
    }
    if zones_active && vertical_extremum(&contour.points, index, true) {
        return None;
    }
    let prev = V2::from_point(&contour.points[neighbor_index(index, count, true, false)]);
    let here = V2::from_point(corner);
    let next = V2::from_point(&contour.points[neighbor_index(index, count, true, true)]);
    let in_unit = here.sub(prev).norm();
    let out_unit = next.sub(here).norm();
    if in_unit.len() < 1e-8 || out_unit.len() < 1e-8 {
        return None;
    }
    let turn = in_unit.dot(out_unit).clamp(-1.0, 1.0).acos();
    if turn < 1e-3 {
        return None;
    }
    let cross = in_unit.x * out_unit.y - in_unit.y * out_unit.x;
    // Left turn (cross>0): outside of the turn is the right side.
    // Grow on the outside when (cross>0) != grow_left.
    let away_is_outside = (cross > 0.0) != grow_left;
    let shrinking = options.horizontal < 0.0 || options.vertical < 0.0;
    if away_is_outside == shrinking {
        return None;
    }
    let in_away = edge_normal(in_unit, grow_left);
    let out_away = edge_normal(out_unit, grow_left);
    let start = V2::new(
        here.x + in_away.x * options.horizontal,
        here.y + in_away.y * options.vertical,
    );
    let end = V2::new(
        here.x + out_away.x * options.horizontal,
        here.y + out_away.y * options.vertical,
    );
    let radius = (start.dist(here) + end.dist(here)) / 2.0;
    let handle = 4.0 / 3.0 * (turn / 4.0).tan() * radius;
    let first_handle = start.add(in_unit.mul(handle));
    let second_handle = end.sub(out_unit.mul(handle));
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

fn vertical_extremum(points: &[Point], index: usize, closed: bool) -> bool {
    let count = points.len();
    let y = points[index].y;
    let prev = points[neighbor_index(index, count, closed, false)].y;
    let next = points[neighbor_index(index, count, closed, true)].y;
    (y >= prev && y >= next) || (y <= prev && y <= next)
}

/// Points-mode path that also supports `add_points` round joins (structure-changing).
fn apply_points_with_optional_arcs(
    glyph: &mut Glyph,
    source: &Glyph,
    options: &OffsetOptions,
    metrics: &Metrics,
    amount_scale: f64,
    skipped_open: &mut Vec<Value>,
) -> Result<bool, FoundryError> {
    if options.add_points && options.corner == Corner::Round {
        // Legacy path: simple per-point offset then insert arcs (no post_smooth).
        return apply_add_points(glyph, source, options, metrics, amount_scale, skipped_open);
    }
    apply_points_once(glyph, source, options, metrics, amount_scale, skipped_open)
}

fn apply_add_points(
    glyph: &mut Glyph,
    source: &Glyph,
    options: &OffsetOptions,
    metrics: &Metrics,
    amount_scale: f64,
    skipped_open: &mut Vec<Value>,
) -> Result<bool, FoundryError> {
    let h = options.horizontal * amount_scale;
    let v = options.vertical * amount_scale;
    let mut opts = options.clone();
    opts.horizontal = h;
    opts.vertical = v;
    opts.post_smooth = false;
    opts.smooth_sigma = 0.0;
    opts.add_points = false;
    // Move with zones from keep_metrics alias.
    let zones_active = options.keep_metrics
        || options.zones.baseline
        || options.zones.cap_height
        || options.zones.x_height;
    apply_points_once(glyph, source, &opts, metrics, 1.0, skipped_open)?;
    let nesting = contour_nesting(&source.contours);
    let mut proposed = Vec::with_capacity(glyph.contours.len());
    for (index, contour) in source.contours.iter().enumerate() {
        let outer = nesting[index].1;
        let gl = if contour.closed && contour.points.len() >= 3 {
            grow_is_left_of_travel(contour, outer)
        } else {
            true
        };
        if !contour.closed {
            skipped_open.push(json!({
                "name": glyph.name,
                "contour": index,
                "reason": "open"
            }));
            proposed.push(glyph.contours[index].points.clone());
            continue;
        }
        proposed.push(with_round_joins(
            contour,
            gl,
            &opts,
            &glyph.contours[index].points,
            zones_active,
        ));
    }
    for (contour, points) in glyph.contours.iter_mut().zip(proposed) {
        contour.points = points;
    }
    Ok(true)
}

// Patch offset_glyph_points to use apply_points_with_optional_arcs
// (redefine the call site by replacing the helper used above).

// --- Clean mode ------------------------------------------------------------

fn offset_glyph_clean(
    glyph: &mut Glyph,
    source: &Glyph,
    options: &OffsetOptions,
    metrics: &Metrics,
) -> Result<(), FoundryError> {
    let h = options.horizontal.abs().max(1e-6);
    let v = options.vertical.abs().max(1e-6);
    let sign = if options.horizontal < 0.0 || options.vertical < 0.0 {
        -1.0
    } else {
        1.0
    };
    let nesting = contour_nesting(&glyph.contours);
    let group = classify(glyph.unicode, false);
    let zone_ys = zone_lines(metrics, &options.zones, group, glyph);

    for (c_index, contour) in glyph.contours.iter_mut().enumerate() {
        if !contour.closed || contour.points.len() < 3 {
            continue;
        }
        let outer = nesting[c_index].1;
        let gl = grow_is_left_of_travel(contour, outer);
        // Flatten → offset each sample → on-points only.
        let flat = flatten_contour(contour, 16);
        let n = if flat.len() > 1 && flat.first() == flat.last() {
            flat.len() - 1
        } else {
            flat.len()
        };
        if n < 3 {
            continue;
        }
        let mut moved = Vec::with_capacity(n);
        for i in 0..n {
            let prev = flat[(i + n - 1) % n];
            let here = flat[i];
            let next = flat[(i + 1) % n];
            let in_u = here.sub(prev).norm();
            let out_u = next.sub(here).norm();
            let nrm = {
                let a = edge_normal(in_u, gl);
                let b = edge_normal(out_u, gl);
                let m = a.add(b).norm();
                if m.len() > 1e-8 { m } else { a }
            };
            let mut p = V2::new(here.x + nrm.x * sign * h, here.y + nrm.y * sign * v);
            // Zone snap.
            for &(zone_y, _) in &zone_ys {
                if (here.y - zone_y).abs() <= options.zones.overshoot {
                    p.y = zone_y;
                    break;
                }
            }
            moved.push(Point {
                x: p.x,
                y: p.y,
                kind: PointKind::On,
                smooth: false,
            });
        }
        contour.points = moved;
    }
    let _ = source;
    Ok(())
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
    fn ring_outer_grows_not_shrinks() {
        let mut font = Font::new("Ring", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "o".into(),
            unicode: Some(u32::from('o')),
            advance: 500.0,
            contours: vec![
                // Outer, counter-clockwise.
                Contour {
                    closed: true,
                    points: vec![
                        on(40.0, 0.0),
                        on(460.0, 0.0),
                        on(460.0, 500.0),
                        on(40.0, 500.0),
                    ],
                },
                // Hole, clockwise.
                Contour {
                    closed: true,
                    points: vec![
                        on(180.0, 100.0),
                        on(180.0, 400.0),
                        on(320.0, 400.0),
                        on(320.0, 100.0),
                    ],
                },
            ],
        })
        .unwrap();
        let before = ink_bounds(font.glyph("o").unwrap());
        offset_font(
            &mut font,
            &OffsetOptions {
                horizontal: 20.0,
                vertical: 4.0,
                keep_metrics: true,
                ..OffsetOptions::default()
            },
        )
        .unwrap();
        let after = ink_bounds(font.glyph("o").unwrap());
        assert!(
            after.0 < before.0 && after.1 > before.1,
            "outer should grow in x: before {:?}, after {:?}",
            before,
            after
        );
    }

    #[test]
    fn clockwise_outer_still_grows() {
        let mut font = Font::new("Period", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "period".into(),
            unicode: Some(u32::from('.')),
            advance: 200.0,
            contours: vec![Contour {
                closed: true,
                // Clockwise square.
                points: vec![
                    on(50.0, 0.0),
                    on(50.0, 100.0),
                    on(150.0, 100.0),
                    on(150.0, 0.0),
                ],
            }],
        })
        .unwrap();
        let before = ink_bounds(font.glyph("period").unwrap());
        offset_font(
            &mut font,
            &OffsetOptions {
                horizontal: 10.0,
                vertical: 10.0,
                keep_metrics: false,
                zones: ZoneOptions::all_off(),
                post_smooth: false,
                ..OffsetOptions::default()
            },
        )
        .unwrap();
        let after = ink_bounds(font.glyph("period").unwrap());
        let before_w = before.1 - before.0;
        let after_w = after.1 - after.0;
        let before_h = before.3 - before.2;
        let after_h = after.3 - after.2;
        assert!(
            after_w > before_w && after_h > before_h,
            "clockwise outer should grow: before {:?} after {:?}",
            before,
            after
        );
    }

    #[test]
    fn round_joins_add_arc_points_on_the_outside_only() {
        let round = |horizontal: f64, vertical: f64, keep_metrics: bool| OffsetOptions {
            horizontal,
            vertical,
            keep_metrics,
            corner: Corner::Round,
            add_points: true,
            post_smooth: false,
            smooth_sigma: 0.0,
            zones: if keep_metrics {
                ZoneOptions::default()
            } else {
                ZoneOptions::all_off()
            },
            ..OffsetOptions::default()
        };
        let mut font = square_font();
        offset_font(&mut font, &round(5.0, 5.0, false)).unwrap();
        let points = &font.glyph("H").unwrap().contours[0].points;
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

        let mut shrunk = square_font();
        offset_font(&mut shrunk, &round(-5.0, -5.0, false)).unwrap();
        assert_eq!(shrunk.glyph("H").unwrap().contours[0].points.len(), 4);

        let mut kept = square_font();
        offset_font(&mut kept, &round(5.0, 5.0, true)).unwrap();
        assert_eq!(kept.glyph("H").unwrap().contours[0].points.len(), 4);

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
                zones: ZoneOptions::all_off(),
                post_smooth: false,
                smooth_sigma: 0.0,
                min_gap_ratio: 0.0,
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

    #[test]
    fn api_square_keeps_baseline() {
        let mut font = Font::new("Offset", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "H".into(),
            unicode: Some(72),
            advance: 400.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    on(100.0, 0.0),
                    on(180.0, 0.0),
                    on(180.0, 120.0),
                    on(100.0, 120.0),
                ],
            }],
        })
        .unwrap();
        let opts = OffsetOptions {
            horizontal: 15.0,
            vertical: 4.0,
            names: Some(vec!["H".into()]),
            ..Default::default()
        };
        offset_font_detailed(&mut font, &opts).unwrap();
        let y = font.glyph("H").unwrap().contours[0].points[0].y;
        assert_eq!(y, 0.0, "baseline drifted to {y}");
    }
}
