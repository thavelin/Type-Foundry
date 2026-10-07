use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::FoundryError;

pub const FONT_FORMAT: &str = "typefoundry.font";
pub const FONT_VERSION: u32 = 1;
pub const MIN_UPM: u16 = 16;
pub const MAX_UPM: u16 = 16384;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PointKind {
    On,
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub kind: PointKind,
    pub smooth: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contour {
    pub closed: bool,
    pub points: Vec<Point>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unicode: Option<u32>,
    pub advance: f64,
    pub contours: Vec<Contour>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub ascender: f64,
    pub cap_height: f64,
    pub x_height: f64,
    pub baseline: f64,
    pub descender: f64,
}

/// Where a font sits in its family. Files without this block load as the Regular of a family
/// named after the font.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    /// The family name shared by every style, such as `Wide`.
    pub family: String,
    /// The style name, such as `Regular`, `Italic`, or `Bold Italic`.
    pub name: String,
    /// OS/2 weight class, 1 to 1000. 400 is Regular, 700 is Bold.
    pub weight: u16,
    pub italic: bool,
    /// Degrees counter-clockwise from vertical, as in UFO and the `post` table. A typical
    /// italic that leans right is negative, such as -12.
    pub italic_angle: f64,
    /// OS/2 usWidthClass. 1 is ultra-condensed, 5 is normal, 9 is ultra-expanded.
    #[serde(default = "normal_width")]
    pub width: u16,
}

fn normal_width() -> u16 {
    5
}

/// Name-table and OS/2 fields the designer sets. Empty strings are left out of the file, and a
/// TrueType export then uses a neutral unique id and a blank vendor id instead of a foundry code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FontInfo {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub copyright: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub designer: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license_url: String,
    /// Name ID 5, with or without a leading "Version ". Empty means `Version 1.000`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// Four-character OS/2 vendor id. Empty means four spaces.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vendor: String,
    /// Name ID 3. Empty means the PostScript name plus the version, with no foundry prefix.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unique_id: String,
}

impl FontInfo {
    pub fn is_empty(&self) -> bool {
        self.copyright.is_empty()
            && self.designer.is_empty()
            && self.license.is_empty()
            && self.license_url.is_empty()
            && self.version.is_empty()
            && self.vendor.is_empty()
            && self.unique_id.is_empty()
    }

    /// Name ID 5. An empty version is `Version 1.000`. A value that already starts with
    /// "Version" is kept as written.
    pub fn version_label(&self) -> String {
        let raw = self.version.trim();
        if raw.is_empty() {
            return "Version 1.000".to_string();
        }
        if raw.len() >= 7 && raw[..7].eq_ignore_ascii_case("version") {
            raw.to_string()
        } else {
            format!("Version {raw}")
        }
    }

    /// `head.fontRevision`, 16.16 fixed, from the first number in [`Self::version_label`].
    pub fn font_revision(&self) -> u32 {
        let label = self.version_label();
        let mut number = None;
        for word in label.split(|ch: char| !ch.is_ascii_digit() && ch != '.') {
            if word.is_empty() {
                continue;
            }
            if let Ok(value) = word.parse::<f64>()
                && value.is_finite()
                && value >= 0.0
            {
                number = Some(value);
                break;
            }
        }
        let fixed = number.unwrap_or(1.0) * 65536.0;
        if fixed <= 0.0 {
            0
        } else if fixed >= f64::from(u32::MAX) {
            u32::MAX
        } else {
            fixed.round() as u32
        }
    }

    /// Four ASCII bytes for OS/2. Empty or invalid becomes four spaces, not a foundry code.
    pub fn vendor_tag(&self) -> [u8; 4] {
        let bytes = self.vendor.as_bytes();
        if bytes.len() == 4
            && bytes
                .iter()
                .all(|byte| byte.is_ascii() && !byte.is_ascii_control())
        {
            [bytes[0], bytes[1], bytes[2], bytes[3]]
        } else {
            *b"    "
        }
    }

    /// Name ID 3. Empty becomes the PostScript name plus the version, with no foundry prefix.
    pub fn export_unique_id(&self, postscript: &str) -> String {
        let id = self.unique_id.trim();
        if id.is_empty() {
            format!("{postscript} {}", self.version_label())
        } else {
            id.to_string()
        }
    }
}

/// Fields for [`Font::set_info`]. `None` keeps the current value.
#[derive(Debug, Clone, Default)]
pub struct InfoUpdate {
    pub copyright: Option<String>,
    pub designer: Option<String>,
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub version: Option<String>,
    pub vendor: Option<String>,
    pub unique_id: Option<String>,
}

/// One kerning pair. `left` and `right` are glyph names or keys in [`Kerning::groups`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernPair {
    pub left: String,
    pub right: String,
    pub value: f64,
}

/// `f i` becomes the glyph named `fi`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ligature {
    pub glyphs: Vec<String>,
    pub name: String,
}

/// Groups, pairs, and ligatures. Absent on a font (`None`) means the file does not say, so a UFO
/// save leaves kerning that is already on disk. Present and empty clears it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Kerning {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pairs: Vec<KernPair>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ligatures: Vec<Ligature>,
}

impl Kerning {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.pairs.is_empty() && self.ligatures.is_empty()
    }
}

impl Default for Style {
    fn default() -> Self {
        Self {
            family: String::new(),
            name: "Regular".to_string(),
            weight: 400,
            italic: false,
            italic_angle: 0.0,
            width: normal_width(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Font {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub upm: u16,
    pub metrics: Metrics,
    #[serde(default)]
    pub style: Style,
    #[serde(default, skip_serializing_if = "FontInfo::is_empty")]
    pub info: FontInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kerning: Option<Kerning>,
    /// Feature-file text. `None` means a UFO save keeps `features.fea` already on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<String>,
    /// Open bag for Style Genome, DesignDecisions, and other foundry data. Unknown keys from
    /// older loaders are kept on round-trip once this field exists; keys we do not know yet
    /// still survive because they live in this map.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lib: BTreeMap<String, Value>,
    pub glyphs: Vec<Glyph>,
}

impl Font {
    pub fn new(name: impl Into<String>, upm: u16) -> Result<Self, FoundryError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(FoundryError::Name);
        }
        if !(MIN_UPM..=MAX_UPM).contains(&upm) {
            return Err(FoundryError::Upm(upm));
        }
        let scale = f64::from(upm) / 1000.0;
        let family = name.clone();
        Ok(Self {
            format: FONT_FORMAT.to_string(),
            version: FONT_VERSION,
            name,
            upm,
            metrics: Metrics {
                ascender: 800.0 * scale,
                cap_height: 700.0 * scale,
                x_height: 500.0 * scale,
                baseline: 0.0,
                descender: -200.0 * scale,
            },
            style: Style {
                family,
                ..Style::default()
            },
            info: FontInfo::default(),
            kerning: None,
            features: None,
            lib: BTreeMap::new(),
            glyphs: Vec::new(),
        })
    }

    pub fn glyph(&self, name: &str) -> Option<&Glyph> {
        self.glyphs.iter().find(|glyph| glyph.name == name)
    }

    pub fn glyph_mut(&mut self, name: &str) -> Option<&mut Glyph> {
        self.glyphs.iter_mut().find(|glyph| glyph.name == name)
    }

    pub fn glyph_names(&self) -> Vec<&str> {
        self.glyphs
            .iter()
            .map(|glyph| glyph.name.as_str())
            .collect()
    }

    /// Insert a glyph, or replace the glyph that already uses its name.
    pub fn insert_glyph(&mut self, glyph: Glyph) -> Result<(), FoundryError> {
        validate_glyph(&glyph)?;
        if let Some(code) = glyph.unicode {
            self.reject_duplicate_unicode(code, &glyph.name)?;
        }
        if let Some(existing) = self.glyph_mut(&glyph.name) {
            *existing = glyph;
        } else {
            self.glyphs.push(glyph);
        }
        Ok(())
    }

    pub(crate) fn reject_duplicate_unicode(
        &self,
        code: u32,
        name: &str,
    ) -> Result<(), FoundryError> {
        if let Some(other) = self
            .glyphs
            .iter()
            .find(|glyph| glyph.name != name && glyph.unicode == Some(code))
        {
            return Err(FoundryError::DuplicateUnicode {
                code,
                glyph: name.to_string(),
                other: other.name.clone(),
            });
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, FoundryError> {
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|err| FoundryError::Json(err.to_string()))?;
        text.push('\n');
        Ok(text)
    }

    pub fn from_json(text: &str) -> Result<Self, FoundryError> {
        let mut font: Self =
            serde_json::from_str(text).map_err(|err| FoundryError::Json(err.to_string()))?;
        font.fill_style();
        font.validate()?;
        Ok(font)
    }

    /// Read `typefoundry.font` JSON, a `.ufo` directory, a folder of SVG glyphs, face 0 of
    /// a `.ttf`, `.otf`, `.ttc`, or `.otc` file, or a WOFF 1 `.woff`.
    pub fn load(path: &Path) -> Result<Self, FoundryError> {
        if path.is_dir() {
            if crate::ufo::is_ufo_path(path) {
                return crate::ufo::load_ufo(path);
            }
            if crate::svgfont::is_svg_font_dir(path) {
                return crate::svgfont::load_svg_dir(path);
            }
            return Err(FoundryError::Import(format!(
                "{} is not a .ufo folder or a folder of SVG glyphs. Name each file with four hex digits, like 0041.svg for A.",
                path.display()
            )));
        }
        if crate::ufo::is_ufo_path(path) {
            return crate::ufo::load_ufo(path);
        }
        if crate::sfnt::is_font_binary_path(path) {
            return crate::sfnt::load_font_binary(path);
        }
        if crate::webfont::is_binary_font_path(path) {
            let bytes = fs::read(path).map_err(|err| FoundryError::Io(err.to_string()))?;
            return crate::webfont::load_binary_font(&bytes, None);
        }
        let text = fs::read_to_string(path).map_err(|err| FoundryError::Io(err.to_string()))?;
        crate::import::load_text(&text)
    }

    pub fn save(&self, path: &Path) -> Result<(), FoundryError> {
        self.save_with(path, false)
    }

    /// `force` replaces an existing file and keeps the previous one as `name.bak`.
    pub fn save_with(&self, path: &Path, force: bool) -> Result<(), FoundryError> {
        self.validate()?;
        crate::save::prepare_write(path, force)?;
        if crate::sfnt::is_read_only_path(path) {
            return Err(FoundryError::Import(format!(
                "{} can be opened but not written; save to .ttf, .ufo, or .json",
                path.display()
            )));
        }
        if crate::webfont::is_readonly_web_font_path(path) {
            return Err(FoundryError::Import(
                "save writes .json, .ufo, or .ttf. Use one of those paths after opening a web font."
                    .to_string(),
            ));
        }
        if crate::ufo::is_ufo_path(path) {
            return crate::ufo::save_ufo(self, path);
        }
        if crate::ttf::is_ttf_path(path) {
            return crate::ttf::save_ttf(self, path);
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|err| FoundryError::Io(err.to_string()))?;
        }
        fs::write(path, self.to_json()?).map_err(|err| FoundryError::Io(err.to_string()))
    }

    /// Replace the name-table fields that were passed. `None` keeps the current value.
    pub fn set_info(&mut self, update: InfoUpdate) -> Result<(), FoundryError> {
        let mut info = self.info.clone();
        if let Some(value) = update.copyright {
            info.copyright = value;
        }
        if let Some(value) = update.designer {
            info.designer = value;
        }
        if let Some(value) = update.license {
            info.license = value;
        }
        if let Some(value) = update.license_url {
            info.license_url = value;
        }
        if let Some(value) = update.version {
            info.version = value;
        }
        if let Some(value) = update.unique_id {
            info.unique_id = value;
        }
        if let Some(value) = update.vendor {
            let bytes = value.as_bytes();
            let valid = value.is_empty()
                || (bytes.len() == 4
                    && bytes
                        .iter()
                        .all(|byte| byte.is_ascii() && !byte.is_ascii_control()));
            if !valid {
                return Err(FoundryError::Style(format!(
                    "vendor id '{value}' must be four ASCII characters"
                )));
            }
            info.vendor = value;
        }
        self.info = info;
        Ok(())
    }

    /// Store kerning. An empty value clears pairs, groups, and ligatures.
    pub fn set_kerning(&mut self, kerning: Kerning) -> Result<(), FoundryError> {
        validate_kerning(&kerning)?;
        self.kerning = Some(kerning);
        Ok(())
    }

    pub fn add_kern_pair(
        &mut self,
        left: String,
        right: String,
        value: f64,
    ) -> Result<(), FoundryError> {
        if !value.is_finite() {
            return Err(FoundryError::NonFinite);
        }
        let mut kerning = self.kerning.clone().unwrap_or_default();
        if let Some(pair) = kerning
            .pairs
            .iter_mut()
            .find(|pair| pair.left == left && pair.right == right)
        {
            pair.value = value;
        } else {
            kerning.pairs.push(KernPair { left, right, value });
        }
        validate_kerning(&kerning)?;
        self.kerning = Some(kerning);
        Ok(())
    }

    pub fn set_kern_group(
        &mut self,
        name: String,
        members: Vec<String>,
    ) -> Result<(), FoundryError> {
        let mut kerning = self.kerning.clone().unwrap_or_default();
        kerning.groups.insert(name, members);
        validate_kerning(&kerning)?;
        self.kerning = Some(kerning);
        Ok(())
    }

    pub fn add_ligature(&mut self, glyphs: Vec<String>, name: String) -> Result<(), FoundryError> {
        let mut kerning = self.kerning.clone().unwrap_or_default();
        if let Some(existing) = kerning
            .ligatures
            .iter_mut()
            .find(|liga| liga.glyphs == glyphs)
        {
            existing.name = name;
        } else {
            kerning.ligatures.push(Ligature { glyphs, name });
        }
        validate_kerning(&kerning)?;
        self.kerning = Some(kerning);
        Ok(())
    }

    /// `None` keeps `features.fea` on a later UFO save. `Some` replaces it, including an empty string.
    pub fn set_features(&mut self, features: Option<String>) {
        self.features = features;
    }

    pub fn move_point(
        &mut self,
        name: &str,
        contour: usize,
        index: usize,
        x: f64,
        y: f64,
    ) -> Result<(), FoundryError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(FoundryError::NonFinite);
        }
        let Some(glyph) = self.glyph_mut(name) else {
            return Err(FoundryError::MissingGlyph(name.to_string()));
        };
        let Some(contour) = glyph.contours.get_mut(contour) else {
            return Err(FoundryError::MissingPoint);
        };
        let Some(point) = contour.points.get_mut(index) else {
            return Err(FoundryError::MissingPoint);
        };
        point.x = x;
        point.y = y;
        Ok(())
    }

    /// Give a font with no family name its own name as the family, and Regular as the style.
    pub(crate) fn fill_style(&mut self) {
        if self.style.family.trim().is_empty() {
            self.style.family = self.name.clone();
        }
        if self.style.name.trim().is_empty() {
            self.style.name = "Regular".to_string();
        }
    }

    /// `Family Style`, for display and the full-name record.
    pub fn full_name(&self) -> String {
        format!("{} {}", self.style.family, self.style.name)
    }

    /// `Family-Style` with spaces and punctuation removed, for file names and PostScript names.
    pub fn file_stem(&self) -> String {
        let squash = |text: &str| -> String {
            let mut name: String = text
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '.')
                .collect();
            while name.contains("..") {
                name = name.replace("..", ".");
            }
            name.trim_matches('.').to_string()
        };
        let family = squash(&self.style.family);
        let style = squash(&self.style.name);
        match (family.is_empty(), style.is_empty()) {
            (true, _) => "Font".to_string(),
            (false, true) => family,
            (false, false) => format!("{family}-{style}"),
        }
    }

    /// The four-style name pair older apps group by: a family of up to Regular, Italic, Bold,
    /// and Bold Italic. Other weights become their own legacy family, like `Wide Light`.
    pub fn legacy_names(&self) -> (String, String) {
        let bold = self.style.weight == 700;
        let ribbi = self.style.weight == 400 || bold;
        let style = match (bold, self.style.italic) {
            (true, true) => "Bold Italic",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (false, false) => "Regular",
        };
        if ribbi {
            return (self.style.family.clone(), style.to_string());
        }
        let extra: Vec<&str> = self
            .style
            .name
            .split_whitespace()
            .filter(|word| !word.eq_ignore_ascii_case("italic"))
            .collect();
        let family = if extra.is_empty() {
            format!("{} W{}", self.style.family, self.style.weight)
        } else {
            format!("{} {}", self.style.family, extra.join(" "))
        };
        let style = if self.style.italic {
            "Italic"
        } else {
            "Regular"
        };
        (family, style.to_string())
    }

    pub(crate) fn validate(&self) -> Result<(), FoundryError> {
        if self.format != FONT_FORMAT {
            return Err(FoundryError::Format(self.format.clone()));
        }
        if self.version != FONT_VERSION {
            return Err(FoundryError::Version(self.version));
        }
        if self.name.trim().is_empty() {
            return Err(FoundryError::Name);
        }
        if !(MIN_UPM..=MAX_UPM).contains(&self.upm) {
            return Err(FoundryError::Upm(self.upm));
        }
        if !metrics_are_finite(&self.metrics) || !self.style.italic_angle.is_finite() {
            return Err(FoundryError::NonFinite);
        }
        if !(1..=1000).contains(&self.style.weight) {
            return Err(FoundryError::Style(format!(
                "weight {} is outside 1-1000",
                self.style.weight
            )));
        }
        if !(1..=9).contains(&self.style.width) {
            return Err(FoundryError::Style(format!(
                "width class {} is outside 1-9",
                self.style.width
            )));
        }
        if !self.info.vendor.is_empty() {
            let bytes = self.info.vendor.as_bytes();
            let ok = bytes.len() == 4
                && bytes
                    .iter()
                    .all(|byte| byte.is_ascii() && !byte.is_ascii_control());
            if !ok {
                return Err(FoundryError::Style(format!(
                    "vendor id '{}' must be four ASCII characters",
                    self.info.vendor
                )));
            }
        }
        let mut seen = BTreeSet::new();
        let mut codes = BTreeSet::new();
        for glyph in &self.glyphs {
            if !seen.insert(glyph.name.clone()) {
                return Err(FoundryError::DuplicateGlyph(glyph.name.clone()));
            }
            if let Some(code) = glyph.unicode
                && !codes.insert(code)
            {
                let other = self
                    .glyphs
                    .iter()
                    .find(|other| other.name != glyph.name && other.unicode == Some(code))
                    .map(|other| other.name.as_str())
                    .unwrap_or("another glyph");
                return Err(FoundryError::DuplicateUnicode {
                    code,
                    glyph: glyph.name.clone(),
                    other: other.to_string(),
                });
            }
            validate_glyph(glyph)?;
        }
        if let Some(kerning) = &self.kerning {
            validate_kerning(kerning)?;
        }
        Ok(())
    }
}

fn validate_kerning(kerning: &Kerning) -> Result<(), FoundryError> {
    let mut first: BTreeSet<&str> = BTreeSet::new();
    let mut second: BTreeSet<&str> = BTreeSet::new();
    for (name, members) in &kerning.groups {
        if name.trim().is_empty() {
            return Err(FoundryError::Edit("a kerning group has no name".into()));
        }
        let kern1 = name.starts_with("public.kern1.");
        let kern2 = name.starts_with("public.kern2.");
        for member in members {
            if member.trim().is_empty() {
                return Err(FoundryError::Edit(format!(
                    "kerning group {name} has an empty member"
                )));
            }
            let slot = if kern1 {
                Some(&mut first)
            } else if kern2 {
                Some(&mut second)
            } else {
                None
            };
            if let Some(slot) = slot {
                if slot.contains(member.as_str()) {
                    return Err(FoundryError::Edit(format!(
                        "{member} is in more than one group on that side of kerning"
                    )));
                }
                slot.insert(member.as_str());
            }
        }
    }
    let mut pairs = BTreeSet::new();
    for pair in &kerning.pairs {
        if pair.left.trim().is_empty() || pair.right.trim().is_empty() {
            return Err(FoundryError::Edit(
                "a kerning pair is missing a side".into(),
            ));
        }
        if !pair.value.is_finite() {
            return Err(FoundryError::NonFinite);
        }
        if !pairs.insert((pair.left.clone(), pair.right.clone())) {
            return Err(FoundryError::Edit(format!(
                "kerning pair {} {} is listed twice",
                pair.left, pair.right
            )));
        }
    }
    for liga in &kerning.ligatures {
        if liga.glyphs.len() < 2 || liga.name.trim().is_empty() {
            return Err(FoundryError::Edit(format!(
                "ligature {} needs a name and at least two glyphs",
                liga.name
            )));
        }
    }
    Ok(())
}

pub fn validate_glyph(glyph: &Glyph) -> Result<(), FoundryError> {
    if glyph.name.trim().is_empty() {
        return Err(FoundryError::GlyphName);
    }
    if !glyph.advance.is_finite() {
        return Err(FoundryError::NonFinite);
    }
    for contour in &glyph.contours {
        if contour.points.is_empty() {
            return Err(FoundryError::EmptyContour(glyph.name.clone()));
        }
        for point in &contour.points {
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(FoundryError::NonFinite);
            }
        }
    }
    Ok(())
}

fn metrics_are_finite(metrics: &Metrics) -> bool {
    metrics.ascender.is_finite()
        && metrics.cap_height.is_finite()
        && metrics.x_height.is_finite()
        && metrics.baseline.is_finite()
        && metrics.descender.is_finite()
}
