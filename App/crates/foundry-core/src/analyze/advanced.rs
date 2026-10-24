//! Heuristics that need contour neighborhoods. All thresholds are in font units or x-height ratios.
use super::vector::{self, Outline};
use super::{Classification, Metric};
use crate::{Font, Glyph};
use kurbo::{ParamCurve, PathSeg, Shape};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn terminals(font: &Font, x: f64, cap: f64) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for c in "cesrCGS".chars() {
        let Some(g) = super::measure::glyph(font, c) else {
            out.insert(c.to_string(),json!({"classification":null,"angle":null,"reason":"glyph missing","method":"short corner-bounded straight terminal candidates"}));
            continue;
        };
        let o = Outline::new(g);
        let Some(b) = o.bounds else {
            continue;
        };
        let zone = if c.is_uppercase() { cap } else { x };
        let mut candidates = Vec::new();
        for p in &o.paths {
            let segs: Vec<_> = p.segments().collect();
            let count = segs.len();
            if count < 3 {
                continue;
            }
            for (i, seg) in segs.iter().enumerate() {
                let PathSeg::Line(line) = seg else {
                    continue;
                };
                let length = line.p0.distance(line.p1);
                if length < zone * 0.015 || length > zone * 0.4 {
                    continue;
                }
                let midpoint = seg.eval(0.5);
                let right = (midpoint.x - b.x0) / b.width();
                // c/C/G open right, e/r upper right, s/S have terminals at opposite corners.
                let eligible = match c {
                    'c' | 'C' | 'G' => right > 0.55,
                    'e' | 'r' => right > 0.45 && midpoint.y > b.y0 + b.height() * 0.3,
                    's' | 'S' => {
                        (right > 0.55 && midpoint.y > b.y0 + b.height() * 0.6)
                            || (right < 0.45 && midpoint.y < b.y0 + b.height() * 0.4)
                    }
                    _ => false,
                };
                if !eligible {
                    continue;
                }
                let before = segs[(i + count - 1) % count];
                let after = segs[(i + 1) % count];
                let v = line.p1 - line.p0;
                let incoming = before.eval(1.) - before.eval(0.99);
                let outgoing = after.eval(0.01) - after.eval(0.);
                let angle = |a: kurbo::Vec2, b: kurbo::Vec2| {
                    if a.hypot() * b.hypot() > 1e-10 {
                        (a.dot(b) / (a.hypot() * b.hypot()))
                            .clamp(-1., 1.)
                            .acos()
                            .to_degrees()
                    } else {
                        0.
                    }
                };
                if angle(incoming, v) < 25. || angle(v, outgoing) < 25. {
                    continue;
                }
                let degrees = v.y.atan2(v.x).to_degrees().rem_euclid(180.);
                let degrees = if degrees > 90. {
                    degrees - 180.
                } else {
                    degrees
                };
                candidates.push((right, length, degrees));
            }
        }
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.total_cmp(&b.1)));
        let chosen = candidates.first();
        let angle = chosen.map(|v| v.2);
        let classification = angle.map(|angle| {
            if angle.abs() < 10. {
                "horizontal"
            } else if (angle.abs() - 90.).abs() < 10. {
                "vertical"
            } else {
                "angled"
            }
        });
        out.insert(c.to_string(),json!({"classification":classification,"angle":angle,"candidate_angles":candidates.iter().map(|v|v.2).collect::<Vec<_>>(),"confidence":if angle.is_some(){0.45}else{0.},"method":"straight segment bounded by >=25 degree corners in expected opening region; rightmost candidate; degrees from horizontal","reason":if angle.is_none(){Some("no unambiguous straight cut; ball/taper/serif/spur require semantic segmentation")}else{None}}));
    }
    out
}
pub(super) fn serif_type(font: &Font, m: &BTreeMap<String, Metric>) -> Option<String> {
    if m.get("serif.presence_score")
        .and_then(|m| m.value)
        .is_none_or(|v| v <= 0.16)
    {
        return Some("none".into());
    }
    let mut thickness = Vec::new();
    let mut curves = 0;
    let mut sloped = 0;
    for c in ['n', 'H', 'I'] {
        let Some(g) = super::measure::glyph(font, c) else {
            continue;
        };
        let o = Outline::new(g);
        let Some(b) = o.bounds else {
            continue;
        };
        let Some(mid) = o
            .intervals(b.y0 + 0.25 * b.height(), false)
            .first()
            .copied()
        else {
            continue;
        };
        let Some(foot) = o
            .intervals(b.y0 + 0.02 * b.height(), false)
            .first()
            .copied()
        else {
            continue;
        };
        let x = (foot.0 + mid.0) * 0.5;
        if mid.0 - foot.0 < 1. {
            continue;
        }
        if let Some((lo, hi)) = o.intervals(x, true).first() {
            thickness.push((hi - lo) / (mid.1 - mid.0));
        }
        for p in &o.paths {
            for s in p.segments() {
                let q = s.eval(0.5);
                if q.y < b.y0 + b.height() * 0.18 && q.x < mid.0 {
                    match s {
                        PathSeg::Line(line) => {
                            let v = line.p1 - line.p0;
                            if v.x.abs() > 1e-6 && v.y.abs() / v.x.abs() > 0.15 {
                                sloped += 1;
                            }
                        }
                        _ => curves += 1,
                    }
                }
            }
        }
    }
    let t = if thickness.is_empty() {
        return None;
    } else {
        thickness.iter().sum::<f64>() / thickness.len() as f64
    };
    Some(
        if t < 0.2 {
            "hairline"
        } else if curves > 0 {
            "bracketed"
        } else if sloped > 0 {
            "wedge"
        } else {
            "slab"
        }
        .into(),
    )
}
pub(super) fn n_inner(font: &Font, x: f64) -> (Option<f64>, [Option<f64>; 2]) {
    let Some(g) = super::measure::glyph(font, 'n') else {
        return (None, [None, None]);
    };
    let o = Outline::new(g);
    let Some(b) = o.bounds else {
        return (None, [None, None]);
    };
    let radius = o
        .paths
        .iter()
        .flat_map(|p| {
            let sign = p.area().signum();
            p.segments().flat_map(move |s| {
                (1..100).filter_map(move |i| {
                    let t = i as f64 / 100.;
                    let q = s.eval(t);
                    let k = vector::curvature(s, t);
                    if k * sign > 1e-9 && q.y > b.y0 + 0.3 * b.height() {
                        Some(1. / k.abs())
                    } else {
                        None
                    }
                })
            })
        })
        .min_by(f64::total_cmp);
    let stem_edge = o
        .intervals(b.y0 + 0.3 * b.height(), false)
        .first()
        .map(|v| v.1);
    let underside = |at: f64| {
        o.intervals(at, true)
            .last()
            .map(|v| v.0)
            .filter(|v| *v > b.y0 + 0.3 * b.height())
    };
    let spring = stem_edge.and_then(|at| underside(at + x * 0.001));
    let rises = [0.02, 0.05].map(|delta| {
        stem_edge
            .and_then(|at| underside(at + delta * x))
            .zip(spring)
            .map(|(a, b)| a - b)
    });
    (radius, rises)
}
pub(super) fn overlap_count(g: &Glyph) -> usize {
    let issues = crate::geometry::outline_valid(g, None);
    let crossing = issues
        .iter()
        .filter(|v| matches!(v.code, "self-intersection" | "contour-cross"))
        .count();
    let o = Outline::new(g);
    let mut nested = 0;
    for p in &o.paths {
        if let Some(seg) = p.segments().next() {
            let q = seg.eval(0.5);
            let sum: i32 = o
                .paths
                .iter()
                .filter(|other| !std::ptr::eq(*other, p))
                .map(|p| p.winding(q))
                .sum();
            if sum != 0 && sum.signum() == p.area().signum() as i32 {
                nested += 1;
            }
        }
    }
    crossing + nested
}
pub(super) fn refine_classification(
    mut class: Classification,
    font: &Font,
    m: &BTreeMap<String, Metric>,
    metadata: &Value,
) -> Classification {
    class.serif_type = serif_type(font, m);
    if matches!(
        class.class.as_str(),
        "geometric sans" | "humanist sans" | "grotesque"
    ) {
        let value = |key: &str| m.get(key).and_then(|m| m.value);
        let round = value("widths.o_x");
        let contrast = value("stems.contrast");
        let aperture =
            value("aperture.c.opening_ratio").or_else(|| value("aperture.C.opening_ratio"));
        let terminal =
            value("terminals.c.end_cut_angle").or_else(|| value("terminals.C.end_cut_angle"));
        let stress = value("stems.stress_angle");
        let c_open = aperture.map(|v| ((v - 0.35) / 0.3).clamp(0., 1.));
        let e_open = value("aperture.e.opening_ratio").map(|v| ((v - 0.17) / 0.11).clamp(0., 1.));
        let open = match (c_open, e_open) {
            (Some(c), Some(e)) => (c + e) * 0.5,
            (Some(c), None) => c,
            (None, Some(e)) => e,
            _ => 0.5,
        };
        let modulation = contrast
            .map(|v| ((v - 1.) / 0.5).clamp(0., 1.))
            .unwrap_or(0.5);
        let roundness = round
            .map(|v| (1. - (v - 1.).abs() / 0.2).clamp(0., 1.))
            .unwrap_or(0.5);
        let vertical = terminal
            .map(|v| ((v.abs() - 45.) / 45.).clamp(0., 1.))
            .unwrap_or(0.);
        let oblique = terminal
            .map(|v| (1. - (v.abs() - 35.).abs() / 35.).clamp(0., 1.))
            .unwrap_or(0.5);
        let horizontal = terminal
            .map(|v| (1. - v.abs() / 45.).clamp(0., 1.))
            .unwrap_or(0.);
        let mut scores = [
            (
                "geometric sans",
                0.30 * roundness + 0.30 * (1. - modulation) + 0.30 * (1. - open) + 0.10 * oblique,
            ),
            (
                "humanist sans",
                0.50 * open
                    + 0.15 * vertical
                    + 0.35 * modulation
                    + 0.15 * stress.map(|v| (v.abs() / 20.).min(1.)).unwrap_or(0.),
            ),
            ("grotesque", 0.45 + 0.15 * (1. - open) + 0.20 * horizontal),
        ];
        scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        let margin = scores[0].1 - scores[1].1;
        let evidence = [round, contrast, aperture, terminal]
            .iter()
            .filter(|v| v.is_some())
            .count() as f64
            / 4.;
        class.class = scores[0].0.into();
        class.confidence = (0.35 + 0.55 * margin / (scores[0].1 + 0.1) + 0.10 * evidence).min(0.9)
            * evidence.sqrt();
        class.reasons = vec![
            format!(
                "outline evidence scores: {} {:.3}, {} {:.3}, {} {:.3}; margin {:.3}",
                scores[0].0,
                scores[0].1,
                scores[1].0,
                scores[1].1,
                scores[2].0,
                scores[2].1,
                margin
            ),
            format!(
                "o roundness {round:?}, contrast {contrast:?}, c aperture {aperture:?}, terminal angle {terminal:?}"
            ),
            "confidence follows the winning margin and evidence coverage; uncalibrated heuristic"
                .into(),
        ];
    }
    // Connected writing evidence: cursive OpenType attachment plus a slanted design.
    if class.class != "mono"
        && font.style.italic
        && (font.features.as_deref().is_some_and(|s| s.contains("curs"))
            || metadata["features"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == "curs")))
    {
        class.class = "script".into();
        class.confidence = 0.5;
        class.reasons = vec![
            "source has cursive attachment feature".into(),
            "source declares italic/slanted design".into(),
            "script inferred from combined metadata and outline analysis; verify specimen".into(),
        ];
    }
    class.method="outline-rules-v2; sans score margin and evidence coverage; serif type from foot thickness, bracket curves and wedge slopes".into();
    class
}

pub(super) fn ink_area(g: &Glyph) -> Option<f64> {
    if g.contours.is_empty() {
        return Some(0.);
    }
    let o = Outline::new(g);
    if overlap_count(g) > 0 {
        o.union_area()
    } else {
        o.area()
    }
}

pub(super) fn inner_handle_fraction(g: &Glyph) -> Option<f64> {
    let o = Outline::new(g);
    let counter = o.inner();
    let mut values = Vec::new();
    for p in &o.paths {
        let is_counter = counter.is_some_and(|c| std::ptr::eq(c, p));
        for seg in p.segments() {
            if !is_counter
                && !(g.unicode == Some('n' as u32)
                    && vector::curvature(seg, 0.5) * p.area().signum() > 1e-9)
            {
                continue;
            }
            let (handle, chord) = match seg {
                PathSeg::Line(_) => continue,
                PathSeg::Quad(s) => (s.p0.distance(s.p1), s.p0.distance(s.p2)),
                PathSeg::Cubic(s) => (s.p0.distance(s.p1), s.p0.distance(s.p3)),
            };
            if chord > 1e-9 {
                values.push(handle / chord);
            }
        }
    }
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n == 0 {
        None
    } else {
        Some((values[(n - 1) / 2] + values[n / 2]) * 0.5)
    }
}
