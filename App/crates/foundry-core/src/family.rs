//! Font families: styles of one design kept as separate fonts, checked against each other, saved
//! together through a family file, and exported with matching names.
//!
//! A family file is JSON, `format` `typefoundry.family`, that lists member font files relative to
//! itself. Members can be any format `Font::load` reads.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::FoundryError;
use crate::font::Font;

pub const FAMILY_FORMAT: &str = "typefoundry.family";
pub const FAMILY_VERSION: u32 = 1;

/// Optional new style values. `None` keeps the current value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleUpdate {
    pub family: Option<String>,
    pub name: Option<String>,
    pub weight: Option<u16>,
    pub italic: Option<bool>,
    pub italic_angle: Option<f64>,
    pub width: Option<u16>,
}

/// Formats a family exports to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Json,
    Ufo,
    Ttf,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Ufo => "ufo",
            Self::Ttf => "ttf",
        }
    }
}

/// One finding from [`check_family`]. `blocking` findings stop an export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FamilyIssue {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    pub detail: String,
    pub blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FamilyFile {
    pub format: String,
    pub version: u32,
    pub family: String,
    /// Member font files, relative to the family file.
    pub styles: Vec<String>,
    /// A version note, such as `v4.6`. Old family files omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Font {
    /// Change the family, style name, weight, or italic settings. A changed family or style name
    /// also renames the font to `Family Style`.
    pub fn set_style(&mut self, update: StyleUpdate) -> Result<(), FoundryError> {
        let mut style = self.style.clone();
        if let Some(family) = update.family {
            if family.trim().is_empty() {
                return Err(FoundryError::Style("the family name is empty".into()));
            }
            style.family = family.trim().to_string();
        }
        if let Some(name) = update.name {
            if name.trim().is_empty() {
                return Err(FoundryError::Style("the style name is empty".into()));
            }
            style.name = name.trim().to_string();
        }
        if let Some(weight) = update.weight {
            if !(1..=1000).contains(&weight) {
                return Err(FoundryError::Style(format!(
                    "weight {weight} is outside 1-1000"
                )));
            }
            style.weight = weight;
        }
        if let Some(italic) = update.italic {
            style.italic = italic;
        }
        if let Some(angle) = update.italic_angle {
            if !angle.is_finite() || angle.abs() >= 90.0 {
                return Err(FoundryError::Style(format!(
                    "italic angle {angle} must be between -90 and 90"
                )));
            }
            style.italic_angle = angle;
        }
        if let Some(width) = update.width {
            if !(1..=9).contains(&width) {
                return Err(FoundryError::Style(format!(
                    "width class {width} is outside 1-9"
                )));
            }
            style.width = width;
        }
        let renamed = style.family != self.style.family || style.name != self.style.name;
        self.style = style;
        if renamed {
            self.name = self.full_name();
        }
        Ok(())
    }

    /// A copy of this font as another style of the same family. A nonzero `slant` leans every
    /// glyph by that many degrees about the baseline and sets the italic angle to match.
    pub fn derive_style(
        &self,
        name: &str,
        weight: Option<u16>,
        italic: Option<bool>,
        slant: f64,
    ) -> Result<Font, FoundryError> {
        if !slant.is_finite() || slant.abs() >= 60.0 {
            return Err(FoundryError::Style(format!(
                "slant {slant} must be between -60 and 60 degrees"
            )));
        }
        let mut derived = self.clone();
        let leaning = slant != 0.0;
        derived.set_style(StyleUpdate {
            name: Some(name.to_string()),
            weight,
            italic: Some(italic.unwrap_or(leaning || self.style.italic)),
            italic_angle: leaning.then_some(self.style.italic_angle - slant),
            ..StyleUpdate::default()
        })?;
        if leaning {
            derived.slant_glyphs_ex(slant, derived.metrics.x_height / 2.0, false)?;
        }
        Ok(derived)
    }

    /// Shear every glyph about `pivot_y`. When `recenter` is true, shift ink back to the advance
    /// centre (legacy behaviour). Production italics use `recenter: false` and `pivot_y` at
    /// `x_height / 2`.
    pub fn slant_glyphs(&mut self, degrees: f64) -> Result<(), FoundryError> {
        self.slant_glyphs_ex(degrees, self.metrics.x_height / 2.0, false)
    }

    pub fn slant_glyphs_ex(
        &mut self,
        degrees: f64,
        pivot_y: f64,
        recenter: bool,
    ) -> Result<(), FoundryError> {
        if !degrees.is_finite() || degrees.abs() >= 60.0 {
            return Err(FoundryError::Style(format!(
                "slant {degrees} must be between -60 and 60 degrees"
            )));
        }
        if !pivot_y.is_finite() {
            return Err(FoundryError::NonFinite);
        }
        if degrees == 0.0 {
            return Ok(());
        }
        let shear = degrees.to_radians().tan();
        for glyph in &mut self.glyphs {
            let before = ink_center_x(glyph);
            for contour in &mut glyph.contours {
                for point in &mut contour.points {
                    point.x += (point.y - pivot_y) * shear;
                }
            }
            if recenter && let (Some(before), Some(after)) = (before, ink_center_x(glyph)) {
                let dx = before - after;
                for contour in &mut glyph.contours {
                    for point in &mut contour.points {
                        point.x += dx;
                    }
                }
            }
        }
        Ok(())
    }

    /// Slant the open font in place, mark it italic, and store the new italic angle.
    pub fn slant(&mut self, degrees: f64) -> Result<(), FoundryError> {
        self.slant_ex(degrees, self.metrics.x_height / 2.0, false)
    }

    pub fn slant_ex(
        &mut self,
        degrees: f64,
        pivot_y: f64,
        recenter: bool,
    ) -> Result<(), FoundryError> {
        self.slant_glyphs_ex(degrees, pivot_y, recenter)?;
        if degrees != 0.0 {
            self.style.italic = true;
            self.style.italic_angle -= degrees;
        }
        Ok(())
    }
}

fn ink_center_x(glyph: &crate::font::Glyph) -> Option<f64> {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    for contour in &glyph.contours {
        for point in &contour.points {
            min_x = min_x.min(point.x);
            max_x = max_x.max(point.x);
        }
    }
    if min_x.is_finite() {
        Some((min_x + max_x) / 2.0)
    } else {
        None
    }
}

/// Check styles meant to ship as one family. Blocking issues are ones that would make exported
/// files collide or not group; the rest are worth knowing before release.
pub fn check_family(fonts: &[&Font]) -> Vec<FamilyIssue> {
    let mut issues = Vec::new();
    let Some(first) = fonts.first() else {
        return issues;
    };
    let issue =
        |code, style: Option<&str>, glyph: Option<&str>, detail: String, blocking| FamilyIssue {
            code,
            style: style.map(str::to_string),
            glyph: glyph.map(str::to_string),
            detail,
            blocking,
        };

    let families: BTreeSet<&str> = fonts.iter().map(|f| f.style.family.as_str()).collect();
    if families.len() > 1 {
        let names: Vec<&str> = families.into_iter().collect();
        issues.push(issue(
            "family",
            None,
            None,
            format!("styles name different families: {}", names.join(", ")),
            true,
        ));
    }

    let mut stems: BTreeMap<String, &str> = BTreeMap::new();
    let mut slots: BTreeMap<(u16, bool), &str> = BTreeMap::new();
    for font in fonts {
        let style = font.style.name.as_str();
        if let Some(other) = stems.insert(font.file_stem(), style) {
            issues.push(issue(
                "style",
                Some(style),
                None,
                format!("{style} and {other} would export to the same file name"),
                true,
            ));
        }
        if let Some(other) = slots.insert((font.style.weight, font.style.italic), style) {
            issues.push(issue(
                "slot",
                Some(style),
                None,
                format!(
                    "{style} and {other} share weight {} and italic {}, so apps may show only one",
                    font.style.weight, font.style.italic
                ),
                false,
            ));
        }
        if font.upm != first.upm {
            issues.push(issue(
                "upm",
                Some(style),
                None,
                format!(
                    "{style} has {} units per em, {} has {}",
                    font.upm, first.style.name, first.upm
                ),
                false,
            ));
        }
        if font.metrics.ascender != first.metrics.ascender
            || font.metrics.descender != first.metrics.descender
        {
            issues.push(issue(
                "metrics",
                Some(style),
                None,
                format!(
                    "{style} has different ascender or descender from {}, so line spacing will differ",
                    first.style.name
                ),
                false,
            ));
        }
        if font.style.italic && font.style.italic_angle == 0.0 {
            issues.push(issue(
                "angle",
                Some(style),
                None,
                format!("{style} is italic but its italic angle is 0"),
                false,
            ));
        }
    }

    // Glyph coverage: every glyph any style has, every style should have, with one Unicode.
    let mut everywhere: BTreeMap<&str, BTreeMap<&str, Option<u32>>> = BTreeMap::new();
    for font in fonts {
        for glyph in &font.glyphs {
            everywhere
                .entry(glyph.name.as_str())
                .or_default()
                .insert(font.style.name.as_str(), glyph.unicode);
        }
    }
    for (glyph, present) in &everywhere {
        for font in fonts {
            if !present.contains_key(font.style.name.as_str()) {
                issues.push(issue(
                    "missing",
                    Some(&font.style.name),
                    Some(glyph),
                    format!("{glyph} is missing from {}", font.style.name),
                    false,
                ));
            }
        }
        let codes: BTreeSet<Option<u32>> = present.values().copied().collect();
        if codes.len() > 1 {
            issues.push(issue(
                "unicode",
                None,
                Some(glyph),
                format!("{glyph} has different Unicode values across styles"),
                false,
            ));
        }
    }
    issues
}

fn blocking_message(issues: &[FamilyIssue]) -> Option<String> {
    let blocking: Vec<&str> = issues
        .iter()
        .filter(|issue| issue.blocking)
        .map(|issue| issue.detail.as_str())
        .collect();
    (!blocking.is_empty()).then(|| blocking.join("; "))
}

/// Write each style as `Family-Style.ext` in `dir`. Refuses when the family check finds a
/// blocking issue, before writing anything. Returns the files written, in input order.
pub fn export_family(
    fonts: &[&Font],
    dir: &Path,
    format: ExportFormat,
    force: bool,
) -> Result<Vec<PathBuf>, FoundryError> {
    if fonts.is_empty() {
        return Err(FoundryError::Family("there are no styles to export".into()));
    }
    if let Some(message) = blocking_message(&check_family(fonts)) {
        return Err(FoundryError::Family(message));
    }
    let paths: Vec<PathBuf> = fonts
        .iter()
        .map(|font| dir.join(format!("{}.{}", font.file_stem(), format.extension())))
        .collect();
    if !force {
        for path in &paths {
            if path.exists() {
                return Err(FoundryError::Exists(path.display().to_string()));
            }
        }
    }
    fs::create_dir_all(dir).map_err(|err| FoundryError::Io(err.to_string()))?;
    for (font, path) in fonts.iter().zip(&paths) {
        font.save_with(path, force)?;
    }
    Ok(paths)
}

/// Save every style as JSON beside the family file, then the family file listing them.
pub fn save_family(
    fonts: &[&Font],
    path: &Path,
    force: bool,
) -> Result<Vec<PathBuf>, FoundryError> {
    save_family_labeled(fonts, path, None, force)
}

fn save_family_labeled(
    fonts: &[&Font],
    path: &Path,
    label: Option<String>,
    force: bool,
) -> Result<Vec<PathBuf>, FoundryError> {
    if fonts.is_empty() {
        return Err(FoundryError::Family("there are no styles to save".into()));
    }
    if path.exists() && !force {
        return Err(FoundryError::Exists(path.display().to_string()));
    }
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let written = export_family(fonts, &dir, ExportFormat::Json, force)?;
    let file = FamilyFile {
        format: FAMILY_FORMAT.to_string(),
        version: FAMILY_VERSION,
        family: fonts[0].style.family.clone(),
        styles: written
            .iter()
            .filter_map(|member| member.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect(),
        label,
    };
    crate::save::prepare_write(path, force)?;
    let mut text =
        serde_json::to_string_pretty(&file).map_err(|err| FoundryError::Json(err.to_string()))?;
    text.push('\n');
    fs::write(path, text).map_err(|err| FoundryError::Io(err.to_string()))?;
    Ok(written)
}

/// Copy a family into `dest_dir`. `family`, when set, renames every style's family. `label` is
/// stored on the new family file, so v4.6 can be made from v4.5 in one step.
pub fn copy_family(
    source: &Path,
    dest_dir: &Path,
    family: Option<&str>,
    label: Option<&str>,
    force: bool,
) -> Result<PathBuf, FoundryError> {
    let (file, members) = load_family(source)?;
    let mut fonts: Vec<Font> = members.into_iter().map(|(_, font)| font).collect();
    if let Some(family) = family {
        for font in &mut fonts {
            font.set_style(StyleUpdate {
                family: Some(family.to_string()),
                ..StyleUpdate::default()
            })?;
        }
    }
    fs::create_dir_all(dest_dir).map_err(|err| FoundryError::Io(err.to_string()))?;
    let family_label = fonts
        .first()
        .map(|font| font.style.family.clone())
        .unwrap_or_else(|| "family".into());
    let safe: String = family_label
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    let dest = dest_dir.join(format!("{safe}.family.json"));
    let refs: Vec<&Font> = fonts.iter().collect();
    let stored_label = label
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .or(file.label);
    save_family_labeled(&refs, &dest, stored_label, force)?;
    Ok(dest)
}

/// Read a family file and every style it lists. Fails on the first style that does not load.
pub fn load_family(path: &Path) -> Result<(FamilyFile, Vec<(PathBuf, Font)>), FoundryError> {
    let text = fs::read_to_string(path).map_err(|err| FoundryError::Io(err.to_string()))?;
    let file: FamilyFile =
        serde_json::from_str(&text).map_err(|err| FoundryError::Json(err.to_string()))?;
    if file.format != FAMILY_FORMAT {
        return Err(FoundryError::Format(file.format));
    }
    if file.version != FAMILY_VERSION {
        return Err(FoundryError::Version(file.version));
    }
    if file.styles.is_empty() {
        return Err(FoundryError::Family(
            "the family file lists no styles".into(),
        ));
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut members = Vec::new();
    for style in &file.styles {
        let member = dir.join(style);
        let font = Font::load(&member)
            .map_err(|err| FoundryError::Family(format!("{}: {err}", member.display())))?;
        members.push((member, font));
    }
    Ok((file, members))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{Contour, Glyph, Point, PointKind};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_IDS: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = TEMP_IDS.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("typefoundry-family-{tick}-{id}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn file_names_keep_version_dots() {
        let mut font = Font::new("Vostok", 1000).unwrap();
        font.set_style(StyleUpdate {
            family: Some("VostokSerifv4.5".into()),
            name: Some("Regular".into()),
            ..StyleUpdate::default()
        })
        .unwrap();
        assert_eq!(font.file_stem(), "VostokSerifv4.5-Regular");
    }

    fn regular() -> Font {
        let mut font = Font::new("Wide", 1000).unwrap();
        for (name, code) in [("H", 72), ("I", 73)] {
            font.insert_glyph(Glyph {
                name: name.into(),
                unicode: Some(code),
                advance: 500.0,
                contours: vec![Contour {
                    closed: true,
                    points: [(100.0, 0.0), (200.0, 0.0), (200.0, 700.0), (100.0, 700.0)]
                        .into_iter()
                        .map(|(x, y)| Point {
                            x,
                            y,
                            kind: PointKind::On,
                            smooth: false,
                        })
                        .collect(),
                }],
            })
            .unwrap();
        }
        font
    }

    #[test]
    fn new_fonts_are_the_regular_of_their_own_family() {
        let font = regular();
        assert_eq!(font.style.family, "Wide");
        assert_eq!(font.style.name, "Regular");
        assert_eq!(font.full_name(), "Wide Regular");
        assert_eq!(font.file_stem(), "Wide-Regular");
        assert_eq!(font.legacy_names(), ("Wide".into(), "Regular".into()));
        // An old file with no style block reads the same way.
        let old = r#"{"format":"typefoundry.font","version":1,"name":"Old","upm":1000,
            "metrics":{"ascender":800,"cap_height":700,"x_height":500,"baseline":0,"descender":-200},
            "glyphs":[]}"#;
        let loaded = Font::from_json(old).unwrap();
        assert_eq!(loaded.style.family, "Old");
        assert_eq!(loaded.style.weight, 400);
    }

    #[test]
    fn derives_a_slanted_italic() {
        let upright = regular();
        let italic = upright.derive_style("Italic", None, None, 12.0).unwrap();
        assert_eq!(italic.style.family, "Wide");
        assert_eq!(italic.name, "Wide Italic");
        assert!(italic.style.italic);
        assert_eq!(italic.style.italic_angle, -12.0);
        // Shear about x_height/2 without re-centring ink (geometry §1.2 / F7).
        let pivot = upright.metrics.x_height / 2.0;
        let shear = 12f64.to_radians().tan();
        let top = italic.glyph("H").unwrap().contours[0].points[2].x;
        let bottom = italic.glyph("H").unwrap().contours[0].points[0].x;
        let expected_top = 200.0 + (700.0 - pivot) * shear;
        let expected_bottom = 100.0 + (0.0 - pivot) * shear;
        assert!((top - expected_top).abs() < 1e-6, "{top}");
        assert!((bottom - expected_bottom).abs() < 1e-6, "{bottom}");
        assert_eq!(italic.legacy_names(), ("Wide".into(), "Italic".into()));
        assert!(upright.derive_style("Back", None, None, 75.0).is_err());

        let light = upright
            .derive_style("Light Italic", Some(300), Some(true), 0.0)
            .unwrap();
        assert_eq!(light.legacy_names(), ("Wide Light".into(), "Italic".into()));
        assert_eq!(light.file_stem(), "Wide-LightItalic");
    }

    #[test]
    fn set_style_checks_and_renames() {
        let mut font = regular();
        font.set_style(StyleUpdate {
            name: Some("Bold".into()),
            weight: Some(700),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(font.name, "Wide Bold");
        assert_eq!(font.legacy_names(), ("Wide".into(), "Bold".into()));
        let before = font.clone();
        assert!(
            font.set_style(StyleUpdate {
                weight: Some(0),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            font.set_style(StyleUpdate {
                family: Some(" ".into()),
                ..Default::default()
            })
            .is_err()
        );
        assert_eq!(font, before);
    }

    #[test]
    fn checks_a_family() {
        let upright = regular();
        let mut italic = upright.derive_style("Italic", None, None, 10.0).unwrap();
        assert!(check_family(&[&upright, &italic]).is_empty());

        italic.delete_glyph("I").unwrap();
        italic.metrics.ascender = 900.0;
        let issues = check_family(&[&upright, &italic]);
        let codes: Vec<&str> = issues.iter().map(|issue| issue.code).collect();
        assert!(codes.contains(&"missing"));
        assert!(codes.contains(&"metrics"));
        assert!(issues.iter().all(|issue| !issue.blocking));

        let twin = upright.clone();
        let issues = check_family(&[&upright, &twin]);
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == "style" && issue.blocking)
        );

        let mut stranger = upright.derive_style("Bold", Some(700), None, 0.0).unwrap();
        stranger
            .set_style(StyleUpdate {
                family: Some("Narrow".into()),
                ..Default::default()
            })
            .unwrap();
        let issues = check_family(&[&upright, &stranger]);
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == "family" && issue.blocking)
        );
    }

    #[test]
    fn saves_opens_and_exports_a_family() {
        let dir = temp_dir();
        let upright = regular();
        let italic = upright.derive_style("Italic", None, None, 12.0).unwrap();

        let family_path = dir.join("Wide.family.json");
        let written = save_family(&[&upright, &italic], &family_path, false).unwrap();
        assert_eq!(
            written,
            vec![dir.join("Wide-Regular.json"), dir.join("Wide-Italic.json")]
        );
        let (file, members) = load_family(&family_path).unwrap();
        assert_eq!(file.family, "Wide");
        assert_eq!(file.styles, vec!["Wide-Regular.json", "Wide-Italic.json"]);
        assert_eq!(members[1].1.style, italic.style);
        assert_eq!(members[1].1.name, italic.name);
        let loaded_h = &members[1].1.glyph("H").unwrap().contours[0].points;
        let source_h = &italic.glyph("H").unwrap().contours[0].points;
        for (loaded, source) in loaded_h.iter().zip(source_h) {
            assert!(
                (loaded.x - source.x).abs() < 1e-6,
                "{} {}",
                loaded.x,
                source.x
            );
            assert!((loaded.y - source.y).abs() < 1e-6);
        }

        let out = dir.join("ttf");
        let fonts = export_family(&[&upright, &italic], &out, ExportFormat::Ttf, false).unwrap();
        assert_eq!(fonts[1], out.join("Wide-Italic.ttf"));
        let reread = Font::load(&fonts[1]).unwrap();
        assert_eq!(reread.style.family, "Wide");
        assert_eq!(reread.style.name, "Italic");
        assert!(reread.style.italic);
        assert!((reread.style.italic_angle + 12.0).abs() < 0.01);
        assert_eq!(reread.style.weight, 400);

        let ufos = export_family(
            &[&upright, &italic],
            &dir.join("ufo"),
            ExportFormat::Ufo,
            false,
        )
        .unwrap();
        let ufo = Font::load(&ufos[1]).unwrap();
        assert_eq!(
            (ufo.style.family.as_str(), ufo.style.name.as_str()),
            ("Wide", "Italic")
        );
        assert!(ufo.style.italic);

        let twin = upright.clone();
        let refused = export_family(
            &[&upright, &twin],
            &dir.join("twins"),
            ExportFormat::Ttf,
            false,
        );
        assert!(matches!(refused, Err(FoundryError::Family(_))));
        assert!(
            !dir.join("twins").exists(),
            "nothing is written when the check blocks"
        );

        let _ = fs::remove_dir_all(dir);
    }
}
