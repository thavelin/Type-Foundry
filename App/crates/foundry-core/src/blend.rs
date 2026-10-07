use std::collections::BTreeSet;

use serde::Serialize;

use crate::font::{Font, Glyph, Metrics, Point};

/// One reason two fonts cannot be blended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompatIssue {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contour: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point: Option<usize>,
    pub detail: String,
}

/// Linear blend. `t` of 0 returns the shape of `a`, `t` of 1 returns the shape of `b`.
pub fn blend_fonts(a: &Font, b: &Font, t: f64) -> Result<Font, Vec<CompatIssue>> {
    if !t.is_finite() {
        return Err(vec![CompatIssue {
            code: "t",
            glyph: None,
            contour: None,
            point: None,
            detail: "blend amount must be a finite number".to_string(),
        }]);
    }
    let issues = compatibility(a, b);
    if !issues.is_empty() {
        return Err(issues);
    }
    Ok(interpolate(a, b, t))
}

pub fn compatibility(a: &Font, b: &Font) -> Vec<CompatIssue> {
    let mut issues = Vec::new();
    if a.upm != b.upm {
        issues.push(CompatIssue {
            code: "upm",
            glyph: None,
            contour: None,
            point: None,
            detail: format!("units per em differ: {} and {}", a.upm, b.upm),
        });
    }

    let names_a: BTreeSet<&str> = a.glyphs.iter().map(|glyph| glyph.name.as_str()).collect();
    let names_b: BTreeSet<&str> = b.glyphs.iter().map(|glyph| glyph.name.as_str()).collect();
    for name in names_a.difference(&names_b) {
        issues.push(CompatIssue {
            code: "missing",
            glyph: Some((*name).to_string()),
            contour: None,
            point: None,
            detail: format!("{name} is missing from {}", b.name),
        });
    }
    for name in names_b.difference(&names_a) {
        issues.push(CompatIssue {
            code: "missing",
            glyph: Some((*name).to_string()),
            contour: None,
            point: None,
            detail: format!("{name} is missing from {}", a.name),
        });
    }

    for glyph_a in &a.glyphs {
        let Some(glyph_b) = b.glyph(&glyph_a.name) else {
            continue;
        };
        issues.extend(glyph_issues(glyph_a, glyph_b));
    }
    issues
}

fn glyph_issues(a: &Glyph, b: &Glyph) -> Vec<CompatIssue> {
    let mut issues = Vec::new();
    if a.unicode != b.unicode {
        issues.push(CompatIssue {
            code: "unicode",
            glyph: Some(a.name.clone()),
            contour: None,
            point: None,
            detail: format!("unicode differs: {:?} and {:?}", a.unicode, b.unicode),
        });
    }
    if a.contours.len() != b.contours.len() {
        issues.push(CompatIssue {
            code: "contours",
            glyph: Some(a.name.clone()),
            contour: None,
            point: None,
            detail: format!(
                "contour count differs: {} and {}",
                a.contours.len(),
                b.contours.len()
            ),
        });
        return issues;
    }
    for (index, (left, right)) in a.contours.iter().zip(b.contours.iter()).enumerate() {
        if left.closed != right.closed {
            issues.push(CompatIssue {
                code: "closed",
                glyph: Some(a.name.clone()),
                contour: Some(index),
                point: None,
                detail: format!("contour {index} closed flag differs"),
            });
        }
        if left.points.len() != right.points.len() {
            issues.push(CompatIssue {
                code: "points",
                glyph: Some(a.name.clone()),
                contour: Some(index),
                point: None,
                detail: format!(
                    "contour {index} point count differs: {} and {}",
                    left.points.len(),
                    right.points.len()
                ),
            });
            continue;
        }
        for (point_index, (point_a, point_b)) in
            left.points.iter().zip(right.points.iter()).enumerate()
        {
            if point_a.kind != point_b.kind {
                issues.push(CompatIssue {
                    code: "kind",
                    glyph: Some(a.name.clone()),
                    contour: Some(index),
                    point: Some(point_index),
                    detail: format!("contour {index} point {point_index} kind differs"),
                });
            }
            if point_a.smooth != point_b.smooth {
                issues.push(CompatIssue {
                    code: "smooth",
                    glyph: Some(a.name.clone()),
                    contour: Some(index),
                    point: Some(point_index),
                    detail: format!("contour {index} point {point_index} smooth flag differs"),
                });
            }
        }
    }
    issues
}

fn interpolate(a: &Font, b: &Font, t: f64) -> Font {
    let glyphs = a
        .glyphs
        .iter()
        .map(|glyph_a| {
            let glyph_b = b
                .glyph(&glyph_a.name)
                .expect("compatibility already checked");
            interpolate_glyph(glyph_a, glyph_b, t)
        })
        .collect();
    Font {
        format: crate::FONT_FORMAT.to_string(),
        version: crate::FONT_VERSION,
        name: format!("{} / {} @ {}", a.name, b.name, format_t(t)),
        upm: a.upm,
        metrics: interpolate_metrics(&a.metrics, &b.metrics, t),
        style: crate::Style {
            family: if a.style.family == b.style.family {
                a.style.family.clone()
            } else {
                format!("{} / {} @ {}", a.name, b.name, format_t(t))
            },
            name: if a.style.name == b.style.name {
                a.style.name.clone()
            } else {
                format!("{} / {} @ {}", a.style.name, b.style.name, format_t(t))
            },
            weight: lerp(f64::from(a.style.weight), f64::from(b.style.weight), t)
                .round()
                .clamp(1.0, 1000.0) as u16,
            italic: a.style.italic,
            italic_angle: lerp(a.style.italic_angle, b.style.italic_angle, t),
            width: lerp(f64::from(a.style.width), f64::from(b.style.width), t)
                .round()
                .clamp(1.0, 9.0) as u16,
        },
        info: a.info.clone(),
        kerning: a.kerning.clone(),
        features: a.features.clone(),
        lib: a.lib.clone(),
        glyphs,
    }
}

fn interpolate_glyph(a: &Glyph, b: &Glyph, t: f64) -> Glyph {
    Glyph {
        name: a.name.clone(),
        unicode: a.unicode,
        advance: lerp(a.advance, b.advance, t),
        contours: a
            .contours
            .iter()
            .zip(b.contours.iter())
            .map(|(left, right)| crate::Contour {
                closed: left.closed,
                points: left
                    .points
                    .iter()
                    .zip(right.points.iter())
                    .map(|(point_a, point_b)| Point {
                        x: lerp(point_a.x, point_b.x, t),
                        y: lerp(point_a.y, point_b.y, t),
                        kind: point_a.kind,
                        smooth: point_a.smooth,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn interpolate_metrics(a: &Metrics, b: &Metrics, t: f64) -> Metrics {
    Metrics {
        ascender: lerp(a.ascender, b.ascender, t),
        cap_height: lerp(a.cap_height, b.cap_height, t),
        x_height: lerp(a.x_height, b.x_height, t),
        baseline: lerp(a.baseline, b.baseline, t),
        descender: lerp(a.descender, b.descender, t),
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn format_t(t: f64) -> String {
    let rendered = format!("{t:.4}");
    let trimmed = rendered.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Contour, PointKind};

    fn square(name: &str, unicode: u32, x: f64, advance: f64) -> Glyph {
        Glyph {
            name: name.to_string(),
            unicode: Some(unicode),
            advance,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    point(x, 0.0),
                    point(x + 100.0, 0.0),
                    point(x + 100.0, 100.0),
                    point(x, 100.0),
                ],
            }],
        }
    }

    fn point(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            kind: PointKind::On,
            smooth: false,
        }
    }

    fn face(name: &str, glyph: Glyph) -> Font {
        let mut font = Font::new(name, 1000).unwrap();
        font.insert_glyph(glyph).unwrap();
        font
    }

    #[test]
    fn midpoint_averages_coordinates_advance_and_metrics() {
        let mut narrow = face("Narrow", square("H", 72, 100.0, 400.0));
        narrow.metrics.ascender = 800.0;
        let mut wide = face("Wide", square("H", 72, 200.0, 800.0));
        wide.metrics.ascender = 900.0;

        let mid = blend_fonts(&narrow, &wide, 0.5).unwrap();

        assert_eq!(mid.name, "Narrow / Wide @ 0.5");
        assert_eq!(mid.upm, 1000);
        assert_eq!(mid.metrics.ascender, 850.0);
        let glyph = mid.glyph("H").unwrap();
        assert_eq!(glyph.advance, 600.0);
        assert_eq!(glyph.contours[0].points[0].x, 150.0);
        assert_eq!(glyph.contours[0].points[0].y, 0.0);
    }

    #[test]
    fn refuses_a_different_point_structure() {
        let narrow = face("Narrow", square("H", 72, 0.0, 400.0));
        let mut other = square("H", 72, 0.0, 800.0);
        other.contours[0].points.pop();
        let wide = face("Wide", other);

        let issues = blend_fonts(&narrow, &wide, 0.5).unwrap_err();
        assert!(issues.iter().any(|issue| issue.code == "points"));
    }

    #[test]
    fn refuses_a_missing_glyph_and_a_bad_blend_amount() {
        let narrow = face("Narrow", square("H", 72, 0.0, 400.0));
        let wide = face("Wide", square("O", 79, 0.0, 800.0));
        let issues = compatibility(&narrow, &wide);
        assert!(
            issues
                .iter()
                .any(|issue| issue.glyph.as_deref() == Some("H"))
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.glyph.as_deref() == Some("O"))
        );

        let bad = blend_fonts(&narrow, &narrow, f64::NAN).unwrap_err();
        assert_eq!(bad[0].code, "t");
    }
}
