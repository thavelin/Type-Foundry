//! Ranked Foundry critiques from audit / Style Genome signals.
//! Deterministic suggestions with confidence; accept/reject becomes a DesignDecision.
//! No external model call — the flywheel starts from executable checks.

use serde::{Deserialize, Serialize};

use crate::error::FoundryError;
use crate::font::Font;
use crate::genome::{DesignDecision, GenomeIssue, check_genome, record_decision};
use crate::outline::{OutlineIssue, SpacingIssue, check_outlines, check_spacing};

/// Which audit layer produced the suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CritiqueLayer {
    Design,
    Technical,
}

/// One ranked suggestion Troy can accept or reject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CritiqueSuggestion {
    /// Stable id for this issue in this font state (`layer:code:glyph…`).
    pub id: String,
    /// 1 = highest priority after ranking.
    pub rank: u32,
    /// 0..1 confidence that the intervention is worth considering.
    pub confidence: f64,
    pub layer: CritiqueLayer,
    pub issue: String,
    pub observation: String,
    pub intervention: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glyphs: Vec<String>,
}

/// Build ranked critiques from technical outline/spacing checks and Style Genome design issues.
pub fn critique_font(font: &Font, min_gap: f64) -> Result<Vec<CritiqueSuggestion>, FoundryError> {
    let mut raw = Vec::new();
    for issue in check_outlines(font) {
        raw.push(from_outline(&issue));
    }
    for issue in check_spacing(font, min_gap, None)? {
        raw.push(from_spacing(&issue));
    }
    for issue in check_genome(font)? {
        raw.push(from_genome(&issue));
    }
    raw.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    filter_rejected(font, &mut raw);
    for (index, suggestion) in raw.iter_mut().enumerate() {
        suggestion.rank = (index + 1) as u32;
    }
    Ok(raw)
}

/// Record Troy's accept/reject as a DesignDecision on `font.lib`.
/// Caller-supplied observation/intervention/glyphs/confidence win when non-empty.
pub fn resolve_critique(
    font: &mut Font,
    suggestion: &CritiqueSuggestion,
    accepted: bool,
) -> Result<usize, FoundryError> {
    resolve_critique_with(font, suggestion, accepted, None, None, None, None)
}

/// Like [`resolve_critique`], with optional caller overrides.
pub fn resolve_critique_with(
    font: &mut Font,
    suggestion: &CritiqueSuggestion,
    accepted: bool,
    observation: Option<String>,
    intervention: Option<String>,
    glyphs: Option<Vec<String>>,
    confidence: Option<f64>,
) -> Result<usize, FoundryError> {
    let count = record_decision(
        font,
        DesignDecision {
            scope: match suggestion.layer {
                CritiqueLayer::Design => "design critique".into(),
                CritiqueLayer::Technical => "technical critique".into(),
            },
            glyphs: glyphs.unwrap_or_else(|| suggestion.glyphs.clone()),
            issue: suggestion.issue.clone(),
            observation: observation
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| suggestion.observation.clone()),
            intervention: intervention
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| suggestion.intervention.clone()),
            accepted,
            confidence: confidence.unwrap_or(suggestion.confidence),
        },
    )?;
    // Suppress rejected ids until the glyph changes: store a hash beside decisions.
    if !accepted {
        let key = "com.typefoundry.rejectedCritique";
        let mut rejected: serde_json::Map<String, serde_json::Value> = font
            .lib
            .get(key)
            .and_then(|value| value.as_object())
            .cloned()
            .unwrap_or_default();
        let hash = glyph_hash_for(font, &suggestion.glyphs);
        rejected.insert(suggestion.id.clone(), serde_json::json!(hash));
        font.lib
            .insert(key.to_string(), serde_json::Value::Object(rejected));
    }
    Ok(count)
}

fn glyph_hash_for(font: &Font, glyphs: &[String]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    for name in glyphs {
        if let Some(glyph) = font.glyph(name) {
            glyph.name.hash(&mut hasher);
            for contour in &glyph.contours {
                for point in &contour.points {
                    point.x.to_bits().hash(&mut hasher);
                    point.y.to_bits().hash(&mut hasher);
                }
            }
        }
    }
    format!("{:x}", hasher.finish())
}

/// Drop suggestions whose id was rejected and whose glyphs are unchanged.
pub fn filter_rejected(font: &Font, suggestions: &mut Vec<CritiqueSuggestion>) {
    let Some(rejected) = font
        .lib
        .get("com.typefoundry.rejectedCritique")
        .and_then(|value| value.as_object())
    else {
        return;
    };
    suggestions.retain(|suggestion| {
        let Some(stored) = rejected.get(&suggestion.id).and_then(|v| v.as_str()) else {
            return true;
        };
        let hash = glyph_hash_for(font, &suggestion.glyphs);
        stored != hash
    });
}

fn from_outline(issue: &OutlineIssue) -> CritiqueSuggestion {
    let (confidence, intervention) = match issue.code {
        "direction" => (
            0.92,
            "Reverse the contour so outer paths are CCW and holes CW.".to_string(),
        ),
        "intersection" => (
            0.90,
            "Remove the self-intersection; rebuild the overlapping segment.".to_string(),
        ),
        "kink" => (
            0.78,
            "Smooth the join or make it a deliberate corner.".to_string(),
        ),
        "overshoot" => (
            0.66,
            "Add a small overshoot past the flat metric at the extreme.".to_string(),
        ),
        "baseline" => (
            0.72,
            "Sit the glyph on the baseline (or mark the float as intentional later).".to_string(),
        ),
        other => (
            0.55,
            format!("Review outline issue `{other}` and correct the path."),
        ),
    };
    CritiqueSuggestion {
        id: format!(
            "technical:{}:{}:{}:{}",
            issue.code,
            issue.glyph,
            issue.contour.map(|c| c.to_string()).unwrap_or_default(),
            issue.point.map(|p| p.to_string()).unwrap_or_default()
        ),
        rank: 0,
        confidence,
        layer: CritiqueLayer::Technical,
        issue: issue.code.to_string(),
        observation: issue.detail.clone(),
        intervention,
        glyphs: vec![issue.glyph.clone()],
    }
}

fn from_spacing(issue: &SpacingIssue) -> CritiqueSuggestion {
    let glyphs: Vec<String> = [issue.left.as_ref(), issue.right.as_ref()]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    let (confidence, intervention) = match issue.code {
        "collision" => (
            0.82,
            "Open the pair gap (sidebearing or kern) above the minimum.".to_string(),
        ),
        "bearing" => (
            0.62,
            "Adjust the flagged sidebearing toward family rhythm.".to_string(),
        ),
        "missing" => (
            0.50,
            "Add the missing glyph or drop the pair from the spacing check.".to_string(),
        ),
        other => (0.50, format!("Review spacing issue `{other}`.")),
    };
    let id_glyphs = glyphs.join("+");
    CritiqueSuggestion {
        id: format!("technical:{}:{}", issue.code, id_glyphs),
        rank: 0,
        confidence,
        layer: CritiqueLayer::Technical,
        issue: issue.code.to_string(),
        observation: issue.detail.clone(),
        intervention,
        glyphs,
    }
}

fn from_genome(issue: &GenomeIssue) -> CritiqueSuggestion {
    let glyphs = issue.glyph.clone().into_iter().collect::<Vec<_>>();
    let (confidence, intervention) = match issue.code.as_str() {
        "missing_genome" => (
            0.95,
            "Capture a Style Genome so design checks have a target.".to_string(),
        ),
        "no_stem" => (
            0.70,
            "Add measurable stems on sample glyphs (H, I, N…) then re-capture.".to_string(),
        ),
        "stem" => (
            0.88,
            "Bring this glyph's primary stem toward the genome median (within tolerance)."
                .to_string(),
        ),
        other => (0.60, format!("Address design genome issue `{other}`.")),
    };
    CritiqueSuggestion {
        id: format!(
            "design:{}:{}",
            issue.code,
            issue.glyph.as_deref().unwrap_or("")
        ),
        rank: 0,
        confidence,
        layer: CritiqueLayer::Design,
        issue: issue.code.clone(),
        observation: issue.detail.clone(),
        intervention,
        glyphs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{Contour, Glyph, Point, PointKind};
    use crate::genome::capture_genome;

    fn on(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            kind: PointKind::On,
            smooth: false,
        }
    }

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

    #[test]
    fn missing_genome_ranks_first_with_high_confidence() {
        let mut font = Font::new("Critique", 1000).unwrap();
        font.insert_glyph(h_like()).unwrap();
        let suggestions = critique_font(&font, 0.0).unwrap();
        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].rank, 1);
        assert_eq!(suggestions[0].issue, "missing_genome");
        assert!(suggestions[0].confidence >= 0.9);
    }

    #[test]
    fn stem_outlier_becomes_a_design_critique() {
        let mut font = Font::new("Critique", 1000).unwrap();
        font.insert_glyph(h_like()).unwrap();
        capture_genome(&mut font, 2.0).unwrap();
        let glyph = font.glyph_mut("H").unwrap();
        glyph.contours[0].points[1].x = 40.0;
        glyph.contours[0].points[2].x = 40.0;
        let suggestions = critique_font(&font, 0.0).unwrap();
        let stem = suggestions
            .iter()
            .find(|s| s.issue == "stem")
            .expect("stem critique");
        assert_eq!(stem.layer, CritiqueLayer::Design);
        assert!(stem.confidence >= 0.8);
        assert!(stem.glyphs.contains(&"H".to_string()));
    }

    #[test]
    fn resolve_records_a_design_decision() {
        let mut font = Font::new("Critique", 1000).unwrap();
        font.insert_glyph(h_like()).unwrap();
        let suggestions = critique_font(&font, 0.0).unwrap();
        let first = suggestions[0].clone();
        let count = resolve_critique(&mut font, &first, true).unwrap();
        assert_eq!(count, 1);
        let listed = crate::genome::list_decisions(&font).unwrap();
        assert!(listed[0].accepted);
        assert_eq!(listed[0].issue, first.issue);
    }
}
