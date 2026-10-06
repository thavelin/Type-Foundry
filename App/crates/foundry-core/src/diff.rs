//! Which glyphs differ between two fonts, down to the point that moved.

use serde::Serialize;

use crate::font::Font;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GlyphDiff {
    pub name: String,
    pub change: &'static str,
    pub detail: String,
}

/// Compare two fonts. Order follows the first font, then glyphs that exist only in the second.
pub fn diff_fonts(before: &Font, after: &Font) -> Vec<GlyphDiff> {
    let mut diffs = Vec::new();
    for glyph in &before.glyphs {
        let Some(other) = after.glyph(&glyph.name) else {
            diffs.push(GlyphDiff {
                name: glyph.name.clone(),
                change: "removed",
                detail: format!("{} is only in {}", glyph.name, before.name),
            });
            continue;
        };
        if glyph.unicode != other.unicode {
            diffs.push(GlyphDiff {
                name: glyph.name.clone(),
                change: "unicode",
                detail: format!(
                    "{} unicode {:?} became {:?}",
                    glyph.name, glyph.unicode, other.unicode
                ),
            });
        }
        if (glyph.advance - other.advance).abs() > 0.01 {
            diffs.push(GlyphDiff {
                name: glyph.name.clone(),
                change: "advance",
                detail: format!(
                    "{} advance {:.1} became {:.1}",
                    glyph.name, glyph.advance, other.advance
                ),
            });
        }
        if glyph.contours.len() != other.contours.len() {
            diffs.push(GlyphDiff {
                name: glyph.name.clone(),
                change: "contours",
                detail: format!(
                    "{} contour count {} became {}",
                    glyph.name,
                    glyph.contours.len(),
                    other.contours.len()
                ),
            });
            continue;
        }
        for (contour_index, (left, right)) in glyph.contours.iter().zip(&other.contours).enumerate()
        {
            if left.points.len() != right.points.len() {
                diffs.push(GlyphDiff {
                    name: glyph.name.clone(),
                    change: "points",
                    detail: format!(
                        "{} contour {contour_index} point count {} became {}",
                        glyph.name,
                        left.points.len(),
                        right.points.len()
                    ),
                });
                continue;
            }
            for (point_index, (a, b)) in left.points.iter().zip(&right.points).enumerate() {
                if (a.x - b.x).abs() > 0.01 || (a.y - b.y).abs() > 0.01 || a.kind != b.kind {
                    diffs.push(GlyphDiff {
                        name: glyph.name.clone(),
                        change: "point",
                        detail: format!(
                            "{} contour {contour_index} point {point_index} moved from {:.1},{:.1} to {:.1},{:.1}",
                            glyph.name, a.x, a.y, b.x, b.y
                        ),
                    });
                }
            }
        }
    }
    for glyph in &after.glyphs {
        if before.glyph(&glyph.name).is_none() {
            diffs.push(GlyphDiff {
                name: glyph.name.clone(),
                change: "added",
                detail: format!("{} is only in {}", glyph.name, after.name),
            });
        }
    }
    diffs
}
