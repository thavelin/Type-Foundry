//! Style Genome measurements, genome capture/check, and DesignDecision storage in `Font.lib`.
//! Stems use the same vertical-edge clustering as `scale_width` — not a ray caster.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::edit::glyph_stem_widths;
use crate::error::FoundryError;
use crate::font::{Font, Glyph, Metrics};
use crate::outline::{OutlineIssue, SpacingIssue, check_outlines, check_spacing};

/// Lib key for the captured Style Genome JSON object.
pub const GENOME_KEY: &str = "com.typefoundry.styleGenome";
/// Lib key for the DesignDecision array.
pub const DECISIONS_KEY: &str = "com.typefoundry.designDecisions";

/// Glyphs preferred when capturing a stem/sidebearing genome. Missing names are skipped.
pub const STEM_SAMPLES: &[&str] = &["H", "I", "N", "P", "R", "h", "n", "u"];

/// One glyph's live measurements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlyphMeasure {
    pub name: String,
    pub advance: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rsb: Option<f64>,
    /// Vertical stem widths, outer-to-inner pairs.
    pub stems: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_stem: Option<f64>,
}

/// Captured Style Genome: family expectations that `check_genome` / `audit` enforce.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleGenome {
    pub metrics: Metrics,
    /// Median primary stem from the sample glyphs that had measurable stems.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_stem: Option<f64>,
    /// Absolute units allowed away from `primary_stem` before a design warning.
    pub stem_tolerance: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_sidebearing: Option<f64>,
    /// Glyphs that contributed to the genome when it was captured.
    pub samples: Vec<String>,
}

/// A meaningful revision choice, stored for later learning. Not inferred yet — only recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesignDecision {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glyphs: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub issue: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub observation: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub intervention: String,
    pub accepted: bool,
    #[serde(default)]
    pub confidence: f64,
}

/// Design-layer issue from comparing the open font to a captured genome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenomeIssue {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    pub detail: String,
}

/// Live measurements for the active font: vertical metrics plus per-glyph stems/sidebearings.
pub fn measure_font(font: &Font) -> Value {
    let glyphs: Vec<GlyphMeasure> = font.glyphs.iter().map(measure_glyph).collect();
    let stem_values: Vec<f64> = glyphs
        .iter()
        .filter(|g| STEM_SAMPLES.contains(&g.name.as_str()))
        .filter_map(|g| g.primary_stem)
        .collect();
    let sidebearings: Vec<f64> = glyphs
        .iter()
        .filter(|g| STEM_SAMPLES.contains(&g.name.as_str()))
        .flat_map(|g| [g.lsb, g.rsb].into_iter().flatten())
        .collect();
    json!({
        "name": font.name,
        "upm": font.upm,
        "metrics": font.metrics,
        "primary_stem": median(&stem_values),
        "median_sidebearing": median(&sidebearings),
        "glyphs": glyphs,
        "count": glyphs.len(),
    })
}

/// Build a Style Genome from the open font and write it into `font.lib`.
pub fn capture_genome(font: &mut Font, stem_tolerance: f64) -> Result<StyleGenome, FoundryError> {
    if !(stem_tolerance.is_finite() && stem_tolerance >= 0.0) {
        return Err(FoundryError::Json(
            "stem_tolerance must be a finite number ≥ 0".into(),
        ));
    }
    let mut stems = Vec::new();
    let mut sides = Vec::new();
    let mut samples = Vec::new();
    for name in STEM_SAMPLES {
        let Some(glyph) = font.glyph(name) else {
            continue;
        };
        let measured = measure_glyph(glyph);
        if measured.primary_stem.is_some() || measured.lsb.is_some() {
            samples.push((*name).to_string());
        }
        if let Some(stem) = measured.primary_stem {
            stems.push(stem);
        }
        if let Some(lsb) = measured.lsb {
            sides.push(lsb);
        }
        if let Some(rsb) = measured.rsb {
            sides.push(rsb);
        }
    }
    let genome = StyleGenome {
        metrics: font.metrics.clone(),
        primary_stem: median(&stems),
        stem_tolerance,
        median_sidebearing: median(&sides),
        samples,
    };
    let value = serde_json::to_value(&genome).map_err(|err| FoundryError::Json(err.to_string()))?;
    font.lib.insert(GENOME_KEY.to_string(), value);
    Ok(genome)
}

/// Read the stored genome, if any.
pub fn stored_genome(font: &Font) -> Result<Option<StyleGenome>, FoundryError> {
    let Some(value) = font.lib.get(GENOME_KEY) else {
        return Ok(None);
    };
    let genome = serde_json::from_value(value.clone())
        .map_err(|err| FoundryError::Json(format!("style genome in lib is broken: {err}")))?;
    Ok(Some(genome))
}

/// Compare live stems on sample glyphs to the captured genome.
pub fn check_genome(font: &Font) -> Result<Vec<GenomeIssue>, FoundryError> {
    let Some(genome) = stored_genome(font)? else {
        return Ok(vec![GenomeIssue {
            code: "missing_genome".into(),
            glyph: None,
            detail: "No Style Genome in font.lib. Run capture_genome first.".into(),
        }]);
    };
    let Some(expected) = genome.primary_stem else {
        return Ok(vec![GenomeIssue {
            code: "no_stem".into(),
            glyph: None,
            detail: "Captured genome has no primary stem (no measurable sample glyphs).".into(),
        }]);
    };
    let mut issues = Vec::new();
    let names: Vec<&str> = if genome.samples.is_empty() {
        STEM_SAMPLES.to_vec()
    } else {
        genome.samples.iter().map(String::as_str).collect()
    };
    for name in names {
        let Some(glyph) = font.glyph(name) else {
            continue;
        };
        let measured = measure_glyph(glyph);
        let Some(stem) = measured.primary_stem else {
            continue;
        };
        let delta = (stem - expected).abs();
        if delta > genome.stem_tolerance {
            let pct = if expected.abs() > f64::EPSILON {
                delta / expected.abs() * 100.0
            } else {
                0.0
            };
            issues.push(GenomeIssue {
                code: "stem".into(),
                glyph: Some(name.to_string()),
                detail: format!(
                    "primary stem {stem:.1} is {delta:.1} units ({pct:.1}%) from genome {expected:.1} (tolerance {})",
                    genome.stem_tolerance
                ),
            });
        }
    }
    Ok(issues)
}

/// Technical outline/spacing checks plus design genome deviations.
pub fn audit_font(font: &Font, min_gap: f64) -> Result<Value, FoundryError> {
    let technical_outlines: Vec<OutlineIssue> = check_outlines(font);
    let technical_spacing: Vec<SpacingIssue> = check_spacing(font, min_gap, None)?;
    let design = check_genome(font)?;
    let technical_count = technical_outlines.len() + technical_spacing.len();
    Ok(json!({
        "technical": {
            "outlines": technical_outlines,
            "spacing": technical_spacing,
            "count": technical_count,
        },
        "design": {
            "issues": design,
            "count": design.len(),
        },
        "count": technical_count + design.len(),
    }))
}

/// Append a DesignDecision to `font.lib`.
pub fn record_decision(font: &mut Font, decision: DesignDecision) -> Result<usize, FoundryError> {
    let mut list = list_decisions(font)?;
    list.push(decision);
    let value = serde_json::to_value(&list).map_err(|err| FoundryError::Json(err.to_string()))?;
    font.lib.insert(DECISIONS_KEY.to_string(), value);
    Ok(list.len())
}

/// Read DesignDecisions from lib.
pub fn list_decisions(font: &Font) -> Result<Vec<DesignDecision>, FoundryError> {
    let Some(value) = font.lib.get(DECISIONS_KEY) else {
        return Ok(Vec::new());
    };
    serde_json::from_value(value.clone())
        .map_err(|err| FoundryError::Json(format!("design decisions in lib are broken: {err}")))
}

fn measure_glyph(glyph: &Glyph) -> GlyphMeasure {
    let stems = glyph_stem_widths(glyph);
    let primary_stem = stems.first().copied();
    let (lsb, rsb) = sidebearings(glyph);
    GlyphMeasure {
        name: glyph.name.clone(),
        advance: glyph.advance,
        lsb,
        rsb,
        stems,
        primary_stem,
    }
}

fn sidebearings(glyph: &Glyph) -> (Option<f64>, Option<f64>) {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    for contour in &glyph.contours {
        for point in &contour.points {
            min_x = min_x.min(point.x);
            max_x = max_x.max(point.x);
        }
    }
    if !min_x.is_finite() {
        return (None, None);
    }
    (Some(min_x), Some(glyph.advance - max_x))
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    Some(if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{Contour, Glyph, Point, PointKind};

    fn on(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            kind: PointKind::On,
            smooth: false,
        }
    }

    /// Two vertical stems of width 20 at x=0–20 and x=80–100, advance 120.
    fn h_like() -> Glyph {
        Glyph {
            name: "H".into(),
            unicode: Some(0x48),
            advance: 120.0,
            contours: vec![
                Contour {
                    closed: true,
                    points: vec![on(0.0, 0.0), on(20.0, 0.0), on(20.0, 100.0), on(0.0, 100.0)],
                },
                Contour {
                    closed: true,
                    points: vec![
                        on(80.0, 0.0),
                        on(100.0, 0.0),
                        on(100.0, 100.0),
                        on(80.0, 100.0),
                    ],
                },
            ],
        }
    }

    fn font_with_h() -> Font {
        let mut font = Font::new("Genome", 1000).unwrap();
        font.insert_glyph(h_like()).unwrap();
        font
    }

    #[test]
    fn measures_stems_and_sidebearings_on_h() {
        let font = font_with_h();
        let measured = measure_glyph(font.glyph("H").unwrap());
        assert_eq!(measured.primary_stem, Some(20.0));
        assert_eq!(measured.lsb, Some(0.0));
        assert_eq!(measured.rsb, Some(20.0));
    }

    #[test]
    fn capture_and_check_genome_flag_a_heavy_stem() {
        let mut font = font_with_h();
        let genome = capture_genome(&mut font, 2.0).unwrap();
        assert_eq!(genome.primary_stem, Some(20.0));
        assert!(stored_genome(&font).unwrap().is_some());
        assert!(check_genome(&font).unwrap().is_empty());

        // Thicken the first stem to 40 by moving its right edge.
        let glyph = font.glyph_mut("H").unwrap();
        glyph.contours[0].points[1].x = 40.0;
        glyph.contours[0].points[2].x = 40.0;
        let issues = check_genome(&font).unwrap();
        assert!(
            issues.iter().any(|issue| issue.code == "stem"),
            "{issues:?}"
        );
    }

    #[test]
    fn audit_includes_technical_and_design() {
        let mut font = font_with_h();
        capture_genome(&mut font, 2.0).unwrap();
        let report = audit_font(&font, 0.0).unwrap();
        assert!(report["technical"]["count"].as_u64().is_some());
        assert_eq!(report["design"]["count"], 0);
    }

    #[test]
    fn decisions_round_trip_in_lib() {
        let mut font = font_with_h();
        let count = record_decision(
            &mut font,
            DesignDecision {
                scope: "Genome".into(),
                glyphs: vec!["H".into()],
                issue: "stem".into(),
                observation: "stem felt heavy".into(),
                intervention: "kept for weight".into(),
                accepted: true,
                confidence: 0.9,
            },
        )
        .unwrap();
        assert_eq!(count, 1);
        let listed = list_decisions(&font).unwrap();
        assert_eq!(listed[0].issue, "stem");
        assert!(listed[0].accepted);
    }
}
