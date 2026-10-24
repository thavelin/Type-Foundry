use super::vector::{self, Outline};
use super::*;
use crate::geometry::{next_segment, prev_segment};
use crate::{Font, Glyph, PointKind};
use kurbo::{ParamCurve, ParamCurveArclen, Shape};

pub(super) fn glyph(font: &Font, ch: char) -> Option<&Glyph> {
    font.glyphs
        .iter()
        .find(|g| g.unicode == Some(ch as u32))
        .or_else(|| font.glyph(&ch.to_string()))
}
fn positive(v: Option<f64>) -> Option<f64> {
    v.filter(|v| *v > 0. && v.is_finite())
}
fn median(mut values: Vec<f64>) -> Option<f64> {
    values.retain(|v| v.is_finite());
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        None
    } else {
        let n = values.len();
        Some((values[(n - 1) / 2] + values[n / 2]) * 0.5)
    }
}
fn selected(g: &Glyph, options: &Options) -> bool {
    options.glyphs.is_empty()
        || options.glyphs.iter().any(|s| {
            s == &g.name
                || g.unicode
                    .and_then(char::from_u32)
                    .is_some_and(|c| s == &c.to_string())
                || g.unicode
                    .is_some_and(|u| s.eq_ignore_ascii_case(&format!("U+{u:04X}")))
        })
}

pub(super) fn study(font: &Font, source: Source, mut metadata: Value, options: &Options) -> Study {
    let outlines: BTreeMap<_, _> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
        .chars()
        .filter_map(|c| glyph(font, c).map(|g| (c, Outline::new(g))))
        .collect();
    let shape = |c: char| outlines.get(&c);
    let bound = |c: char| shape(c).and_then(|o| o.bounds);
    let x = positive(metadata["vertical_metrics"]["x_height"].as_f64())
        .or_else(|| bound('x').map(|b| b.y1));
    let cap = positive(metadata["vertical_metrics"]["cap_height"].as_f64())
        .or_else(|| bound('H').map(|b| b.y1));
    let xh = x.unwrap_or(0.);
    let cp = cap.unwrap_or(0.);
    let stroke = |c: char, f: f64| {
        shape(c)
            .zip(bound(c))
            .and_then(|(o, b)| o.stroke(b.y0 + b.height() * f, false))
    };
    let vertical = |c: char| {
        median(
            [0.25, 0.4, 0.6]
                .into_iter()
                .filter_map(|f| stroke(c, f))
                .collect(),
        )
    };
    let horizontal = |c: char| {
        shape(c)
            .zip(bound(c))
            .and_then(|(o, b)| o.stroke((b.x0 + b.x1) * 0.5, true))
    };
    let stem = vertical('n');
    let stem_h = vertical('H');
    let stem_o = vertical('o');
    let bar_o = horizontal('o');
    let mut metrics = BTreeMap::new();
    let mut add = |name: &str, value: Option<f64>, units: &str, method: &str, used: &[&str]| {
        metrics.insert(name.into(), Metric::new(value, units, method, used, xh, cp));
    };
    add(
        "proportions.x_cap",
        ratio(x, cap),
        "ratio",
        "source heights; fallback exact x/H outline maxima",
        &["x", "H"],
    );
    add(
        "proportions.ascender_x",
        ratio(metadata["vertical_metrics"]["ascender"].as_f64(), x),
        "ratio",
        "declared ascender / x-height",
        &[],
    );
    add(
        "proportions.descender_x",
        ratio(
            metadata["vertical_metrics"]["descender"]
                .as_f64()
                .map(f64::abs),
            x,
        ),
        "ratio",
        "absolute declared descender / x-height",
        &[],
    );
    add(
        "proportions.cap_upm",
        ratio(cap, Some(font.upm as f64)),
        "ratio",
        "cap height / UPM",
        &["H"],
    );
    for (name, v, used) in [
        ("stems.lowercase", stem, "n"),
        ("stems.uppercase", stem_h, "H"),
        ("stems.o_sides", stem_o, "o"),
    ] {
        add(
            name,
            v,
            "font_units",
            "median minimum filled horizontal-ray span at 25/40/60% ink height",
            &[used],
        );
    }
    for (name, c) in [
        ("strokes.o_top", 'o'),
        ("strokes.H_bar", 'H'),
        ("strokes.e_bar", 'e'),
    ] {
        add(
            name,
            horizontal(c),
            "font_units",
            "minimum filled vertical-ray span at ink-width midpoint",
            &[&c.to_string()],
        );
    }
    add(
        "stems.contrast",
        ratio(stem_o, bar_o)
            .filter(|_| shape('o').and_then(Outline::inner).is_some())
            .map(|v| v.max(1. / v)),
        "ratio",
        "max(o side / o top, reciprocal)",
        &["o"],
    );
    add(
        "weight.stem_x_height",
        ratio(stem, x),
        "ratio",
        "lowercase vertical stem / x-height",
        &["n"],
    );
    // Radial thickness and stress: sample 180 directions about the o bounding-box centre.
    let stress = shape('o').zip(bound('o')).and_then(|(o, b)| {
        o.inner()?;
        let center = kurbo::Point::new((b.x0 + b.x1) * 0.5, (b.y0 + b.y1) * 0.5);
        let radius = b.width().max(b.height());
        let mut samples = Vec::new();
        for angle in 0..180 {
            let a = (angle as f64).to_radians();
            let d = kurbo::Vec2::new(a.cos(), a.sin());
            let line = kurbo::Line::new(center, center + d * radius);
            let mut hits: Vec<_> = o
                .paths
                .iter()
                .flat_map(|p| p.segments())
                .flat_map(|s| {
                    s.intersect_line(line)
                        .into_iter()
                        .map(move |h| s.eval(h.segment_t).distance(center))
                })
                .collect();
            hits.sort_by(f64::total_cmp);
            hits.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
            if hits.len() == 2 {
                samples.push((angle as f64, hits[1] - hits[0]));
            }
        }
        let min = samples.iter().min_by(|a, b| a.1.total_cmp(&b.1))?;
        let max = samples.iter().map(|p| p.1).max_by(f64::total_cmp)?;
        if max / min.1 < 1.01 {
            None
        } else {
            Some(min.0 - 90.)
        }
    });
    add(
        "stems.stress_angle",
        stress,
        "degrees",
        "thinnest o radial span; 180 rays; degrees from vertical; null for monoline ambiguity",
        &["o"],
    );
    for c in ['o', 'O', 's', 'a'] {
        let zone = if c.is_uppercase() { cap } else { x };
        let used = c.to_string();
        add(
            &format!("overshoot.{c}.above"),
            bound(c).zip(zone).map(|(b, z)| b.y1 - z),
            "font_units",
            "exact Bezier maximum minus zone (signed)",
            &[&used],
        );
        add(
            &format!("overshoot.{c}.below"),
            bound(c).map(|b| -b.y0),
            "font_units",
            "baseline minus exact Bezier minimum (signed)",
            &[&used],
        );
    }
    let mut arch_samples = Vec::new();
    for fraction in [0.1, 0.2, 0.5, 0.9] {
        let thickness = shape('n').zip(bound('n')).and_then(|(o, b)| {
            {
                let stems = o.intervals(b.y0 + 0.3 * b.height(), false);
                let lo = stems.first()?.1;
                let hi = stems.last()?.0;
                if hi <= lo {
                    return None;
                }
                o.intervals(lo + (hi - lo) * fraction, true)
            }
            .last()
            .map(|(a, b)| b - a)
        });
        add(
            &format!("arch.n.thickness_{}", (fraction * 100.) as u32),
            thickness,
            "font_units",
            "topmost filled vertical-ray span across counter gap between stem interiors at 30% height",
            &["n"],
        );
    }
    if let Some((o, b)) = shape('n').zip(bound('n')) {
        for i in 1..100 {
            let at = b.x0 + b.width() * i as f64 / 100.;
            if let Some((a, z)) = o.intervals(at, true).last()
                && z - a < 0.5 * b.height()
            {
                arch_samples.push((at, *a, z - a));
            }
        }
    }
    add(
        "arch.n.thinnest",
        arch_samples.iter().map(|p| p.2).min_by(f64::total_cmp),
        "font_units",
        "minimum top span over 99 vertical rays excluding spans taller than half ink height",
        &["n"],
    );
    add(
        "arch.n.springing_x",
        ratio(arch_samples.iter().map(|p| p.1).min_by(f64::total_cmp), x),
        "ratio",
        "minimum underside height in sampled arch / x-height",
        &["n"],
    );
    for c in ['n', 'o', 'O'] {
        let inner = shape(c).and_then(Outline::inner);
        let radius = inner
            .and_then(|p| {
                p.segments()
                    .flat_map(|s| {
                        (1..100).map(move |i| vector::curvature(s, i as f64 / 100.).abs())
                    })
                    .filter(|k| k.is_finite() && *k > 1e-9)
                    .max_by(f64::total_cmp)
            })
            .map(|v| 1. / v);
        add(
            &format!("inner.{c}.tightest_radius"),
            radius,
            "font_units",
            "minimum radius from analytic curvature sampled at 99 positions per inner Bezier",
            &[&c.to_string()],
        );
        for delta in [0.02, 0.05] {
            let rise = inner.zip(x).and_then(|(p, x)| {
                let b = p.bounding_box();
                let cut = b.x0 + delta * x;
                let line = kurbo::Line::new((cut, b.y0 - 1.), (cut, b.y1 + 1.));
                p.segments()
                    .flat_map(|s| {
                        s.intersect_line(line)
                            .into_iter()
                            .map(move |h| s.eval(h.segment_t).y)
                    })
                    .max_by(f64::total_cmp)
                    .map(|top| b.y1 - top)
            });
            add(
                &format!("inner.{c}.rise_{:02}", (delta * 100.) as u32),
                rise,
                "font_units",
                "counter top minus upper counter intersection at leftmost counter + fraction*x-height",
                &[&c.to_string()],
            );
        }
        let handles = glyph(font, c).and_then(super::advanced::inner_handle_fraction);
        add(
            &format!("inner.{c}.handle_fraction"),
            handles,
            "ratio",
            "median first control-handle length / chord on closed counter; n uses concave segments",
            &[&c.to_string()],
        );
        if c != 'n' {
            let exponent = inner.and_then(|p| {
                let b = p.bounding_box();
                if b.width() <= 0. || b.height() <= 0. {
                    return None;
                }
                let mut points = Vec::new();
                for seg in p.segments() {
                    for i in 1..16 {
                        let q = seg.eval(i as f64 / 16.);
                        let u = ((q.x - (b.x0 + b.x1) * 0.5) / (b.width() * 0.5)).abs();
                        let v = ((q.y - (b.y0 + b.y1) * 0.5) / (b.height() * 0.5)).abs();
                        if u > 0.05 && u < 0.95 && v > 0.05 && v < 0.95 {
                            points.push((u, v));
                        }
                    }
                }
                if points.len() < 8 {
                    return None;
                }
                (50..=800).map(|n| n as f64 / 100.).min_by(|a, b| {
                    let error = |n: f64| {
                        points
                            .iter()
                            .map(|(u, v)| (u.powf(n) + v.powf(n) - 1.).powi(2))
                            .sum::<f64>()
                    };
                    error(*a).total_cmp(&error(*b))
                })
            });
            add(
                &format!("inner.{c}.superellipse_exponent"),
                exponent,
                "ratio",
                "axis-aligned counter superellipse least-squares grid fit p=0.50..8.00 in 0.01 steps",
                &[&c.to_string()],
            );
        }
    }
    // Foot extension, relative to the mid-stem, is classification evidence rather than a name-table label.
    let mut serif_evidence = Vec::new();
    for c in ['n', 'H', 'I'] {
        let foot = shape(c).zip(bound(c)).and_then(|(o, b)| {
            let foot = o.intervals(b.y0 + 0.02 * b.height(), false);
            let mid = o.intervals(b.y0 + 0.25 * b.height(), false);
            let f = foot.first()?;
            let m = mid.first()?;
            ratio(Some((m.0 - f.0).max(f.1 - m.1)), Some(m.1 - m.0))
        });
        if let Some(v) = foot {
            serif_evidence.push(v);
        }
        add(
            &format!("serif.{c}.foot_extension"),
            foot,
            "ratio",
            "first stem's max foot extension at 2% height vs 25%, divided by mid-stem",
            &[&c.to_string()],
        );
    }
    add(
        "serif.presence_score",
        median(serif_evidence),
        "ratio",
        "median n/H/I foot-extension ratios",
        &["n", "H", "I"],
    );
    for c in ['n', 'k', '4', '6', '9', '8'] {
        let thinning = shape(c).zip(bound(c)).and_then(|(o, b)| {
            let widths: Vec<_> = (20..80)
                .filter_map(|i| o.stroke(b.y0 + b.height() * i as f64 / 100., false))
                .collect();
            let min = widths.iter().copied().min_by(f64::total_cmp);
            ratio(min, median(widths))
        });
        add(
            &format!("joins.{c}.thinning_ratio"),
            thinning,
            "ratio",
            "minimum / median filled horizontal-ray thickness in central 20..79% band; join/waist proxy",
            &[&c.to_string()],
        );
    }
    let advances: Vec<_> = font
        .glyphs
        .iter()
        .filter(|g| {
            g.unicode
                .and_then(char::from_u32)
                .is_some_and(|c| c.is_ascii_alphanumeric())
        })
        .map(|g| g.advance)
        .collect();
    let spread = advances
        .iter()
        .copied()
        .max_by(f64::total_cmp)
        .zip(advances.iter().copied().min_by(f64::total_cmp))
        .map(|(a, b)| a - b);
    add(
        "widths.ascii_advance_count",
        Some(advances.len() as f64),
        "count",
        "encoded ASCII alphanumeric glyph count for monospacing evidence",
        &["a-z", "A-Z", "0-9"],
    );
    add(
        "widths.advance_spread_upm",
        ratio(spread, Some(font.upm as f64)),
        "ratio",
        "max minus min encoded ASCII alphanumeric advance / UPM",
        &["a-z", "A-Z", "0-9"],
    );
    add(
        "widths.o_x",
        ratio(bound('o').map(|b| b.width()), x),
        "ratio",
        "exact o ink width / x-height",
        &["o"],
    );
    let pairs = metadata["kerning"]["pairs"].as_f64();
    let count = font.glyphs.len() as f64;
    add(
        "spacing.kerning_density",
        ratio(pairs, Some(count * count)),
        "ratio",
        "explicit pair count / glyph-count squared; class coverage is not expanded",
        &[],
    );
    drop(add);
    metadata["measurement_profile"] = json!(options.profile);
    let bulk = options.profile == MeasurementProfile::Atlas;
    let mut area_cache: BTreeMap<String, Option<f64>> = BTreeMap::new();
    let mut area_for = |g: &Glyph| {
        *area_cache.entry(g.name.clone()).or_insert_with(|| {
            if bulk
                && (g.contours.len() > 24
                    || g.contours.iter().map(|c| c.points.len()).sum::<usize>() > 600)
            {
                None
            } else {
                super::advanced::ink_area(g)
            }
        })
    };
    let mut rows = BTreeMap::new();
    for g in font.glyphs.iter().filter(|g| selected(g, options)) {
        let o = Outline::new(g);
        let b = o.bounds;
        let used = [g.name.as_str()];
        let mut row = BTreeMap::new();
        let mut put = |k: &str, v: Option<f64>, units: &str, method: &str| {
            row.insert(k.into(), Metric::new(v, units, method, &used, xh, cp));
        };
        put(
            "advance",
            Some(g.advance),
            "font_units",
            "source horizontal advance",
        );
        put(
            "ink_width",
            b.map(|b| b.width()),
            "font_units",
            "analytic Bezier extrema",
        );
        put("lsb", b.map(|b| b.x0), "font_units", "analytic minimum x");
        put(
            "rsb",
            b.map(|b| g.advance - b.x1),
            "font_units",
            "advance minus analytic maximum x",
        );
        for (reference, c) in [("n", 'n'), ("H", 'H')] {
            put(
                &format!("advance_relative_{reference}"),
                ratio(Some(g.advance), glyph(font, c).map(|g| g.advance)),
                "ratio",
                "advance / reference glyph advance",
            );
            put(
                &format!("ink_width_relative_{reference}"),
                ratio(b.map(|b| b.width()), bound(c).map(|b| b.width())),
                "ratio",
                "ink width / reference ink width",
            );
        }
        let area = if g.contours.is_empty() {
            Some(0.)
        } else {
            area_for(g)
        };
        put(
            "ink_area",
            area,
            "font_units_squared",
            "exact signed Bezier area; detected overlaps use adaptive nonzero-winding integration (relative tolerance 1e-8, depth 16)",
        );
        let darkness = ratio(area, Some(g.advance));
        put(
            "darkness",
            darkness,
            "font_units",
            "ink area / advance; normalize by x-height for dimensionless coverage",
        );
        for c in ['n', 'H'] {
            let ref_dark = glyph(font, c).and_then(|g| ratio(area_for(g), Some(g.advance)));
            put(
                &format!("darkness_relative_{c}"),
                ratio(darkness, ref_dark),
                "ratio",
                "glyph (ink area / advance) / reference (ink area / advance)",
            );
        }
        let reference_stem = if g
            .unicode
            .and_then(char::from_u32)
            .is_some_and(|c| c.is_uppercase())
        {
            stem_h
        } else {
            stem
        };
        put(
            "lsb_stem",
            ratio(b.map(|b| b.x0), reference_stem),
            "ratio",
            "LSB / lowercase n or uppercase H stem",
        );
        put(
            "rsb_stem",
            ratio(b.map(|b| g.advance - b.x1), reference_stem),
            "ratio",
            "RSB / lowercase n or uppercase H stem",
        );
        put(
            "points",
            Some(g.contours.iter().map(|c| c.points.len()).sum::<usize>() as f64),
            "count",
            "count all on-curve and off-curve points",
        );
        let mut kinks = 0;
        let near_zero = g
            .contours
            .iter()
            .map(|c| {
                vector::path(c)
                    .segments()
                    .filter(|s| s.arclen(1e-6) < font.upm as f64 * 1e-6)
                    .count()
            })
            .sum::<usize>();
        let mut cw = 0;
        let mut ccw = 0;
        for contour in &g.contours {
            let p = vector::path(contour);
            if p.area() > 0. {
                ccw += 1
            } else if p.area() < 0. {
                cw += 1
            }
            for (i, p) in contour
                .points
                .iter()
                .enumerate()
                .filter(|(_, p)| p.kind == PointKind::On)
            {
                if let Some((j, controls)) = next_segment(&contour.points, i, contour.closed)
                    && p.smooth
                    && let Some((prev, ins)) = prev_segment(&contour.points, i, contour.closed)
                {
                    let a = ins.last().copied().unwrap_or(&contour.points[prev]);
                    let b = controls.first().copied().unwrap_or(&contour.points[j]);
                    let u = kurbo::Vec2::new(p.x - a.x, p.y - a.y);
                    let v = kurbo::Vec2::new(b.x - p.x, b.y - p.y);
                    if u.hypot() > 1e-8
                        && v.hypot() > 1e-8
                        && (u.dot(v) / (u.hypot() * v.hypot()))
                            .clamp(-1., 1.)
                            .acos()
                            .to_degrees()
                            > 1.
                    {
                        kinks += 1;
                    }
                }
            }
        }
        put(
            "quality.kinks",
            Some(kinks as f64),
            "count",
            "tangent breaks >1 degree at source smooth points; binary imports do not preserve smooth flags",
        );
        put(
            "quality.near_zero_segments",
            Some(near_zero as f64),
            "count",
            "segments with Bezier arclength shorter than 1e-6 UPM, integration accuracy 1e-6",
        );
        put(
            "quality.missing_extrema",
            Some(vector::extrema_missing(g) as f64),
            "count",
            "analytic interior Bezier extrema without explicit split points",
        );
        put(
            "quality.clockwise_contours",
            Some(cw as f64),
            "count",
            "negative exact signed area",
        );
        put(
            "quality.counterclockwise_contours",
            Some(ccw as f64),
            "count",
            "positive exact signed area",
        );
        row.insert("quality.overlaps".into(),Metric::new(if bulk{None}else{Some(super::advanced::overlap_count(g) as f64)},"count","12-step curve polygon intersection flags plus same-winding nested contours; approximate, tangencies may be missed",&used,xh,cp));
        if bulk {
            if let Some(m) = row.get_mut("quality.overlaps") {
                m.reason=Some("atlas profile defers quadratic overlap census; use full profile on selected fonts".into());
            }
            if area.is_none() {
                for key in [
                    "ink_area",
                    "darkness",
                    "darkness_relative_n",
                    "darkness_relative_H",
                ] {
                    if let Some(m) = row.get_mut(key) {
                        m.reason=Some("atlas profile area budget exceeded (>24 contours or >600 points); not measured zero".into());
                    }
                }
            }
        }
        rows.insert(g.name.clone(), row);
    }
    // Copy the key width/spacing table into the scalar feature map used by class ranges.
    for c in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".chars() {
        let used = c.to_string();
        for (key, units) in [
            ("advance", "font_units"),
            ("ink_width", "font_units"),
            ("lsb", "font_units"),
            ("rsb", "font_units"),
            ("advance_relative_n", "ratio"),
            ("advance_relative_H", "ratio"),
            ("lsb_stem", "ratio"),
            ("rsb_stem", "ratio"),
        ] {
            let m = glyph(font, c)
                .and_then(|g| rows.get(&g.name))
                .and_then(|r| r.get(key))
                .cloned()
                .unwrap_or_else(|| {
                    Metric::new(None, units, "missing or excluded glyph", &[&used], xh, cp)
                });
            metrics.insert(format!("widths.{c}.{key}"), m);
        }
    }
    let text = "The quick brown fox jumps over the lazy dog 0123456789";
    let mut area = 0.;
    let mut advance = 0.;
    let mut complete = true;
    for c in text.chars() {
        if let Some(g) = glyph(font, c) {
            if let Some(value) = area_for(g) {
                area += value;
            } else {
                complete = false;
            }
            advance += g.advance;
        } else {
            complete = false;
        }
    }
    metrics.insert("spacing.texture_greyness".into(),Metric::new(if complete {ratio(Some(area),Some(advance*(font.metrics.ascender-font.metrics.descender)))}else{None},"ratio","sum outline ink areas / (unshaped advances * declared ascender-descender); standard ASCII pangram plus digits; no kerning",&[text],xh,cp));
    if bulk
        && !complete
        && let Some(m) = metrics.get_mut("spacing.texture_greyness")
    {
        m.reason = Some(
            "atlas profile: a pangram glyph is missing or exceeds the area complexity budget"
                .into(),
        );
    }
    let terminals = super::advanced::terminals(font, xh, cp);
    for c in "cesrCGS".chars() {
        let value = terminals
            .get(&c.to_string())
            .and_then(|v| v["angle"].as_f64());
        metrics.insert(format!("terminals.{c}.end_cut_angle"),Metric::new(value,"degrees","short straight cuts bounded by corners in expected opening region; heuristic; see terminals map",&[&c.to_string()],xh,cp));
    }
    let (radius, rises) = super::advanced::n_inner(font, xh);
    metrics.insert(
        "inner.n.tightest_radius".into(),
        Metric::new(
            radius,
            "font_units",
            "minimum sampled concave radius in upper 70% of n; open counter underside",
            &["n"],
            xh,
            cp,
        ),
    );
    for (delta, rise) in [2, 5].into_iter().zip(rises) {
        metrics.insert(
            format!("inner.n.rise_{delta:02}"),
            Metric::new(
                rise,
                "font_units",
                "rise of n counter underside from near stem at 0.02/0.05 x-height inward",
                &["n"],
                xh,
                cp,
            ),
        );
    }

    let classification = super::advanced::refine_classification(
        classify(&metrics, &metadata),
        font,
        &metrics,
        &metadata,
    );
    Study{format:"typefoundry.study".into(),version:VERSION,algorithm:ALGORITHM.into(),source,metadata,metrics,glyphs:rows,classification,terminals,nearest:vec![],outliers:vec![],warnings:vec!["Classification and arch/join/serif measurements are heuristics; inspect method fields. Area integrates nonzero winding for detected overlaps; overlap detection uses sampled curves. Complex terminals can return null; overlap census is approximate. No shaping or hinting is applied.".into()]}
}
fn classify(m: &BTreeMap<String, Metric>, _metadata: &Value) -> Classification {
    let v = |k: &str| m.get(k).and_then(|v| v.value);
    let serif = v("serif.presence_score");
    let contrast = v("stems.contrast");
    let spread = v("widths.advance_spread_upm");
    let round = v("widths.o_x");
    let stress = v("stems.stress_angle");
    let serif_present = serif.is_some_and(|v| v > 0.16);
    let (class, confidence, reasons) = if v("widths.ascii_advance_count").unwrap_or(0.) >= 20.
        && spread.is_some_and(|v| v < 0.002)
    {
        (
            "mono",
            0.92,
            vec![
                "ASCII alphanumeric advances agree within 0.002 em".into(),
                format!("advance spread = {:.5} em", spread.unwrap_or(0.)),
                "measured advances, independent of font name".into(),
            ],
        )
    } else if serif_present {
        let slab = contrast.is_some_and(|v| v < 1.4);
        (
            if slab { "slab" } else { "serif text" },
            0.65,
            vec![
                format!("foot extension / stem = {:.3}", serif.unwrap_or(0.)),
                contrast
                    .map(|v| format!("o contrast = {v:.3}"))
                    .unwrap_or_else(|| "o contrast unavailable".into()),
                "foot region measured on n, H and I".into(),
            ],
        )
    } else if v("weight.stem_x_height").is_some_and(|v| !(0.035..=0.32).contains(&v))
        || v("proportions.x_cap").is_some_and(|v| !(0.45..=0.9).contains(&v))
    {
        (
            "display",
            0.45,
            vec![
                "extreme stem weight or x/cap proportion".into(),
                "no convincing serif foot extension".into(),
                "display is a low-confidence residual heuristic".into(),
            ],
        )
    } else if round.is_some_and(|v| (0.92..1.08).contains(&v)) && contrast.is_some_and(|v| v < 1.25)
    {
        (
            "geometric sans",
            0.66,
            vec![
                format!("o ink width / x-height = {:.3}", round.unwrap_or(0.)),
                "near-monoline o".into(),
                "no foot extensions detected".into(),
            ],
        )
    } else if stress.is_some_and(|v| v.abs() > 12.) || contrast.is_some_and(|v| v > 1.35) {
        (
            "humanist sans",
            0.58,
            vec![
                "oblique o stress or stroke modulation".into(),
                contrast
                    .map(|v| format!("o contrast = {v:.3}"))
                    .unwrap_or_else(|| "o contrast unavailable".into()),
                "no convincing serif foot extensions".into(),
            ],
        )
    } else {
        (
            "grotesque",
            if contrast.is_some() && serif.is_some() {
                0.5
            } else {
                0.15
            },
            vec![
                "sans residual class under current rules".into(),
                "geometric and humanist evidence below thresholds or missing".into(),
                "missing evidence lowers confidence; script is not inferred from italic alone"
                    .into(),
            ],
        )
    };
    let serif_type = if !serif_present {
        Some("none".into())
    } else if contrast.is_some_and(|v| v < 1.4) {
        Some("slab".into())
    } else {
        None
    };
    Classification{class:class.into(),confidence,reasons,serif_type,method:"outline-rules-v1; uncalibrated confidence; bracketed/hairline/wedge and script need reliable terminal segmentation".into()}
}
