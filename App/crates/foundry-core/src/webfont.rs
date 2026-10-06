//! Web font import.
//!
//! A webfontjson file is a JSON or `callback({...})` document whose `css`
//! field holds `@font-face` rules. Each rule embeds a font as a base64 data
//! URI. `.ttf` and `.otf` bytes are read directly. WOFF 1 and WOFF2 bytes are
//! unpacked first. Embedded OpenType (`.eot`) is refused. Saving back over
//! `.otf`, `.woff`, or `.woff2` is refused so a JSON write cannot replace the
//! binary file.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine;
use serde_json::Value;
use ttf_parser::name_id;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::error::FoundryError;
use crate::font::{Contour, Font, Glyph, Point, PointKind};

pub(crate) fn is_binary_font_path(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("ttf" | "otf" | "woff" | "woff2" | "eot")
    )
}

pub(crate) fn is_readonly_web_font_path(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("otf" | "woff" | "woff2" | "eot")
    )
}

pub(crate) fn is_webfont_json(value: &Value) -> bool {
    value
        .get("css")
        .and_then(Value::as_str)
        .is_some_and(|css| css.to_ascii_lowercase().contains("@font-face"))
}

pub(crate) fn load_json_value(value: &Value) -> Result<Font, FoundryError> {
    let css = value
        .get("css")
        .and_then(Value::as_str)
        .ok_or_else(|| FoundryError::Import("web font JSON has no css field".to_string()))?;
    let faces = css_faces(css)?;
    if faces.is_empty() {
        return Err(FoundryError::Import(
            "web font JSON has no @font-face rule".to_string(),
        ));
    }
    let face = faces
        .iter()
        .find(|face| is_regular(&face.weight, &face.style))
        .unwrap_or(&faces[0]);
    load_binary_font(&face.bytes, Some(&face.label()))
}

pub(crate) fn load_binary_font(
    bytes: &[u8],
    preferred_name: Option<&str>,
) -> Result<Font, FoundryError> {
    if !bytes.starts_with(b"wOFF") && !bytes.starts_with(b"wOF2") && looks_like_eot(bytes) {
        return Err(FoundryError::Import(
            "Embedded OpenType is not imported".to_string(),
        ));
    }
    let sfnt = crate::sfnt::sfnt_bytes(bytes).map_err(FoundryError::Import)?;
    let face = Face::parse(&sfnt, 0)
        .map_err(|err| FoundryError::Import(format!("could not read the web font: {err}")))?;
    let upm = face.units_per_em();
    let preferred = preferred_name
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let name = preferred
        .map(str::to_string)
        .unwrap_or_else(|| face_name(&face));
    let mut font = Font::new(name, upm)?;
    font.metrics.ascender = f64::from(face.ascender());
    font.metrics.descender = f64::from(face.descender());
    font.metrics.baseline = 0.0;
    let scale = f64::from(upm) / 1000.0;
    font.metrics.cap_height = face
        .capital_height()
        .map(f64::from)
        .unwrap_or(700.0 * scale);
    font.metrics.x_height = face.x_height().map(f64::from).unwrap_or(500.0 * scale);

    let codes = unicode_map(&face);
    let mut glyphs = Vec::new();
    for index in 0..face.number_of_glyphs() {
        let id = GlyphId(index);
        let unicode = codes.get(&index).copied();
        let mut builder = Ink::default();
        if face.outline_glyph(id, &mut builder).is_none() {
            builder.contours.clear();
        }
        if builder.contours.is_empty() && unicode.is_none() && index != 0 {
            continue;
        }
        let name = unique_name(
            face.glyph_name(id)
                .filter(|name| !name.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| fallback_name(index, unicode)),
            index,
            &glyphs,
        );
        glyphs.push(Glyph {
            name,
            unicode,
            advance: f64::from(face.glyph_hor_advance(id).unwrap_or(0)),
            contours: builder.contours,
        });
    }
    glyphs.sort_by(|left, right| glyph_order(left).cmp(&glyph_order(right)));
    for glyph in glyphs {
        font.insert_glyph(glyph)?;
    }
    if let Some(top) = top_of(&font, "H") {
        font.metrics.cap_height = top;
    }
    if let Some(top) = top_of(&font, "x") {
        font.metrics.x_height = top;
    }
    font.validate()?;
    Ok(font)
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
}

fn looks_like_eot(bytes: &[u8]) -> bool {
    bytes.len() > 36 && bytes[34] == 0x4c && bytes[35] == 0x50
}

struct CssFace {
    family: String,
    weight: String,
    style: String,
    bytes: Vec<u8>,
}

impl CssFace {
    fn label(&self) -> String {
        let mut parts = vec![self.family.clone()];
        if !is_normal_weight(&self.weight) {
            parts.push(self.weight.clone());
        }
        if !is_normal_style(&self.style) {
            parts.push(self.style.clone());
        }
        let label = parts.join(" ");
        if label.is_empty() {
            "Imported".to_string()
        } else {
            label
        }
    }
}

fn is_regular(weight: &str, style: &str) -> bool {
    is_normal_weight(weight) && is_normal_style(style)
}

fn is_normal_weight(weight: &str) -> bool {
    let weight = weight.trim();
    weight.is_empty() || weight.eq_ignore_ascii_case("normal") || weight == "400"
}

fn is_normal_style(style: &str) -> bool {
    let style = style.trim();
    style.is_empty()
        || style.eq_ignore_ascii_case("normal")
        || style.eq_ignore_ascii_case("regular")
}

fn css_faces(css: &str) -> Result<Vec<CssFace>, FoundryError> {
    let mut faces = Vec::new();
    let lower = css.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(at) = lower[search_from..].find("@font-face") {
        let start = search_from + at;
        let Some(brace) = css[start..].find('{') else {
            break;
        };
        let body_start = start + brace + 1;
        let Some(end) = css[body_start..].find('}') else {
            return Err(FoundryError::Import(
                "a web font @font-face rule is missing its closing brace".to_string(),
            ));
        };
        let body = &css[body_start..body_start + end];
        faces.push(parse_face(body)?);
        search_from = body_start + end + 1;
        if search_from >= css.len() {
            break;
        }
    }
    Ok(faces)
}

fn parse_face(body: &str) -> Result<CssFace, FoundryError> {
    let (mime, payload, rest) = take_src(body)?;
    if mime.to_ascii_lowercase().contains("ms-fontobject") {
        return Err(FoundryError::Import(
            "Embedded OpenType is not imported".to_string(),
        ));
    }
    let cleaned: String = payload.chars().filter(|ch| !ch.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .map_err(|_| FoundryError::Import("a web font data URI is not valid base64".to_string()))?;
    let family = css_value(&rest, "font-family").unwrap_or_else(|| "Imported".to_string());
    Ok(CssFace {
        weight: css_value(&rest, "font-weight").unwrap_or_default(),
        style: css_value(&rest, "font-style").unwrap_or_default(),
        family,
        bytes,
    })
}

fn take_src(body: &str) -> Result<(String, String, String), FoundryError> {
    let lower = body.to_ascii_lowercase();
    let src_at = lower
        .find("src:")
        .ok_or_else(|| FoundryError::Import("a web font face has no src".to_string()))?;
    let data_at = lower[src_at..]
        .find("data:")
        .ok_or_else(|| FoundryError::Import("a web font src is not a data URI".to_string()))?
        + src_at;
    let marker = ";base64,";
    let encoded_at = lower[data_at..]
        .find(marker)
        .ok_or_else(|| FoundryError::Import("a web font data URI is not base64".to_string()))?
        + data_at;
    let payload_start = encoded_at + marker.len();
    let payload_end = body[payload_start..].find(')').ok_or_else(|| {
        FoundryError::Import("a web font data URI is missing its closing parenthesis".to_string())
    })? + payload_start;
    let mime = body[data_at + "data:".len()..encoded_at].to_string();
    let payload = body[payload_start..payload_end].to_string();
    let rest = format!("{}{}", &body[..src_at], &body[payload_end + 1..]);
    Ok((mime, payload, rest))
}

fn css_value(body: &str, property: &str) -> Option<String> {
    for piece in body.split(';') {
        let Some((name, value)) = piece.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(property) {
            let value = value.trim().trim_matches(|ch| ch == '\'' || ch == '"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn face_name(face: &Face<'_>) -> String {
    for wanted in [name_id::FULL_NAME, name_id::FAMILY] {
        for name in face.names() {
            if name.name_id == wanted
                && let Some(text) = name.to_string()
            {
                let text = text.trim();
                if !text.is_empty() {
                    return text.to_string();
                }
            }
        }
    }
    "Imported".to_string()
}

fn unicode_map(face: &Face<'_>) -> BTreeMap<u16, u32> {
    let mut codes = BTreeMap::new();
    let Some(cmap) = face.tables().cmap else {
        return codes;
    };
    for subtable in cmap.subtables {
        if !subtable.is_unicode() {
            continue;
        }
        subtable.codepoints(|code| {
            if let Some(id) = subtable.glyph_index(code) {
                codes.entry(id.0).or_insert(code);
            }
        });
    }
    codes
}

fn fallback_name(index: u16, unicode: Option<u32>) -> String {
    if index == 0 {
        return ".notdef".to_string();
    }
    unicode
        .map(name_for_unicode)
        .unwrap_or_else(|| format!("glyph{index}"))
}

fn name_for_unicode(code: u32) -> String {
    match char::from_u32(code) {
        Some(' ') => "space".to_string(),
        Some(ch) if !ch.is_control() && !ch.is_whitespace() => ch.to_string(),
        _ => format!("uni{code:04X}"),
    }
}

fn unique_name(name: String, index: u16, glyphs: &[Glyph]) -> String {
    if glyphs.iter().all(|glyph| glyph.name != name) {
        name
    } else {
        format!("{name}.{index}")
    }
}

fn glyph_order(glyph: &Glyph) -> (u8, u32, &str) {
    if glyph.name == ".notdef" {
        (0, 0, glyph.name.as_str())
    } else {
        (1, glyph.unicode.unwrap_or(u32::MAX), glyph.name.as_str())
    }
}

fn top_of(font: &Font, name: &str) -> Option<f64> {
    let glyph = font.glyph(name)?;
    glyph
        .contours
        .iter()
        .flat_map(|contour| contour.points.iter())
        .filter(|point| point.kind == PointKind::On)
        .map(|point| point.y)
        .max_by(f64::total_cmp)
}

#[derive(Default)]
struct Ink {
    contours: Vec<Contour>,
    current: Vec<Point>,
    closed: bool,
}

impl Ink {
    fn finish(&mut self) {
        if self.current.is_empty() {
            self.closed = false;
            return;
        }
        if self.current.len() >= 2 && same_point(&self.current[0], self.current.last().unwrap()) {
            self.current.pop();
            self.closed = true;
        }
        if !self.current.is_empty() {
            self.contours.push(Contour {
                closed: self.closed,
                points: std::mem::take(&mut self.current),
            });
        }
        self.closed = false;
    }
}

impl OutlineBuilder for Ink {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish();
        self.current.push(on(f64::from(x), f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.current.push(on(f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.current.push(off(f64::from(x1), f64::from(y1)));
        self.current.push(on(f64::from(x), f64::from(y)));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.current.push(off(f64::from(x1), f64::from(y1)));
        self.current.push(off(f64::from(x2), f64::from(y2)));
        self.current.push(on(f64::from(x), f64::from(y)));
    }

    fn close(&mut self) {
        self.closed = true;
        self.finish();
    }
}

fn on(x: f64, y: f64) -> Point {
    Point {
        x,
        y,
        kind: PointKind::On,
        smooth: false,
    }
}

fn off(x: f64, y: f64) -> Point {
    Point {
        x,
        y,
        kind: PointKind::Off,
        smooth: false,
    }
}

fn same_point(a: &Point, b: &Point) -> bool {
    (a.x - b.x).abs() <= 0.001 && (a.y - b.y).abs() <= 0.001
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::load_text;
    use crate::ttf::write_ttf;

    fn square() -> Font {
        let mut font = Font::new("Square", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "H".into(),
            unicode: Some(u32::from('H')),
            advance: 400.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    on(40.0, 0.0),
                    on(120.0, 0.0),
                    on(120.0, 120.0),
                    on(40.0, 120.0),
                ],
            }],
        })
        .unwrap();
        font
    }

    #[test]
    fn a_callback_wrapped_font_round_trips_the_square() {
        let bytes = write_ttf(&square()).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let text = format!(
            "fontsLoadedCallback({{\"css\":\"@font-face{{font-family:Square;src:url(data:font/opentype;base64,{encoded});font-weight:normal;}}\"}});"
        );
        let loaded = load_text(&text).unwrap();
        assert_eq!(loaded.name, "Square");
        let h = loaded.glyph("H").unwrap();
        assert_eq!(h.advance, 400.0);
        assert_eq!(h.contours[0].points[0], on(40.0, 0.0));
        assert_eq!(h.contours[0].points.len(), 4);
        assert!(h.contours[0].closed);
    }

    #[test]
    fn a_bold_face_is_skipped_when_a_regular_one_is_present() {
        let bytes = write_ttf(&square()).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let text = format!(
            "{{\"css\":\"@font-face{{font-family:Heavy;font-weight:700;src:url(data:font/ttf;base64,{encoded});}}@font-face{{font-family:Square;font-weight:normal;src:url(data:font/ttf;base64,{encoded});}}\"}}"
        );
        let loaded = load_text(&text).unwrap();
        assert_eq!(loaded.name, "Square");
    }

    #[test]
    fn an_embedded_woff_round_trips_the_square() {
        let ttf = write_ttf(&square()).unwrap();
        let woff = crate::sfnt::wrap_woff(&ttf, true);
        let encoded = base64::engine::general_purpose::STANDARD.encode(&woff);
        let text = format!(
            "fontsLoadedCallback({{\"css\":\"@font-face{{font-family:Square;src:url(data:font/woff;base64,{encoded});font-weight:normal;}}\"}});"
        );
        let loaded = load_text(&text).unwrap();
        assert_eq!(loaded.name, "Square");
        let h = loaded.glyph("H").unwrap();
        assert_eq!(h.advance, 400.0);
        assert_eq!(h.contours[0].points[0], on(40.0, 0.0));
        assert_eq!(h.contours[0].points.len(), 4);
    }

    #[test]
    fn names_a_broken_embedded_woff2() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"wOF2\0\x01\0\0rest");
        let text = format!(
            "{{\"css\":\"@font-face{{font-family:Old;src:url(data:font/woff2;base64,{encoded});}}\"}}"
        );
        let error = load_text(&text).unwrap_err();
        assert!(
            error.to_string().contains("WOFF2 could not be unpacked"),
            "{error}"
        );
    }

    #[test]
    fn refuses_embedded_opentype() {
        let text = r#"{"css":"@font-face{font-family:Old;src:url(data:application/vnd.ms-fontobject;base64,AAAA);}"}"#;
        let error = load_text(text).unwrap_err();
        assert!(error.to_string().contains("Embedded OpenType"));
    }
}
