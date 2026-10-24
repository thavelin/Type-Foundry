//! Letterform studies. A study records proportions and drops the outlines.
//!
//! The numbers are stem color, contrast, width, vertical metrics, overshoot, and
//! how much a family's stem grows from its lightest roman to its heaviest. They
//! are not enough to redraw a glyph. Contours never enter the study file.
//!
//! A study file in the Type Foundry `studies` folder, or in the user studies
//! folder, can be opened by its file name.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::edit::Side;
use crate::error::FoundryError;
use crate::font::{Font, Glyph};
use crate::geometry::{flatten_contour, ink_bounds, measure_bars_ray};

pub const STUDY_FORMAT: &str = "typefoundry.study";
pub const STUDY_VERSION: u32 = 1;

/// One measured face. Every length except `fitting`, `contrast`, `o_proportion`,
/// and `x_over_cap` is a fraction of the em.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LetterformStudy {
    pub family: String,
    pub style: String,
    pub weight: u16,
    pub italic: bool,
    pub italic_angle: f64,
    pub width: u16,
    pub upm: u16,
    pub glyphs: usize,
    pub encoded: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_height: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap_height: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ascender: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descender: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_over_cap: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overshoot_o: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overshoot_cap: Option<f64>,
    /// Lowercase vertical stem, from n, then h, then o.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stem: Option<f64>,
    /// Cap vertical stem, from H.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stem_cap: Option<f64>,
    /// Horizontal thickness, from H, then e.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<f64>,
    /// Vertical stem divided by the horizontal bar. Near 1 is monoline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_n: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_o: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_h: Option<f64>,
    /// Ink width of o divided by the x-height. Near 1 is a round o.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub o_proportion: Option<f64>,
    /// Average n sidebearing divided by the n stem. Higher is a looser fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fitting: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub designer: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vendor: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FamilyStudy {
    pub family: String,
    pub faces: usize,
    pub weight_min: u16,
    pub weight_max: u16,
    pub italic_faces: usize,
    pub widths: Vec<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_x_over_cap: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_stem: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_contrast: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_width_n: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_o_proportion: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_fitting: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_overshoot_o: Option<f64>,
    /// Heaviest roman stem divided by the lightest, at the text face's width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight_growth: Option<f64>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StudyFailure {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StudyLibrary {
    pub format: String,
    pub version: u32,
    /// Always this sentence. The file is not a font.
    pub outlines: String,
    pub root: String,
    pub faces: Vec<LetterformStudy>,
    pub families: Vec<FamilyStudy>,
    pub notes: Vec<String>,
    pub duplicates: usize,
    pub failed: Vec<StudyFailure>,
}

/// Measure `root` (a font file or a folder) and write a study. A second file with
/// the same family, style, weight, italic, and width is counted as a duplicate.
/// Static desktop files are preferred over variable and web copies. Variable
/// fonts contribute only the default instance when no static face wins.
pub fn write_study(
    root: &Path,
    out: &Path,
    force: bool,
    progress: bool,
) -> Result<StudyLibrary, FoundryError> {
    let library = study_path(root, progress)?;
    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|err| FoundryError::Io(err.to_string()))?;
    }
    crate::save::prepare_write(out, force)?;
    let mut text = serde_json::to_string_pretty(&library)
        .map_err(|err| FoundryError::Json(err.to_string()))?;
    text.push('\n');
    fs::write(out, text).map_err(|err| FoundryError::Io(err.to_string()))?;
    Ok(library)
}

pub fn study_path(root: &Path, progress: bool) -> Result<StudyLibrary, FoundryError> {
    if !root.exists() {
        return Err(FoundryError::Io(format!(
            "{} does not exist",
            root.display()
        )));
    }
    let mut files = Vec::new();
    collect_fonts(root, &mut files)?;
    files.sort_by(|left, right| {
        source_rank(left)
            .cmp(&source_rank(right))
            .then(left.cmp(right))
    });
    let total = files.len();
    let mut faces = Vec::new();
    let mut seen = Vec::<String>::new();
    let mut duplicates = 0usize;
    let mut failed = Vec::new();
    for (index, path) in files.iter().enumerate() {
        if progress {
            eprintln!("study {}/{} {}", index + 1, total, path.display());
        }
        match Font::load(path) {
            Ok(font) => {
                if font.glyphs.is_empty() {
                    failed.push(StudyFailure {
                        path: path.display().to_string(),
                        reason: "font has no glyphs".to_string(),
                    });
                    continue;
                }
                let key = face_key(&font);
                if seen.iter().any(|existing| existing == &key) {
                    duplicates += 1;
                    continue;
                }
                seen.push(key);
                faces.push(study_font(&font, &path.display().to_string()));
            }
            Err(err) => failed.push(StudyFailure {
                path: path.display().to_string(),
                reason: err.to_string(),
            }),
        }
    }
    if faces.is_empty() {
        return Err(FoundryError::Import(format!(
            "no readable fonts in {}",
            root.display()
        )));
    }
    faces.sort_by(|left, right| {
        left.family
            .cmp(&right.family)
            .then(left.width.cmp(&right.width))
            .then(left.weight.cmp(&right.weight))
            .then(left.italic.cmp(&right.italic))
            .then(left.style.cmp(&right.style))
    });
    let families = family_studies(&faces);
    let notes = corpus_notes(&faces, &families, duplicates, failed.len());
    Ok(StudyLibrary {
        format: STUDY_FORMAT.to_string(),
        version: STUDY_VERSION,
        outlines: "not stored".to_string(),
        root: root.display().to_string(),
        faces,
        families,
        notes,
        duplicates,
        failed,
    })
}

pub fn study_font(font: &Font, source: &str) -> LetterformStudy {
    let upm = f64::from(font.upm);
    let x_height = font.metrics.x_height;
    let cap_height = font.metrics.cap_height;
    let n = sample(font, 'n');
    let h = sample(font, 'h');
    let o = sample(font, 'o');
    let e = sample(font, 'e');
    let cap_h = sample(font, 'H');
    let cap_o = sample(font, 'O');
    let stem_units = n
        .and_then(|glyph| vertical_stem(glyph, zone(x_height, glyph)))
        .or_else(|| h.and_then(|glyph| vertical_stem(glyph, zone(x_height, glyph))))
        .or_else(|| o.and_then(|glyph| vertical_stem(glyph, zone(x_height, glyph))));
    let stem_cap_units = cap_h.and_then(|glyph| vertical_stem(glyph, zone(cap_height, glyph)));
    let bar_units = cap_h
        .and_then(horizontal_bar)
        .or_else(|| e.and_then(horizontal_bar));
    let stem = stem_units.and_then(|value| ratio(value, upm));
    let bar = bar_units.and_then(|value| ratio(value, upm));
    let contrast = match (stem_units.or(stem_cap_units), bar_units) {
        (Some(stem), Some(bar)) => ratio(stem, bar),
        _ => None,
    };
    LetterformStudy {
        family: font.style.family.clone(),
        style: font.style.name.clone(),
        weight: font.style.weight,
        italic: font.style.italic,
        italic_angle: round4(font.style.italic_angle),
        width: font.style.width,
        upm: font.upm,
        glyphs: font.glyphs.len(),
        encoded: font
            .glyphs
            .iter()
            .filter(|glyph| glyph.unicode.is_some())
            .count(),
        x_height: ratio(x_height, upm),
        cap_height: ratio(cap_height, upm),
        ascender: ratio(font.metrics.ascender, upm),
        descender: ratio(font.metrics.descender, upm),
        x_over_cap: ratio(x_height, cap_height),
        overshoot_o: o.and_then(|glyph| overshoot(glyph, x_height, upm)),
        overshoot_cap: cap_o.and_then(|glyph| overshoot(glyph, cap_height, upm)),
        stem,
        stem_cap: stem_cap_units.and_then(|value| ratio(value, upm)),
        bar,
        contrast,
        width_n: n.and_then(|glyph| ratio(glyph.advance, upm)),
        width_o: o.and_then(|glyph| ratio(glyph.advance, upm)),
        width_h: cap_h.and_then(|glyph| ratio(glyph.advance, upm)),
        o_proportion: o.and_then(|glyph| {
            let (min_x, max_x, _, _) = ink_bounds(glyph);
            ratio(max_x - min_x, x_height)
        }),
        fitting: n.and_then(|glyph| fitting(glyph, stem_units)),
        designer: font.info.designer.clone(),
        vendor: font.info.vendor.clone(),
        source: source.to_string(),
    }
}

/// What [`apply_study`] changed. Lengths are font units. Ratios are unitless.
/// Overshoot is reported and not drawn: a study has no outlines to copy.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProportionFit {
    pub family: String,
    pub style: String,
    pub weight: u16,
    pub x_over_cap_before: f64,
    pub x_over_cap_after: f64,
    pub scale: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebearing: Option<f64>,
    pub glyphs: Vec<String>,
    pub ascender: f64,
    pub descender: f64,
    pub cap_height: f64,
    pub x_height: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overshoot_drawn: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overshoot_study: Option<f64>,
}

/// Read a `typefoundry.study` file. The file has ratios, not outlines.
pub fn read_study(path: &Path) -> Result<StudyLibrary, FoundryError> {
    let text = fs::read_to_string(path).map_err(|err| FoundryError::Io(err.to_string()))?;
    let library: StudyLibrary =
        serde_json::from_str(&text).map_err(|err| FoundryError::Json(err.to_string()))?;
    if library.format != STUDY_FORMAT {
        return Err(FoundryError::Import(format!(
            "{} is not a typefoundry study",
            path.display()
        )));
    }
    if library.version != STUDY_VERSION {
        return Err(FoundryError::Import(format!(
            "unsupported study version {}",
            library.version
        )));
    }
    Ok(library)
}

/// One study file the foundry can open by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedStudy {
    /// File stem, without `.json`.
    pub name: String,
    pub path: String,
    pub faces: usize,
    pub families: usize,
    /// `shipped` for the copy kept with the Type Foundry, `user` for the AppData copy.
    pub place: String,
}

/// The `studies` folder next to the Type Foundry source.
///
/// This is the source tree the crate was compiled from, so a binary built from
/// that tree finds the folder without the shell's working directory.
pub fn shipped_studies_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../studies")
}

/// `%APPDATA%/Type Foundry/data/studies` when `APPDATA` is set.
/// The path is read at runtime and is not baked into the binary.
pub fn user_studies_dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join("Type Foundry")
            .join("data")
            .join("studies"),
    )
}

fn study_dirs() -> Vec<(&'static str, PathBuf)> {
    let mut dirs = vec![("shipped", shipped_studies_dir())];
    if let Some(dir) = user_studies_dir() {
        dirs.push(("user", dir));
    }
    dirs
}

/// Studies the foundry can open by name. The shipped copy wins when both folders
/// have the same file stem. A `.json` file that is not a study is skipped.
pub fn list_studies() -> Result<Vec<ListedStudy>, FoundryError> {
    let dirs = study_dirs();
    list_studies_in(&dirs)
}

fn list_studies_in(dirs: &[(&str, PathBuf)]) -> Result<Vec<ListedStudy>, FoundryError> {
    let mut listed = Vec::new();
    let mut seen = BTreeSet::new();
    for (place, dir) in dirs {
        if !dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        let entries = fs::read_dir(dir).map_err(|err| FoundryError::Io(err.to_string()))?;
        for entry in entries {
            let path = entry
                .map_err(|err| FoundryError::Io(err.to_string()))?
                .path();
            if is_json(&path) {
                files.push(path);
            }
        }
        files.sort();
        for path in files {
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let key = name.to_ascii_lowercase();
            if seen.contains(&key) {
                continue;
            }
            let Some((faces, families)) = study_counts(&path)? else {
                continue;
            };
            seen.insert(key);
            listed.push(ListedStudy {
                name: name.to_string(),
                path: plain_path(&path),
                faces,
                families,
                place: (*place).to_string(),
            });
        }
    }
    listed.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    Ok(listed)
}

/// A study file, or a library name. An existing file wins. Otherwise the name is
/// a file stem, with or without `.json`, or a unique part of one stem.
/// `klim` selects `klim-2026-10-17` when that is the only match.
pub fn resolve_study(query: &Path) -> Result<PathBuf, FoundryError> {
    let dirs = study_dirs();
    resolve_study_in(query, &dirs)
}

fn resolve_study_in(query: &Path, dirs: &[(&str, PathBuf)]) -> Result<PathBuf, FoundryError> {
    if query.is_file() {
        return Ok(query.to_path_buf());
    }
    if query.exists() {
        return Err(FoundryError::Io(format!(
            "{} is not a study file",
            query.display()
        )));
    }
    let raw = query.to_string_lossy();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FoundryError::Edit(
            "a study fit needs a study file or name".into(),
        ));
    }
    if looks_like_path(trimmed) {
        return Err(FoundryError::Io(format!("{trimmed} does not exist")));
    }
    let stem = Path::new(trimmed)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(trimmed);
    let wanted = stem.to_ascii_lowercase();
    if wanted.is_empty() {
        return Err(FoundryError::Edit(
            "a study fit needs a study file or name".into(),
        ));
    }
    let catalog = list_studies_in(dirs)?;
    let exact: Vec<&ListedStudy> = catalog
        .iter()
        .filter(|study| study.name.eq_ignore_ascii_case(&wanted))
        .collect();
    if let Some(path) = pick_named(&exact, trimmed)? {
        return Ok(path);
    }
    let loose: Vec<&ListedStudy> = catalog
        .iter()
        .filter(|study| study.name.to_ascii_lowercase().contains(&wanted))
        .collect();
    if let Some(path) = pick_named(&loose, trimmed)? {
        return Ok(path);
    }
    let names = catalog
        .iter()
        .map(|study| study.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if names.is_empty() {
        return Err(FoundryError::Edit(format!("no study is named {trimmed}")));
    }
    Err(FoundryError::Edit(format!(
        "no study is named {trimmed}. Studies: {names}"
    )))
}

fn pick_named(studies: &[&ListedStudy], label: &str) -> Result<Option<PathBuf>, FoundryError> {
    match studies {
        [] => Ok(None),
        [study] => Ok(Some(PathBuf::from(&study.path))),
        many => {
            let names = many
                .iter()
                .map(|study| study.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(FoundryError::Edit(format!(
                "{label} matches {names}. Pass the full study name."
            )))
        }
    }
}

/// Drop `.` and `..` so a path built from the crate directory reads as a real folder.
fn plain_path(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(parts.last(), Some(Component::Normal(_))) {
                    parts.pop();
                } else {
                    parts.push(component);
                }
            }
            other => parts.push(other),
        }
    }
    parts.iter().collect::<PathBuf>().display().to_string()
}

fn looks_like_path(query: &str) -> bool {
    query.contains(['/', '\\']) || Path::new(query).has_root()
}

fn is_json(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

/// Face and family counts, or `None` when the file is not a study.
fn study_counts(path: &Path) -> Result<Option<(usize, usize)>, FoundryError> {
    let text = fs::read_to_string(path)
        .map_err(|err| FoundryError::Io(format!("{}: {err}", path.display())))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|err| FoundryError::Json(format!("{}: {err}", path.display())))?;
    let Some(format) = value.get("format").and_then(|item| item.as_str()) else {
        return Ok(None);
    };
    if format != STUDY_FORMAT {
        return Ok(None);
    }
    let version = value
        .get("version")
        .and_then(|item| item.as_u64())
        .unwrap_or(0);
    if version != u64::from(STUDY_VERSION) {
        return Err(FoundryError::Import(format!(
            "{} has unsupported study version {version}",
            path.display()
        )));
    }
    let faces = value
        .get("faces")
        .and_then(|item| item.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    let families = value
        .get("families")
        .and_then(|item| item.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    Ok(Some((faces, families)))
}

/// The face to copy proportions from. `family` matches the full name, or the name
/// with a leading "Test " ignored, so "The Future" selects "Test The Future" and
/// not "Test The Future Mono". No style picks the text face. `weight` picks the
/// roman nearest that weight.
pub fn select_face<'a>(
    library: &'a StudyLibrary,
    family: &str,
    style: Option<&str>,
    weight: Option<u16>,
) -> Result<&'a LetterformStudy, FoundryError> {
    let wanted = family_key(family);
    if wanted.is_empty() {
        return Err(FoundryError::Edit("a study fit needs a family name".into()));
    }
    let mut matched: Vec<&LetterformStudy> = library
        .faces
        .iter()
        .filter(|face| family_key(&face.family) == wanted)
        .collect();
    if matched.is_empty() {
        return Err(FoundryError::Edit(format!(
            "no family in the study is named {family}"
        )));
    }
    let mut names: Vec<&str> = matched.iter().map(|face| face.family.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    if names.len() > 1 {
        return Err(FoundryError::Edit(format!(
            "{family} matches {}. Pass the full family name.",
            names.join(", ")
        )));
    }
    if let Some(style) = style.map(str::trim).filter(|style| !style.is_empty()) {
        let styled: Vec<&LetterformStudy> = matched
            .iter()
            .copied()
            .filter(|face| face.style.eq_ignore_ascii_case(style))
            .collect();
        if styled.is_empty() {
            return Err(FoundryError::Edit(format!(
                "{} has no style named {style}",
                names[0]
            )));
        }
        matched = styled;
    }
    if matched.len() == 1 {
        return Ok(matched[0]);
    }
    let wanted_weight = weight.unwrap_or(400);
    matched.sort_by_key(|face| {
        (
            u8::from(style.map(str::trim).unwrap_or("").is_empty() && face.italic),
            u8::from(face.width != 5),
            face.weight.abs_diff(wanted_weight),
        )
    });
    Ok(matched[0])
}

/// Fit the open font to one studied face. Lowercase letters scale about the
/// baseline so their x-height over the cap height matches the study. Caps keep
/// their height. Sidebearings of letters and figures become the study's fitting
/// times this font's stem. Vertical metrics come from the study and widen to
/// cover the ink. Outlines are not copied from the study, and round overshoot is
/// left as drawn.
pub fn apply_study(
    font: &mut Font,
    study_path: &Path,
    family: &str,
    style: Option<&str>,
    weight: Option<u16>,
) -> Result<ProportionFit, FoundryError> {
    let resolved = resolve_study(study_path)?;
    let library = read_study(&resolved)?;
    let face = select_face(&library, family, style, weight)?.clone();
    let mut fitted = font.clone();
    let report = fit_proportions(&mut fitted, &face)?;
    *font = fitted;
    Ok(report)
}

fn fit_proportions(font: &mut Font, face: &LetterformStudy) -> Result<ProportionFit, FoundryError> {
    let ratio = face.x_over_cap.ok_or_else(|| {
        FoundryError::Edit(format!(
            "{} {} has no x-height over cap height",
            face.family, face.style
        ))
    })?;
    if !(0.2..=1.2).contains(&ratio) {
        return Err(FoundryError::Edit(format!(
            "{} {} has an x-height over cap height of {ratio}, outside 0.2 to 1.2",
            face.family, face.style
        )));
    }
    let cap = seated_top(font, on_cap_line)
        .filter(|value| *value > 1.0)
        .unwrap_or(font.metrics.cap_height);
    let x_height = seated_top(font, on_x_line)
        .filter(|value| *value > 1.0)
        .unwrap_or(font.metrics.x_height);
    if cap <= 1.0 || x_height <= 1.0 {
        return Err(FoundryError::Edit(
            "this font has no cap height and x-height to fit".into(),
        ));
    }
    let scale = cap * ratio / x_height;
    if !scale.is_finite() || scale <= 0.0 || scale > 4.0 {
        return Err(FoundryError::Edit(
            "lowercase scale must be greater than 0 and at most 4".into(),
        ));
    }
    let mut glyphs = Vec::new();
    for glyph in &mut font.glyphs {
        let Some(code) = glyph.unicode else {
            continue;
        };
        if (0x61..=0x7A).contains(&code) && glyph_has_ink(glyph) {
            scale_about_origin(glyph, scale);
            glyphs.push(glyph.name.clone());
        }
    }
    let target_x = cap * ratio;
    for glyph in &mut font.glyphs {
        let Some(code) = glyph.unicode else {
            continue;
        };
        if !centers_on_x_height(code) || !glyph_has_ink(glyph) {
            continue;
        }
        let (_, _, min_y, max_y) = ink_bounds(glyph);
        let dy = target_x / 2.0 - (min_y + max_y) / 2.0;
        if dy.abs() < 0.01 {
            continue;
        }
        for contour in &mut glyph.contours {
            for point in &mut contour.points {
                point.y += dy;
            }
        }
        glyphs.push(glyph.name.clone());
    }
    let upm = f64::from(font.upm);
    let stem_units = sample(font, 'n')
        .and_then(|glyph| vertical_stem(glyph, target_x))
        .or_else(|| sample(font, 'h').and_then(|glyph| vertical_stem(glyph, target_x)))
        .or_else(|| sample(font, 'o').and_then(|glyph| vertical_stem(glyph, target_x)));
    let sidebearing = match (face.fitting, stem_units) {
        (Some(fitting), Some(stem)) if fitting.is_finite() && stem.is_finite() => {
            Some(fitting * stem)
        }
        _ => None,
    };
    if let Some(bearing) = sidebearing {
        let names: Vec<String> = font
            .glyphs
            .iter()
            .filter(|glyph| glyph.unicode.is_some_and(takes_text_bearing) && glyph_has_ink(glyph))
            .map(|glyph| glyph.name.clone())
            .collect();
        for name in &names {
            font.set_sidebearing(name, Side::Left, bearing)?;
            font.set_sidebearing(name, Side::Right, bearing)?;
            if !glyphs.contains(name) {
                glyphs.push(name.clone());
            }
        }
    }
    let (ink_bottom, ink_top) = metric_ink(font);
    let ascender = face
        .ascender
        .map(|value| value * upm)
        .unwrap_or(ink_top)
        .max(ink_top)
        .max(cap);
    let descender = face
        .descender
        .map(|value| value * upm)
        .unwrap_or(ink_bottom)
        .min(ink_bottom)
        .min(0.0);
    font.metrics.ascender = ascender;
    font.metrics.cap_height = cap;
    font.metrics.x_height = target_x;
    font.metrics.baseline = 0.0;
    font.metrics.descender = descender;
    glyphs.sort();
    glyphs.dedup();
    Ok(ProportionFit {
        family: face.family.clone(),
        style: face.style.clone(),
        weight: face.weight,
        x_over_cap_before: round4(x_height / cap),
        x_over_cap_after: round4(target_x / cap),
        scale,
        sidebearing,
        glyphs,
        ascender,
        descender,
        cap_height: cap,
        x_height: target_x,
        overshoot_drawn: sample(font, 'o').and_then(|glyph| overshoot(glyph, target_x, upm)),
        overshoot_study: face.overshoot_o,
    })
}

fn family_key(name: &str) -> String {
    let trimmed = name.trim().to_lowercase();
    trimmed
        .strip_prefix("test ")
        .unwrap_or(&trimmed)
        .trim()
        .to_string()
}

fn seated_top(font: &Font, pred: fn(u32) -> bool) -> Option<f64> {
    let mut tops = Vec::new();
    for glyph in &font.glyphs {
        let Some(code) = glyph.unicode else {
            continue;
        };
        if !pred(code) {
            continue;
        }
        let (_, _, min_y, max_y) = ink_bounds(glyph);
        if min_y.is_finite() && max_y > min_y + 1.0 && min_y.abs() <= 30.0 {
            tops.push(max_y);
        }
    }
    median(&tops)
}

fn metric_ink(font: &Font) -> (f64, f64) {
    let mut bottom = 0.0_f64;
    let mut top = 0.0_f64;
    for glyph in &font.glyphs {
        let Some(code) = glyph.unicode else {
            continue;
        };
        if !counts_toward_metrics(code) {
            continue;
        }
        let (_, _, min_y, max_y) = ink_bounds(glyph);
        if min_y.is_finite() {
            bottom = bottom.min(min_y);
        }
        if max_y.is_finite() {
            top = top.max(max_y);
        }
    }
    (bottom, top)
}

fn scale_about_origin(glyph: &mut Glyph, factor: f64) {
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.x *= factor;
            point.y *= factor;
        }
    }
    glyph.advance *= factor;
}

fn glyph_has_ink(glyph: &Glyph) -> bool {
    let (min_x, _, _, _) = ink_bounds(glyph);
    min_x.is_finite()
}

fn on_x_line(code: u32) -> bool {
    matches!(
        char::from_u32(code),
        Some('a' | 'm' | 'n' | 'r' | 'u' | 'v' | 'w' | 'x' | 'z')
    )
}

fn on_cap_line(code: u32) -> bool {
    matches!(
        char::from_u32(code),
        Some(
            'A' | 'B'
                | 'D'
                | 'E'
                | 'F'
                | 'H'
                | 'I'
                | 'K'
                | 'L'
                | 'M'
                | 'N'
                | 'P'
                | 'R'
                | 'T'
                | 'V'
                | 'W'
                | 'X'
                | 'Y'
                | 'Z'
        )
    )
}

fn centers_on_x_height(code: u32) -> bool {
    matches!(
        char::from_u32(code),
        Some('-' | '+' | '<' | '=' | '>' | '~')
    )
}

fn takes_text_bearing(code: u32) -> bool {
    matches!(code, 0x30..=0x39 | 0x41..=0x5A | 0x61..=0x7A)
}

fn counts_toward_metrics(code: u32) -> bool {
    takes_text_bearing(code)
        || matches!(
            char::from_u32(code),
            Some('|' | '$' | '@' | ',' | '_' | '\'' | '`' | '-' | '+' | '.')
        )
}

fn family_studies(faces: &[LetterformStudy]) -> Vec<FamilyStudy> {
    let mut names: Vec<&str> = faces.iter().map(|face| face.family.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let group: Vec<&LetterformStudy> =
                faces.iter().filter(|face| face.family == name).collect();
            family_study(&group)
        })
        .collect()
}

fn family_study(group: &[&LetterformStudy]) -> FamilyStudy {
    let mut weights: Vec<u16> = group.iter().map(|face| face.weight).collect();
    weights.sort_unstable();
    let mut widths: Vec<u16> = group.iter().map(|face| face.width).collect();
    widths.sort_unstable();
    widths.dedup();
    let text = group.iter().copied().min_by_key(|face| text_rank(face));
    let weight_growth = text.and_then(|text| weight_growth(group, text.width));
    let mut study = FamilyStudy {
        family: group[0].family.clone(),
        faces: group.len(),
        weight_min: weights[0],
        weight_max: *weights.last().unwrap_or(&weights[0]),
        italic_faces: group.iter().filter(|face| face.italic).count(),
        widths,
        text_style: text.map(|face| face.style.clone()),
        text_x_over_cap: text.and_then(|face| face.x_over_cap),
        text_stem: text.and_then(|face| face.stem),
        text_contrast: text.and_then(|face| face.contrast),
        text_width_n: text.and_then(|face| face.width_n),
        text_o_proportion: text.and_then(|face| face.o_proportion),
        text_fitting: text.and_then(|face| face.fitting),
        text_overshoot_o: text.and_then(|face| face.overshoot_o),
        weight_growth,
        summary: String::new(),
    };
    study.summary = family_summary(&study);
    study
}

fn weight_growth(group: &[&LetterformStudy], width: u16) -> Option<f64> {
    let mut stems: Vec<f64> = group
        .iter()
        .filter(|face| !face.italic && face.width == width)
        .filter_map(|face| face.stem)
        .filter(|stem| *stem > 0.0)
        .collect();
    if stems.len() < 2 {
        return None;
    }
    stems.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    ratio(stems[stems.len() - 1], stems[0])
}

fn family_summary(study: &FamilyStudy) -> String {
    let mut parts = vec![format!(
        "{}: {} faces, weight {}–{}",
        study.family, study.faces, study.weight_min, study.weight_max
    )];
    if study.italic_faces > 0 {
        parts.push(format!("{} italic", study.italic_faces));
    }
    if study.widths.len() > 1 {
        parts.push(format!("widths {}", join_widths(&study.widths)));
    }
    if let Some(style) = &study.text_style {
        parts.push(format!("text {style}"));
    }
    if let Some(value) = study.text_x_over_cap {
        parts.push(format!("x/cap {value:.3}"));
    }
    if let Some(value) = study.text_stem {
        parts.push(format!("stem {:.1}% em", value * 100.0));
    }
    if let Some(value) = study.text_contrast {
        parts.push(format!("contrast {value:.2}"));
    }
    if let Some(value) = study.text_width_n {
        parts.push(format!("n {:.0}% em", value * 100.0));
    }
    if let Some(value) = study.text_o_proportion {
        parts.push(format!("o/x {value:.2}"));
    }
    if let Some(value) = study.text_fitting {
        parts.push(format!("fit {value:.2}"));
    }
    if let Some(value) = study.weight_growth {
        parts.push(format!("weight {value:.2}×"));
    }
    parts.join(", ")
}

fn corpus_notes(
    faces: &[LetterformStudy],
    families: &[FamilyStudy],
    duplicates: usize,
    failures: usize,
) -> Vec<String> {
    let mut notes = vec![format!(
        "Studied {} faces in {} families. Glyph outlines were measured and discarded.",
        faces.len(),
        families.len()
    )];
    if duplicates > 0 {
        notes.push(format!(
            "{duplicates} files repeated a face already measured. Static desktop fonts win over variable and web copies. A variable font would only have contributed its default instance."
        ));
    }
    if failures > 0 {
        notes.push(format!("{failures} files could not be read."));
    }
    let text: Vec<&FamilyStudy> = families.iter().collect();
    if let Some(value) = median_of(text.iter().filter_map(|family| family.text_x_over_cap)) {
        notes.push(format!(
            "The text face of each family has a median x-height of {value:.3} of the cap height. Tall here means 0.70 or more. Short means 0.62 or less."
        ));
    }
    if let Some(value) = median_of(text.iter().filter_map(|family| family.text_stem)) {
        notes.push(format!(
            "Median text-face stem is {:.1}% of the em.",
            value * 100.0
        ));
    }
    if let Some(value) = median_of(text.iter().filter_map(|family| family.text_contrast)) {
        notes.push(format!(
            "Median stem-to-bar contrast is {value:.2}. Near 1 is monoline (1.15 or under). 1.45 or more is contrasted."
        ));
    }
    push_group(
        &mut notes,
        "Tall x-height",
        &names_where(families, |family| {
            family.text_x_over_cap.is_some_and(|value| value >= 0.70)
        }),
    );
    push_group(
        &mut notes,
        "Short x-height",
        &names_where(families, |family| {
            family.text_x_over_cap.is_some_and(|value| value <= 0.62)
        }),
    );
    push_group(
        &mut notes,
        "Monoline",
        &names_where(families, |family| {
            family.text_contrast.is_some_and(|value| value <= 1.15)
        }),
    );
    push_group(
        &mut notes,
        "Contrasted",
        &names_where(families, |family| {
            family.text_contrast.is_some_and(|value| value >= 1.45)
        }),
    );
    push_group(
        &mut notes,
        "Condensed widths present",
        &names_where(families, |family| {
            family.widths.iter().any(|width| *width <= 3)
        }),
    );
    notes.push(
        "This pass learns color, proportion, contrast, width, overshoot, and weight growth. It does not learn terminals, apertures, serifs, or the drawing of a letter."
            .to_string(),
    );
    notes
}

fn push_group(notes: &mut Vec<String>, label: &str, names: &[String]) {
    if names.is_empty() {
        return;
    }
    notes.push(format!("{label}: {}.", names.join(", ")));
}

fn names_where(families: &[FamilyStudy], keep: impl Fn(&FamilyStudy) -> bool) -> Vec<String> {
    families
        .iter()
        .filter(|family| keep(family))
        .map(|family| family.family.clone())
        .collect()
}

fn text_rank(face: &LetterformStudy) -> (u8, u8, u16) {
    (
        u8::from(face.italic),
        u8::from(face.width != 5),
        face.weight.abs_diff(400),
    )
}

fn sample(font: &Font, ch: char) -> Option<&Glyph> {
    let code = u32::from(ch);
    font.glyphs
        .iter()
        .find(|glyph| glyph.unicode == Some(code))
        .or_else(|| font.glyph(&ch.to_string()))
}

fn zone(metric: f64, glyph: &Glyph) -> f64 {
    if metric > 1.0 {
        return metric;
    }
    let (_, _, _, max_y) = ink_bounds(glyph);
    if max_y.is_finite() && max_y > 1.0 {
        max_y
    } else {
        0.0
    }
}

fn overshoot(glyph: &Glyph, line: f64, upm: f64) -> Option<f64> {
    let (_, _, _, max_y) = ink_bounds(glyph);
    if !max_y.is_finite() || line <= 1.0 {
        return None;
    }
    ratio(max_y - line, upm)
}

fn fitting(glyph: &Glyph, stem: Option<f64>) -> Option<f64> {
    let stem = stem.filter(|value| *value > 1.0)?;
    let (min_x, max_x, _, _) = ink_bounds(glyph);
    if !min_x.is_finite() {
        return None;
    }
    let lsb = min_x;
    let rsb = glyph.advance - max_x;
    ratio((lsb + rsb) / 2.0, stem)
}

/// Vertical stem thickness. Rays avoid treating a crossbar as a stem by dropping
/// any ink run wider than half the letter.
fn vertical_stem(glyph: &Glyph, zone_y: f64) -> Option<f64> {
    if zone_y <= 1.0 {
        return None;
    }
    let (min_x, max_x, _, _) = ink_bounds(glyph);
    if !min_x.is_finite() {
        return None;
    }
    let ink_width = max_x - min_x;
    if ink_width <= 1.0 {
        return None;
    }
    let mut runs = Vec::new();
    for fraction in [0.20, 0.35, 0.50, 0.65, 0.80] {
        runs.extend(ink_runs_at_y(glyph, zone_y * fraction));
    }
    let kept: Vec<f64> = runs
        .into_iter()
        .filter(|width| *width > 1.0 && *width <= ink_width * 0.5)
        .collect();
    median(&kept)
}

fn horizontal_bar(glyph: &Glyph) -> Option<f64> {
    median(&measure_bars_ray(glyph))
}

fn ink_runs_at_y(glyph: &Glyph, y: f64) -> Vec<f64> {
    let mut xs = Vec::new();
    for contour in &glyph.contours {
        if !contour.closed {
            continue;
        }
        let poly = flatten_contour(contour, 16);
        let count = if poly.len() > 1 && poly.first() == poly.last() {
            poly.len() - 1
        } else {
            poly.len()
        };
        for index in 0..count {
            let start = poly[index];
            let end = poly[(index + 1) % count];
            if (start.y > y) == (end.y > y) || (end.y - start.y).abs() < 1e-9 {
                continue;
            }
            let t = (y - start.y) / (end.y - start.y);
            if (0.0..=1.0).contains(&t) {
                xs.push(start.x + (end.x - start.x) * t);
            }
        }
    }
    xs.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let mut unique = Vec::new();
    for x in xs {
        if unique
            .last()
            .is_none_or(|previous: &f64| (x - previous).abs() > 0.5)
        {
            unique.push(x);
        }
    }
    let mut widths = Vec::new();
    let mut index = 0;
    while index + 1 < unique.len() {
        widths.push(unique[index + 1] - unique[index]);
        index += 2;
    }
    widths
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    let value = if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    };
    value.is_finite().then_some(value)
}

fn median_of(values: impl Iterator<Item = f64>) -> Option<f64> {
    median(&values.collect::<Vec<_>>())
}

fn ratio(part: f64, whole: f64) -> Option<f64> {
    if !part.is_finite() || !whole.is_finite() || whole.abs() < 1e-6 {
        return None;
    }
    let value = part / whole;
    value.is_finite().then_some(round4(value))
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn face_key(font: &Font) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        normalize(&font.style.family),
        normalize(&font.style.name),
        font.style.weight,
        font.style.italic,
        font.style.width
    )
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn join_widths(widths: &[u16]) -> String {
    widths
        .iter()
        .map(|width| width.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn source_rank(path: &Path) -> u8 {
    let text = path.to_string_lossy().to_ascii_lowercase();
    if text.contains("web font") || text.contains(".woff") {
        3
    } else if text.contains("variable") {
        2
    } else if text.contains("static") {
        0
    } else {
        1
    }
}

fn collect_fonts(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), FoundryError> {
    if root.is_file() {
        if is_study_file(root) {
            out.push(root.to_path_buf());
        }
        return Ok(());
    }
    let entries = fs::read_dir(root).map_err(|err| FoundryError::Io(err.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|err| FoundryError::Io(err.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            if crate::ufo::is_ufo_path(&path) {
                out.push(path);
            } else if !is_hidden(&path) {
                collect_fonts(&path, out)?;
            }
        } else if is_study_file(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

fn is_study_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc" | "woff" | "woff2" | "json"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{Contour, Point, PointKind};
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_IDS: AtomicU64 = AtomicU64::new(0);

    fn on(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            kind: PointKind::On,
            smooth: false,
        }
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Contour {
        Contour {
            closed: true,
            points: vec![on(x0, y0), on(x1, y0), on(x1, y1), on(x0, y1)],
        }
    }

    fn glyph(name: &str, unicode: char, advance: f64, contours: Vec<Contour>) -> Glyph {
        Glyph {
            strokes: Vec::new(),
            name: name.to_string(),
            unicode: Some(u32::from(unicode)),
            advance,
            contours,
        }
    }

    fn sample_font(weight: u16, stem: f64) -> Font {
        let mut font = Font::new("Sample", 1000).unwrap();
        font.style.family = "Sample".to_string();
        font.style.name = if weight >= 700 {
            "Bold".to_string()
        } else {
            "Regular".to_string()
        };
        font.style.weight = weight;
        font.metrics.ascender = 800.0;
        font.metrics.cap_height = 700.0;
        font.metrics.x_height = 500.0;
        font.metrics.descender = -200.0;
        let right = 400.0 + stem;
        font.insert_glyph(glyph(
            "n",
            'n',
            right + stem,
            vec![rect(0.0, 0.0, stem, 500.0), rect(400.0, 0.0, right, 500.0)],
        ))
        .unwrap();
        font.insert_glyph(glyph(
            "H",
            'H',
            right + stem,
            vec![
                rect(0.0, 0.0, stem, 700.0),
                rect(400.0, 0.0, right, 700.0),
                rect(stem, 310.0, 400.0, 390.0),
            ],
        ))
        .unwrap();
        font.insert_glyph(glyph(
            "o",
            'o',
            600.0,
            vec![
                rect(40.0, -10.0, 520.0, 510.0),
                rect(40.0 + stem, 70.0, 520.0 - stem, 430.0),
            ],
        ))
        .unwrap();
        font
    }

    #[test]
    fn measures_proportions_and_drops_outlines() {
        let study = study_font(&sample_font(400, 80.0), "Sample-Regular.json");
        assert_eq!(study.stem, Some(0.08));
        assert_eq!(study.stem_cap, Some(0.08));
        assert_eq!(study.bar, Some(0.08));
        assert_eq!(study.contrast, Some(1.0));
        assert_eq!(study.x_over_cap, Some(0.7143));
        assert_eq!(study.overshoot_o, Some(0.01));
        assert_eq!(study.o_proportion, Some(0.96));
        assert_eq!(study.width_n, Some(0.56));
        assert_eq!(study.fitting, Some(0.5));
        let text = serde_json::to_string(&study).unwrap();
        assert!(!text.contains("contours"));
        assert!(!text.contains("\"points\""));
        assert!(!text.contains("520"));
    }

    fn temp_dir() -> PathBuf {
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = TEMP_IDS.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("typefoundry-study-{tick}-{id}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_folder_keeps_one_measurement_per_face() {
        let root = temp_dir();
        let static_dir = root.join("desktop fonts (Static, OTF)");
        let variable_dir = root.join("desktop fonts (Variable, TTF)");
        fs::create_dir_all(&static_dir).unwrap();
        fs::create_dir_all(&variable_dir).unwrap();
        sample_font(400, 80.0)
            .save(&static_dir.join("Sample-Regular.json"))
            .unwrap();
        sample_font(400, 80.0)
            .save(&variable_dir.join("Sample-Variable.json"))
            .unwrap();
        sample_font(700, 160.0)
            .save(&static_dir.join("Sample-Bold.json"))
            .unwrap();
        fs::write(root.join("notes.txt"), "ignore").unwrap();
        fs::write(static_dir.join("bad.otf"), [0, 1, 2, 3]).unwrap();
        let out = root.join("study.json");
        let library = write_study(&root, &out, false, false).unwrap();
        assert_eq!(library.outlines, "not stored");
        assert_eq!(library.faces.len(), 2);
        assert_eq!(library.duplicates, 1);
        assert_eq!(library.failed.len(), 1);
        assert!(
            library
                .faces
                .iter()
                .all(|face| face.source.contains("Static"))
        );
        assert_eq!(library.families[0].weight_growth, Some(2.0));
        let stored: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(stored["outlines"], json!("not stored"));
        assert!(stored.get("glyphs").is_none());
        let again = write_study(&root, &out, false, false);
        assert!(again.is_err());
        let _ = fs::remove_dir_all(&root);
    }

    fn study_face(
        family: &str,
        style: &str,
        weight: u16,
        x_over_cap: f64,
        fitting: Option<f64>,
    ) -> LetterformStudy {
        LetterformStudy {
            family: family.to_string(),
            style: style.to_string(),
            weight,
            italic: style.to_lowercase().contains("italic"),
            italic_angle: 0.0,
            width: 5,
            upm: 1000,
            glyphs: 2,
            encoded: 2,
            x_height: Some(0.7 * x_over_cap),
            cap_height: Some(0.7),
            ascender: Some(0.93),
            descender: Some(-0.2),
            x_over_cap: Some(x_over_cap),
            overshoot_o: Some(0.012),
            overshoot_cap: None,
            stem: Some(0.08),
            stem_cap: None,
            bar: None,
            contrast: Some(1.05),
            width_n: None,
            width_o: None,
            width_h: None,
            o_proportion: None,
            fitting,
            designer: String::new(),
            vendor: String::new(),
            source: "study.json".to_string(),
        }
    }

    fn empty_library(faces: Vec<LetterformStudy>) -> StudyLibrary {
        StudyLibrary {
            format: STUDY_FORMAT.to_string(),
            version: STUDY_VERSION,
            outlines: "not stored".to_string(),
            root: String::new(),
            faces,
            families: Vec::new(),
            notes: Vec::new(),
            duplicates: 0,
            failed: Vec::new(),
        }
    }

    fn short_sample() -> Font {
        let mut font = Font::new("Vostok Sans", 1000).unwrap();
        font.metrics.cap_height = 700.0;
        font.metrics.x_height = 350.0;
        font.metrics.ascender = 800.0;
        font.metrics.descender = -120.0;
        font.insert_glyph(glyph(
            "n",
            'n',
            240.0,
            vec![rect(0.0, 0.0, 40.0, 350.0), rect(160.0, 0.0, 200.0, 350.0)],
        ))
        .unwrap();
        font.insert_glyph(glyph(
            "H",
            'H',
            140.0,
            vec![rect(0.0, 0.0, 40.0, 700.0), rect(100.0, 0.0, 140.0, 700.0)],
        ))
        .unwrap();
        font.insert_glyph(glyph(
            "-",
            '-',
            120.0,
            vec![rect(40.0, 160.0, 100.0, 190.0)],
        ))
        .unwrap();
        font
    }

    #[test]
    fn a_short_lowercase_grows_to_the_study_ratio() {
        let mut font = short_sample();
        let face = study_face("Test The Future", "Regular", 400, 0.7, Some(1.0));
        let report = fit_proportions(&mut font, &face).unwrap();
        assert!((report.scale - 1.4).abs() < 0.001, "{}", report.scale);
        assert!((report.x_over_cap_before - 0.5).abs() < 0.001);
        assert!((font.metrics.x_height - 490.0).abs() < 0.5);
        assert!((font.metrics.cap_height - 700.0).abs() < 0.5);
        assert!((font.metrics.ascender - 930.0).abs() < 0.5);
        assert!((font.metrics.descender - -200.0).abs() < 0.5);
        let n = font.glyph("n").unwrap();
        let (min_x, max_x, min_y, max_y) = ink_bounds(n);
        assert!(min_y.abs() < 0.5, "{min_y}");
        assert!((max_y - 490.0).abs() < 0.5, "{max_y}");
        assert!((min_x - 56.0).abs() < 1.0, "lsb {min_x}");
        assert!(
            (n.advance - max_x - 56.0).abs() < 1.0,
            "rsb {}",
            n.advance - max_x
        );
        let (_, _, h_bottom, h_top) = ink_bounds(font.glyph("H").unwrap());
        assert!(h_bottom.abs() < 0.5, "{h_bottom}");
        assert!((h_top - 700.0).abs() < 0.5, "{h_top}");
        let (_, _, y0, y1) = ink_bounds(font.glyph("-").unwrap());
        assert!(((y0 + y1) / 2.0 - 245.0).abs() < 1.0, "{y0} {y1}");
        assert_eq!(report.overshoot_study, Some(0.012));
        assert_eq!(report.overshoot_drawn, None);
    }

    #[test]
    fn the_future_names_the_text_face_and_not_the_mono() {
        let library = empty_library(vec![
            study_face("Test The Future Mono", "Regular", 400, 0.6233, Some(1.2)),
            study_face("Test The Future", "Black", 900, 0.687, Some(0.24)),
            study_face("Test The Future", "Regular", 400, 0.6233, Some(0.99)),
            study_face("Test The Future", "Italic", 400, 0.6233, Some(0.9)),
        ]);
        let text = select_face(&library, "The Future", None, None).unwrap();
        assert_eq!(text.family, "Test The Future");
        assert_eq!(text.style, "Regular");
        let black = select_face(&library, "Test The Future", None, Some(900)).unwrap();
        assert_eq!(black.style, "Black");
        let italic = select_face(&library, "the future", Some("Italic"), None).unwrap();
        assert_eq!(italic.style, "Italic");
        assert!(select_face(&library, "No Such Sans", None, None).is_err());
    }

    fn write_named_study(dir: &Path, name: &str) {
        let body = serde_json::to_string(&empty_library(vec![study_face(
            "Test The Future",
            "Regular",
            400,
            0.62,
            Some(1.0),
        )]))
        .unwrap();
        fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn a_study_name_resolves_and_the_shipped_copy_wins() {
        let root = temp_dir();
        let shipped = root.join("shipped");
        let user = root.join("user");
        fs::create_dir_all(&shipped).unwrap();
        fs::create_dir_all(&user).unwrap();
        write_named_study(&shipped, "klim-2026-10-17.json");
        write_named_study(&user, "klim-2026-10-17.json");
        write_named_study(&user, "other-sans.json");
        fs::write(shipped.join("notes.txt"), "ignore").unwrap();
        fs::write(shipped.join("scratch.json"), "{}").unwrap();
        let dirs = [("shipped", shipped.clone()), ("user", user.clone())];
        let listed = list_studies_in(&dirs).unwrap();
        assert_eq!(listed.len(), 2);
        let klim = listed
            .iter()
            .find(|study| study.name == "klim-2026-10-17")
            .unwrap();
        assert_eq!(klim.place, "shipped");
        assert_eq!(klim.faces, 1);
        assert_eq!(klim.families, 0);
        assert_eq!(
            resolve_study_in(Path::new("klim"), &dirs)
                .unwrap()
                .canonicalize()
                .unwrap(),
            shipped.join("klim-2026-10-17.json").canonicalize().unwrap()
        );
        assert_eq!(
            resolve_study_in(Path::new("other-sans"), &dirs)
                .unwrap()
                .canonicalize()
                .unwrap(),
            user.join("other-sans.json").canonicalize().unwrap()
        );
        let direct = user.join("other-sans.json");
        assert_eq!(resolve_study_in(&direct, &dirs).unwrap(), direct);
        write_named_study(&user, "klim-notes.json");
        let err = resolve_study_in(Path::new("klim"), &dirs)
            .unwrap_err()
            .to_string();
        assert!(err.contains("klim-2026-10-17"), "{err}");
        assert!(err.contains("klim-notes"), "{err}");
        assert!(resolve_study_in(Path::new("klim-2026-10-17"), &dirs).is_ok());
        let missing = resolve_study_in(Path::new("missing/file.json"), &dirs)
            .unwrap_err()
            .to_string();
        assert!(missing.contains("does not exist"), "{missing}");
        let unknown = resolve_study_in(Path::new("no-such-study"), &dirs)
            .unwrap_err()
            .to_string();
        assert!(unknown.contains("no study is named"), "{unknown}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_shipped_klim_study_selects_the_future() {
        let path = shipped_studies_dir().join("klim-2026-10-17.json");
        if !path.is_file() {
            return;
        }
        let library = read_study(&path).unwrap();
        let face = select_face(&library, "The Future", None, None).unwrap();
        assert_eq!(face.family, "Test The Future");
        assert_eq!(face.style, "Regular");
        assert_eq!(face.weight, 400);
        let listed = list_studies().unwrap();
        let klim = listed
            .iter()
            .find(|study| study.name == "klim-2026-10-17")
            .unwrap();
        assert_eq!(klim.place, "shipped");
        assert_eq!(klim.faces, library.faces.len());
        assert_eq!(klim.families, library.families.len());
        let resolved = resolve_study(Path::new("klim-2026-10-17")).unwrap();
        assert_eq!(
            resolved.canonicalize().unwrap(),
            path.canonicalize().unwrap()
        );
    }
}
