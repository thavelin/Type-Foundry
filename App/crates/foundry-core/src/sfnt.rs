//! Binary font import read by [`Font::load`]: TrueType (`.ttf`), OpenType with CFF or CFF2
//! outlines (`.otf`), the first face of a collection (`.ttc`, `.otc`), and WOFF 1 (`.woff`).
//!
//! Outlines arrive as the parser draws them. Composite glyphs are decomposed, TrueType implied
//! on-curve points become real on-curve points, and a variable font gives its default instance.
//! WOFF 1 and WOFF2 are unpacked into an sfnt, then read the same way.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use ttf_parser::{Face, GlyphId, OutlineBuilder, Tag, name_id};

use crate::error::FoundryError;
use crate::font::{
    Contour, Font, Glyph, KernPair, Kerning, Ligature, MAX_UPM, MIN_UPM, Point, PointKind,
    validate_glyph,
};

/// Windows language ID for English (United States) in the `name` table.
const ENGLISH_US: u16 = 0x0409;
/// Extensions [`Font::load`] reads as a binary font.
const IMPORT_EXTENSIONS: [&str; 6] = ["ttf", "otf", "ttc", "otc", "woff", "woff2"];
const WOFF_HEADER: usize = 48;
const WOFF_ENTRY: usize = 20;

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
}

/// True when [`Font::load`] should read the path as a binary font, including WOFF2 so it can
/// refuse it by name.
pub(crate) fn is_font_binary_path(path: &Path) -> bool {
    extension(path).is_some_and(|ext| IMPORT_EXTENSIONS.contains(&ext.as_str()))
}

/// True for a binary format that can be opened but not written. `.ttf` is written by the
/// TrueType exporter, so it is not in this list.
pub(crate) fn is_read_only_path(path: &Path) -> bool {
    extension(path).is_some_and(|ext| ext != "ttf" && IMPORT_EXTENSIONS.contains(&ext.as_str()))
}

pub(crate) fn load_font_binary(path: &Path) -> Result<Font, FoundryError> {
    let bytes = fs::read(path).map_err(|err| FoundryError::Io(err.to_string()))?;
    let fallback = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    read_font_binary(&bytes, &fallback)
        .map_err(|message| FoundryError::Import(format!("{}: {message}", path.display())))
}

/// Read face 0 of TrueType, OpenType, a collection, or WOFF 1. `fallback_name` names the font
/// when the file has no family name.
pub(crate) fn read_font_binary(bytes: &[u8], fallback_name: &str) -> Result<Font, String> {
    let sfnt = sfnt_bytes(bytes)?;
    let face = Face::parse(&sfnt, 0).map_err(|err| format!("not a readable font: {err}"))?;

    let upm = face.units_per_em();
    if !(MIN_UPM..=MAX_UPM).contains(&upm) {
        return Err(format!("units per em {upm} is outside {MIN_UPM}-{MAX_UPM}"));
    }
    let names = family_and_style(&face);
    let name = match &names {
        Some((family, Some(style))) => format!("{family} {style}"),
        Some((family, None)) => family.clone(),
        None => fallback_name.to_string(),
    };
    let mut font = Font::new(name, upm).map_err(|err| err.to_string())?;
    if let Some((family, style)) = names {
        font.style.family = family;
        font.style.name = style.unwrap_or_else(|| "Regular".to_string());
    }
    font.style.weight = face.weight().to_number().clamp(1, 1000);
    font.style.width = face.width().to_number().clamp(1, 9);
    font.style.italic = face.is_italic();
    font.style.italic_angle = f64::from(face.italic_angle());
    apply_binary_info(&mut font, &face);
    font.metrics.ascender = f64::from(face.ascender());
    font.metrics.descender = f64::from(face.descender());
    if let Some(cap_height) = face.capital_height().filter(|value| *value > 0) {
        font.metrics.cap_height = f64::from(cap_height);
    }
    if let Some(x_height) = face.x_height().filter(|value| *value > 0) {
        font.metrics.x_height = f64::from(x_height);
    }
    font.metrics.baseline = 0.0;

    let unicodes = unicode_map(&face);
    let post_names = post_names(&face);
    let mut taken = BTreeMap::new();
    let mut glyphs = Vec::with_capacity(usize::from(face.number_of_glyphs()));
    for index in 0..face.number_of_glyphs() {
        let id = GlyphId(index);
        let unicode = unicodes.get(&index).copied();
        let stored = post_names
            .as_ref()
            .and_then(|names| names.get(usize::from(index)).cloned().flatten());
        let name = match stored {
            Some(name) => unique_name(name, index, &mut taken),
            None => unique_name(glyph_name(&face, id, unicode), index, &mut taken),
        };
        let mut pen = Pen::default();
        // `None` means the glyph has no outline, such as a space. That is not an error.
        let _ = face.outline_glyph(id, &mut pen);
        let glyph = Glyph {
            name,
            unicode,
            advance: f64::from(face.glyph_hor_advance(id).unwrap_or(0)),
            contours: pen.finish(),
        };
        // Names are already unique, so skip insert_glyph's name search, which is slow for
        // fonts with tens of thousands of glyphs.
        validate_glyph(&glyph).map_err(|err| err.to_string())?;
        glyphs.push(glyph);
    }
    font.glyphs = glyphs;
    import_kern(&face, &mut font);
    import_ligatures(&face, &mut font);
    Ok(font)
}

fn apply_binary_info(font: &mut Font, face: &Face<'_>) {
    let lookup = |id: u16| name_text(face, id);
    if let Some(text) = lookup(name_id::COPYRIGHT_NOTICE) {
        font.info.copyright = text;
    }
    if let Some(text) = lookup(name_id::UNIQUE_ID) {
        font.info.unique_id = text;
    }
    if let Some(text) = lookup(name_id::VERSION) {
        font.info.version = text;
    }
    if let Some(text) = lookup(name_id::DESIGNER) {
        font.info.designer = text;
    }
    if let Some(text) = lookup(name_id::LICENSE) {
        font.info.license = text;
    }
    if let Some(text) = lookup(name_id::LICENSE_URL) {
        font.info.license_url = text;
    }
    if let Some(os2) = face.raw_face().table(Tag::from_bytes(b"OS/2"))
        && os2.len() >= 62
    {
        let vendor = &os2[58..62];
        if vendor.iter().all(|byte| byte.is_ascii()) {
            let text = String::from_utf8_lossy(vendor).trim().to_string();
            if !text.is_empty() {
                font.info.vendor = text;
            }
        }
    }
}

fn name_text(face: &Face<'_>, id: u16) -> Option<String> {
    let read = |english_only: bool| {
        face.names()
            .into_iter()
            .filter(|name| name.name_id == id && name.is_unicode())
            .filter(|name| !english_only || name.language_id == ENGLISH_US)
            .find_map(|name| name.to_string())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    };
    read(true).or_else(|| read(false))
}

fn import_kern(face: &Face<'_>, font: &mut Font) {
    let Some(table) = face.tables().kern else {
        return;
    };
    let mut pairs = Vec::new();
    for subtable in table.subtables {
        if !subtable.horizontal {
            continue;
        }
        let ttf_parser::kern::Format::Format0(format) = subtable.format else {
            continue;
        };
        for pair in format.pairs {
            let left = usize::from(pair.left().0);
            let right = usize::from(pair.right().0);
            let Some(left_name) = font.glyphs.get(left).map(|glyph| glyph.name.clone()) else {
                continue;
            };
            let Some(right_name) = font.glyphs.get(right).map(|glyph| glyph.name.clone()) else {
                continue;
            };
            let value = f64::from(pair.value);
            if value == 0.0 {
                continue;
            }
            pairs.push(KernPair {
                left: left_name,
                right: right_name,
                value,
            });
        }
    }
    if pairs.is_empty() {
        return;
    }
    let kerning = font.kerning.get_or_insert_with(Kerning::default);
    kerning.pairs = pairs;
}

fn import_ligatures(face: &Face<'_>, font: &mut Font) {
    let Some(gsub) = face.tables().gsub else {
        return;
    };
    let mut ligatures = Vec::new();
    for lookup in gsub.lookups {
        for subtable in lookup
            .subtables
            .into_iter::<ttf_parser::gsub::SubstitutionSubtable>()
        {
            let ttf_parser::gsub::SubstitutionSubtable::Ligature(liga) = subtable else {
                continue;
            };
            for (index, glyph) in font.glyphs.iter().enumerate() {
                let Some(coverage) = liga.coverage.get(GlyphId(index as u16)) else {
                    continue;
                };
                let Some(set) = liga.ligature_sets.get(coverage) else {
                    continue;
                };
                let mut slot = 0u16;
                while let Some(lig) = set.get(slot) {
                    slot += 1;
                    let mut names = vec![glyph.name.clone()];
                    let mut complete = true;
                    for component in lig.components {
                        match font.glyphs.get(usize::from(component.0)) {
                            Some(component_glyph) => names.push(component_glyph.name.clone()),
                            None => complete = false,
                        }
                    }
                    let Some(target) = font.glyphs.get(usize::from(lig.glyph.0)) else {
                        continue;
                    };
                    if complete && names.len() >= 2 {
                        ligatures.push(Ligature {
                            glyphs: names,
                            name: target.name.clone(),
                        });
                    }
                }
            }
        }
    }
    if ligatures.is_empty() {
        return;
    }
    let kerning = font.kerning.get_or_insert_with(Kerning::default);
    kerning.ligatures = ligatures;
}

/// Bytes `ttf-parser` can read. WOFF 1 and WOFF2 are unpacked into an sfnt. Anything else is
/// returned as-is.
pub(crate) fn sfnt_bytes(bytes: &[u8]) -> Result<Cow<'_, [u8]>, String> {
    match bytes.get(..4) {
        Some(b"wOFF") => Ok(Cow::Owned(unpack_woff(bytes)?)),
        Some(b"wOF2") => wuff::decompress_woff2(bytes)
            .map(Cow::Owned)
            .map_err(|err| format!("WOFF2 could not be unpacked: {err}")),
        _ => Ok(Cow::Borrowed(bytes)),
    }
}

struct WoffTable {
    tag: [u8; 4],
    checksum: u32,
    data: Vec<u8>,
}

/// Rebuild the sfnt a WOFF 1 file wraps. Compressed tables use zlib (RFC 1950). A table stored
/// with `compLength == origLength` is copied as-is. Metadata and private blocks are skipped.
fn unpack_woff(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < WOFF_HEADER {
        return Err("WOFF header is truncated".to_string());
    }
    let file_len =
        u32::try_from(bytes.len()).map_err(|_| "WOFF is larger than 4 GiB".to_string())?;
    if be_u32(bytes, 8) != file_len {
        return Err("WOFF length does not match the file".to_string());
    }
    if be_u16(bytes, 14) != 0 {
        return Err("WOFF reserved field is not zero".to_string());
    }
    let num_tables = usize::from(be_u16(bytes, 12));
    if num_tables == 0 {
        return Err("WOFF has no tables".to_string());
    }
    let dir_end = WOFF_HEADER + num_tables * WOFF_ENTRY;
    if bytes.len() < dir_end {
        return Err("WOFF table directory is truncated".to_string());
    }

    let mut tables = Vec::with_capacity(num_tables);
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for index in 0..num_tables {
        let at = WOFF_HEADER + index * WOFF_ENTRY;
        let tag: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
        let offset = usize::try_from(be_u32(bytes, at + 4)).unwrap_or(usize::MAX);
        let comp_len = usize::try_from(be_u32(bytes, at + 8)).unwrap_or(usize::MAX);
        let orig_len = usize::try_from(be_u32(bytes, at + 12)).unwrap_or(usize::MAX);
        let checksum = be_u32(bytes, at + 16);
        if comp_len > orig_len {
            return Err(format!(
                "WOFF table {} is larger compressed than stored",
                tag_name(&tag)
            ));
        }
        let data = if orig_len == 0 {
            Vec::new()
        } else {
            if offset % 4 != 0 {
                return Err(format!(
                    "WOFF table {} is not 4-byte aligned",
                    tag_name(&tag)
                ));
            }
            let end = offset
                .checked_add(comp_len)
                .ok_or_else(|| format!("WOFF table {} offset overflows", tag_name(&tag)))?;
            if offset < dir_end || end > bytes.len() {
                return Err(format!("WOFF table {} is outside the file", tag_name(&tag)));
            }
            if spans.iter().any(|&(lo, hi)| offset < hi && lo < end) {
                return Err(format!(
                    "WOFF table {} overlaps another table",
                    tag_name(&tag)
                ));
            }
            spans.push((offset, end));
            if comp_len == orig_len {
                bytes[offset..end].to_vec()
            } else {
                inflate(&bytes[offset..end], orig_len, &tag)?
            }
        };
        tables.push(WoffTable {
            tag,
            checksum,
            data,
        });
    }
    tables.sort_by_key(|table| table.tag);
    for pair in tables.windows(2) {
        if pair[0].tag == pair[1].tag {
            return Err(format!("WOFF repeats table {}", tag_name(&pair[0].tag)));
        }
    }
    assemble_sfnt(be_u32(bytes, 4), &tables)
}

fn inflate(src: &[u8], orig_len: usize, tag: &[u8; 4]) -> Result<Vec<u8>, String> {
    let mut decoder = flate2::read::ZlibDecoder::new(src).take(orig_len as u64 + 1);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|err| format!("WOFF table {} did not inflate: {err}", tag_name(tag)))?;
    if out.len() != orig_len {
        return Err(format!(
            "WOFF table {} inflated to {} bytes, not {orig_len}",
            tag_name(tag),
            out.len()
        ));
    }
    Ok(out)
}

fn assemble_sfnt(flavor: u32, tables: &[WoffTable]) -> Result<Vec<u8>, String> {
    let count = u16::try_from(tables.len()).map_err(|_| "WOFF has too many tables".to_string())?;
    let (pow, log, _) = crate::ttf::power_of_two(count);
    let search = pow.saturating_mul(16);
    let shift = count.saturating_mul(16).saturating_sub(search);
    let mut out = Vec::new();
    push_u32(&mut out, flavor);
    push_u16(&mut out, count);
    push_u16(&mut out, search);
    push_u16(&mut out, log);
    push_u16(&mut out, shift);

    let mut offset = 12 + tables.len() * 16;
    let mut body = Vec::new();
    for table in tables {
        let at = u32::try_from(offset).map_err(|_| "rebuilt sfnt is too large".to_string())?;
        let len =
            u32::try_from(table.data.len()).map_err(|_| "WOFF table is too large".to_string())?;
        out.extend_from_slice(&table.tag);
        push_u32(&mut out, table.checksum);
        push_u32(&mut out, at);
        push_u32(&mut out, len);
        let padded = table.data.len().div_ceil(4) * 4;
        body.extend_from_slice(&table.data);
        body.resize(body.len() + padded - table.data.len(), 0);
        offset += padded;
    }
    out.extend(body);
    Ok(out)
}

fn tag_name(tag: &[u8; 4]) -> String {
    if tag
        .iter()
        .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
    {
        String::from_utf8_lossy(tag).into_owned()
    } else {
        tag.iter().map(|byte| format!("{byte:02X}")).collect()
    }
}

fn be_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn be_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend(value.to_be_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_be_bytes());
}

/// Typographic family and style when present, else the legacy pair.
fn family_and_style(face: &Face<'_>) -> Option<(String, Option<String>)> {
    // Prefer the Windows English (United States) record, then any readable one.
    let lookup = |id: u16| {
        let read = |english_only: bool| {
            face.names()
                .into_iter()
                .filter(|name| name.name_id == id && name.is_unicode())
                .filter(|name| !english_only || name.language_id == ENGLISH_US)
                .find_map(|name| name.to_string())
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
        };
        read(true).or_else(|| read(false))
    };
    let family = lookup(name_id::TYPOGRAPHIC_FAMILY).or_else(|| lookup(name_id::FAMILY))?;
    let style = lookup(name_id::TYPOGRAPHIC_SUBFAMILY).or_else(|| lookup(name_id::SUBFAMILY));
    Some((family, style))
}

/// The lowest Unicode value mapped to each glyph. Other values for the same glyph are dropped.
fn unicode_map(face: &Face<'_>) -> BTreeMap<u16, u32> {
    let mut map: BTreeMap<u16, u32> = BTreeMap::new();
    let Some(cmap) = face.tables().cmap else {
        return map;
    };
    for subtable in cmap.subtables {
        if !subtable.is_unicode() {
            continue;
        }
        subtable.codepoints(|codepoint| {
            if let Some(id) = subtable.glyph_index(codepoint) {
                map.entry(id.0)
                    .and_modify(|best| *best = (*best).min(codepoint))
                    .or_insert(codepoint);
            }
        });
    }
    map
}

/// Glyph names from a format 2 `post` table, read in one pass. `ttf-parser` finds each custom
/// name by walking the string list from the start, which is quadratic for large fonts.
fn post_names(face: &Face<'_>) -> Option<Vec<Option<String>>> {
    const STANDARD_NAMES: u16 = 258;
    let post = face.tables().post?;
    let raw = face.raw_face().table(Tag::from_bytes(b"post"))?;
    let read_u16 = |at: usize| {
        raw.get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    if raw.get(..4)? != [0, 2, 0, 0] {
        return None;
    }
    let count = read_u16(32)?;
    let custom: Vec<&str> = post.names().collect();
    let names = (0..count)
        .map(|glyph| {
            let index = read_u16(34 + 2 * usize::from(glyph))?;
            let name = if index < STANDARD_NAMES {
                post.glyph_name(GlyphId(glyph))?
            } else {
                custom.get(usize::from(index - STANDARD_NAMES)).copied()?
            };
            let name = name.trim();
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect();
    Some(names)
}

fn glyph_name(face: &Face<'_>, id: GlyphId, unicode: Option<u32>) -> String {
    if let Some(name) = face
        .glyph_name(id)
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return name.to_string();
    }
    match unicode {
        Some(code) if code <= 0xFFFF => format!("uni{code:04X}"),
        Some(code) => format!("u{code:05X}"),
        None if id.0 == 0 => ".notdef".to_string(),
        None => format!("glyph{:05}", id.0),
    }
}

fn unique_name(name: String, index: u16, taken: &mut BTreeMap<String, u16>) -> String {
    let name = if taken.contains_key(&name) {
        format!("{name}.{index}")
    } else {
        name
    };
    taken.insert(name.clone(), index);
    name
}

/// Collects drawn outlines as closed contours of on and off points.
#[derive(Default)]
struct Pen {
    contours: Vec<Contour>,
    points: Vec<Point>,
}

impl Pen {
    fn push(&mut self, x: f32, y: f32, kind: PointKind) {
        self.points.push(Point {
            x: f64::from(x),
            y: f64::from(y),
            kind,
            smooth: false,
        });
    }

    fn end_contour(&mut self) {
        let mut points = std::mem::take(&mut self.points);
        // The parser draws back to the start point. A closed contour does not repeat it, and any
        // off-points before it now wrap to the first point.
        if points.len() > 1
            && let (Some(first), Some(last)) = (points.first(), points.last())
            && first.x == last.x
            && first.y == last.y
            && last.kind == PointKind::On
        {
            points.pop();
        }
        if !points.is_empty() {
            self.contours.push(Contour {
                closed: true,
                points,
            });
        }
    }

    fn finish(mut self) -> Vec<Contour> {
        self.end_contour();
        self.contours
    }
}

impl OutlineBuilder for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.end_contour();
        self.push(x, y, PointKind::On);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.push(x, y, PointKind::On);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.push(x1, y1, PointKind::Off);
        self.push(x, y, PointKind::On);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.push(x1, y1, PointKind::Off);
        self.push(x2, y2, PointKind::Off);
        self.push(x, y, PointKind::On);
    }

    fn close(&mut self) {
        self.end_contour();
    }
}

/// Wrap an sfnt as WOFF 1. When `compress` is set, a table is zlib-compressed only when that
/// makes it smaller, which is what the WOFF spec allows.
#[cfg(test)]
pub(crate) fn wrap_woff(sfnt: &[u8], compress: bool) -> Vec<u8> {
    let num = usize::from(u16::from_be_bytes([sfnt[4], sfnt[5]]));
    let mut entries = Vec::with_capacity(num);
    for index in 0..num {
        let at = 12 + index * 16;
        let tag = sfnt[at..at + 4].to_vec();
        let checksum = u32::from_be_bytes(sfnt[at + 4..at + 8].try_into().unwrap());
        let offset = u32::from_be_bytes(sfnt[at + 8..at + 12].try_into().unwrap()) as usize;
        let length = u32::from_be_bytes(sfnt[at + 12..at + 16].try_into().unwrap()) as usize;
        let data = &sfnt[offset..offset + length];
        let stored = if compress {
            let deflated = zlib_compress(data);
            if deflated.len() < data.len() {
                deflated
            } else {
                data.to_vec()
            }
        } else {
            data.to_vec()
        };
        entries.push((tag, checksum, length, stored));
    }

    let data_at = 48 + num * 20;
    let mut directory = Vec::new();
    let mut body = Vec::new();
    for (tag, checksum, orig_len, stored) in &entries {
        let cursor = data_at + body.len();
        directory.extend_from_slice(tag);
        directory.extend_from_slice(&(cursor as u32).to_be_bytes());
        directory.extend_from_slice(&(stored.len() as u32).to_be_bytes());
        directory.extend_from_slice(&(*orig_len as u32).to_be_bytes());
        directory.extend_from_slice(&checksum.to_be_bytes());
        body.extend_from_slice(stored);
        while (data_at + body.len()) % 4 != 0 {
            body.push(0);
        }
    }
    let total = data_at + body.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"wOFF");
    out.extend_from_slice(&sfnt[..4]);
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(&(num as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(sfnt.len() as u32).to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&[0u8; 24]);
    out.extend(directory);
    out.extend(body);
    assert_eq!(out.len(), total);
    out
}

#[cfg(test)]
fn zlib_compress(data: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use std::io::Write;
    let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend_fonts;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_IDS: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = TEMP_IDS.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("typefoundry-sfnt-{tick}-{id}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn point(x: f64, y: f64, kind: PointKind) -> Point {
        Point {
            x,
            y,
            kind,
            smooth: false,
        }
    }

    fn face(name: &str, shift: f64, advance: f64) -> Font {
        let mut font = Font::new(name, 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "H".into(),
            unicode: Some(u32::from('H')),
            advance,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    point(40.0 + shift, 0.0, PointKind::On),
                    point(40.0 + shift, 700.0, PointKind::On),
                    point(160.0 + shift, 700.0, PointKind::On),
                    point(160.0 + shift, 0.0, PointKind::On),
                ],
            }],
        })
        .unwrap();
        font.insert_glyph(Glyph {
            name: "o".into(),
            unicode: Some(u32::from('o')),
            advance: 500.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    point(50.0, 0.0, PointKind::On),
                    point(250.0 + shift, 0.0, PointKind::On),
                    point(250.0 + shift, 250.0, PointKind::Off),
                    point(50.0, 250.0, PointKind::On),
                ],
            }],
        })
        .unwrap();
        font.insert_glyph(Glyph {
            name: "space".into(),
            unicode: Some(32),
            advance: 250.0,
            contours: Vec::new(),
        })
        .unwrap();
        font
    }

    #[test]
    fn a_saved_ttf_opens_with_names_unicodes_and_points() {
        let dir = temp_dir();
        let path = dir.join("Round.ttf");
        let source = face("Round", 0.0, 400.0);
        source.save(&path).unwrap();

        let loaded = Font::load(&path).unwrap();
        assert_eq!(loaded.name, "Round Regular");
        assert_eq!(loaded.upm, 1000);
        assert_eq!(loaded.glyph_names(), vec![".notdef", "H", "o", "space"]);

        let h = loaded.glyph("H").unwrap();
        assert_eq!(h.unicode, Some(72));
        assert_eq!(h.advance, 400.0);
        assert_eq!(h.contours, source.glyph("H").unwrap().contours);

        let o = loaded.glyph("o").unwrap();
        let kinds: Vec<PointKind> = o.contours[0].points.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            vec![PointKind::On, PointKind::On, PointKind::Off, PointKind::On]
        );
        assert_eq!(o.contours, source.glyph("o").unwrap().contours);

        let space = loaded.glyph("space").unwrap();
        assert!(space.contours.is_empty());
        assert_eq!(space.advance, 250.0);

        // Saving the import again keeps one .notdef and the same glyphs.
        let again = dir.join("Again.ttf");
        loaded.save(&again).unwrap();
        assert_eq!(Font::load(&again).unwrap().glyphs, loaded.glyphs);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn two_ttfs_blend_at_the_midpoint() {
        let dir = temp_dir();
        let narrow = dir.join("Narrow.ttf");
        let wide = dir.join("Wide.ttf");
        face("Narrow", 0.0, 400.0).save(&narrow).unwrap();
        face("Wide", 100.0, 800.0).save(&wide).unwrap();

        let mid = blend_fonts(
            &Font::load(&narrow).unwrap(),
            &Font::load(&wide).unwrap(),
            0.5,
        )
        .unwrap();
        let h = mid.glyph("H").unwrap();
        assert_eq!(h.advance, 600.0);
        assert_eq!(h.contours[0].points[0].x, 90.0);

        let _ = fs::remove_dir_all(dir);
    }

    // A minimal OpenType font with CFF outlines, built byte by byte so no third-party font is
    // needed. Glyph 1 is named `o` through a custom charset and draws one cubic.
    fn cff_font() -> Vec<u8> {
        fn index(items: &[&[u8]]) -> Vec<u8> {
            let mut out = (items.len() as u16).to_be_bytes().to_vec();
            if items.is_empty() {
                return out;
            }
            out.push(1);
            let mut offset = 1u8;
            out.push(offset);
            for item in items {
                offset += item.len() as u8;
                out.push(offset);
            }
            for item in items {
                out.extend_from_slice(item);
            }
            out
        }
        fn int(value: i32) -> Vec<u8> {
            let mut out = vec![29];
            out.extend_from_slice(&value.to_be_bytes());
            out
        }
        let small = |value: i32| (value + 139) as u8;

        let notdef: Vec<u8> = vec![14];
        let glyph: Vec<u8> = vec![
            small(10),
            small(0),
            21, // rmoveto to (10, 0)
            small(80),
            small(0),
            5, // rlineto to (90, 0)
            small(0),
            small(50),
            small(-30),
            small(50),
            small(-50),
            small(0),
            8,  // rrcurveto
            14, // endchar
        ];
        let header = [1u8, 0, 4, 1];
        let names = index(&[b"Cubic"]);
        let strings = index(&[b"o"]);
        let globals = index(&[]);
        let charset = [0u8, 0x01, 0x87]; // format 0, glyph 1 is SID 391, the first custom string
        let top_len = 2 * 6; // two operators, each a 5-byte integer and a 1-byte operator
        let top_index_len = 2 + 1 + 2 + top_len;
        let charset_at = header.len() + names.len() + top_index_len + strings.len() + globals.len();
        let charstrings_at = charset_at + charset.len();
        let mut top = int(charset_at as i32);
        top.push(15);
        top.extend(int(charstrings_at as i32));
        top.push(17);
        assert_eq!(top.len(), top_len);

        let mut cff = header.to_vec();
        cff.extend(names);
        cff.extend(index(&[&top]));
        cff.extend(strings);
        cff.extend(globals);
        cff.extend_from_slice(&charset);
        cff.extend(index(&[&notdef, &glyph]));

        let mut head = vec![0u8; 54];
        head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        let mut hhea = vec![0u8; 36];
        hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        hhea[4..6].copy_from_slice(&760i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-240i16).to_be_bytes());
        hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
        let mut hmtx = Vec::new();
        for advance in [500u16, 520] {
            hmtx.extend_from_slice(&advance.to_be_bytes());
            hmtx.extend_from_slice(&0i16.to_be_bytes());
        }
        let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
        maxp.extend_from_slice(&2u16.to_be_bytes());

        let tables: [(&[u8; 4], Vec<u8>); 5] = [
            (b"CFF ", cff),
            (b"head", head),
            (b"hhea", hhea),
            (b"hmtx", hmtx),
            (b"maxp", maxp),
        ];
        let mut out = b"OTTO".to_vec();
        out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0, 64, 0, 2, 0, 16]);
        let mut offset = 12 + 16 * tables.len();
        let mut body = Vec::new();
        for (tag, data) in &tables {
            out.extend_from_slice(*tag);
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut padded = data.clone();
            padded.resize(data.len().div_ceil(4) * 4, 0);
            offset += padded.len();
            body.extend(padded);
        }
        out.extend(body);
        out
    }

    #[test]
    fn an_otf_with_cff_outlines_keeps_its_cubic() {
        let dir = temp_dir();
        let path = dir.join("Cubic.otf");
        fs::write(&path, cff_font()).unwrap();

        let font = Font::load(&path).unwrap();
        assert_eq!(font.name, "Cubic", "no name table, so the file stem");
        assert_eq!(font.metrics.ascender, 760.0);
        assert_eq!(font.glyph_names(), vec![".notdef", "o"]);
        let o = font.glyph("o").unwrap();
        assert_eq!(o.advance, 520.0);
        assert_eq!(o.unicode, None);
        let points: Vec<(f64, f64, PointKind)> = o.contours[0]
            .points
            .iter()
            .map(|p| (p.x, p.y, p.kind))
            .collect();
        assert_eq!(
            points,
            vec![
                (10.0, 0.0, PointKind::On),
                (90.0, 0.0, PointKind::On),
                (90.0, 50.0, PointKind::Off),
                (60.0, 100.0, PointKind::Off),
                (10.0, 100.0, PointKind::On),
            ]
        );
        assert!(o.contours[0].closed);

        let refused = font.save(&dir.join("Copy.otf")).unwrap_err();
        assert!(refused.to_string().contains("not written"), "{refused}");
        assert!(!dir.join("Copy.otf").exists());

        let _ = fs::remove_dir_all(dir);
    }

    fn woff_compressed_a_table(woff: &[u8]) -> bool {
        let num = usize::from(u16::from_be_bytes([woff[12], woff[13]]));
        (0..num).any(|index| {
            let at = 48 + index * 20;
            let comp = u32::from_be_bytes(woff[at + 8..at + 12].try_into().unwrap());
            let orig = u32::from_be_bytes(woff[at + 12..at + 16].try_into().unwrap());
            comp < orig
        })
    }

    #[test]
    fn a_woff_opens_the_same_outlines_as_its_sfnt() {
        let dir = temp_dir();
        let ttf_path = dir.join("Round.ttf");
        face("Round", 0.0, 400.0).save(&ttf_path).unwrap();
        let ttf = fs::read(&ttf_path).unwrap();
        let compressed = wrap_woff(&ttf, true);
        assert!(
            woff_compressed_a_table(&compressed),
            "the fixture should zlib-compress at least one table"
        );
        let woff_path = dir.join("Round.woff");
        fs::write(&woff_path, &compressed).unwrap();
        let from_ttf = Font::load(&ttf_path).unwrap();
        let from_woff = Font::load(&woff_path).unwrap();
        assert_eq!(from_woff.name, from_ttf.name);
        assert_eq!(from_woff.upm, from_ttf.upm);
        assert_eq!(from_woff.metrics, from_ttf.metrics);
        assert_eq!(from_woff.glyphs, from_ttf.glyphs);

        let stored = dir.join("Stored.woff");
        fs::write(&stored, wrap_woff(&ttf, false)).unwrap();
        assert_eq!(Font::load(&stored).unwrap().glyphs, from_ttf.glyphs);

        let otf_path = dir.join("Cubic.otf");
        let otf_woff = dir.join("Cubic.woff");
        fs::write(&otf_path, cff_font()).unwrap();
        fs::write(&otf_woff, wrap_woff(&fs::read(&otf_path).unwrap(), true)).unwrap();
        assert_eq!(
            Font::load(&otf_woff).unwrap().glyphs,
            Font::load(&otf_path).unwrap().glyphs
        );

        let refused = from_woff.save(&dir.join("Copy.woff")).unwrap_err();
        assert!(refused.to_string().contains("not written"), "{refused}");
        assert!(!dir.join("Copy.woff").exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_broken_woff_names_the_problem() {
        let dir = temp_dir();
        let short = dir.join("Short.woff");
        fs::write(&short, b"wOFF").unwrap();
        let err = Font::load(&short).unwrap_err().to_string();
        assert!(err.contains("truncated"), "{err}");

        let ttf_path = dir.join("Round.ttf");
        face("Round", 0.0, 400.0).save(&ttf_path).unwrap();
        let mut woff = wrap_woff(&fs::read(&ttf_path).unwrap(), true);
        let num = usize::from(u16::from_be_bytes([woff[12], woff[13]]));
        let flipped = (0..num).find_map(|index| {
            let at = 48 + index * 20;
            let offset = u32::from_be_bytes(woff[at + 4..at + 8].try_into().unwrap()) as usize;
            let comp = u32::from_be_bytes(woff[at + 8..at + 12].try_into().unwrap()) as usize;
            let orig = u32::from_be_bytes(woff[at + 12..at + 16].try_into().unwrap()) as usize;
            (comp < orig && comp > 0).then_some(offset + comp - 1)
        });
        woff[flipped.expect("a compressed table")] ^= 0xff;
        let bad = dir.join("Bad.woff");
        fs::write(&bad, &woff).unwrap();
        let err = Font::load(&bad).unwrap_err().to_string();
        assert!(
            err.contains("did not inflate") || err.contains("inflated to"),
            "{err}"
        );

        woff.push(0);
        let long = dir.join("Long.woff");
        fs::write(&long, &woff).unwrap();
        let err = Font::load(&long).unwrap_err().to_string();
        assert!(err.contains("does not match"), "{err}");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_woff2_opens_the_same_outlines_as_its_sfnt() {
        let dir = temp_dir();
        let ttf_path = dir.join("Round.ttf");
        face("Round", 0.0, 400.0).save(&ttf_path).unwrap();
        let ttf = fs::read(&ttf_path).unwrap();
        let woff2 = ttf2woff2::encode(&ttf, ttf2woff2::BrotliQuality::default()).unwrap();
        assert_eq!(&woff2[..4], b"wOF2");
        let woff2_path = dir.join("Round.woff2");
        fs::write(&woff2_path, &woff2).unwrap();

        let from_ttf = Font::load(&ttf_path).unwrap();
        let from_woff2 = Font::load(&woff2_path).unwrap();
        assert_eq!(from_woff2.name, from_ttf.name);
        assert_eq!(from_woff2.upm, from_ttf.upm);
        assert_eq!(from_woff2.metrics, from_ttf.metrics);
        assert_eq!(from_woff2.glyphs, from_ttf.glyphs);

        let refused = from_woff2.save(&dir.join("Copy.woff2")).unwrap_err();
        assert!(refused.to_string().contains("not written"), "{refused}");
        assert!(!dir.join("Copy.woff2").exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_broken_woff2_and_garbage_are_named() {
        let dir = temp_dir();
        let woff = dir.join("Web.woff2");
        fs::write(&woff, b"wOF2\0\x01\0\0rest").unwrap();
        let err = Font::load(&woff).unwrap_err().to_string();
        assert!(err.contains("WOFF2 could not be unpacked"), "{err}");
        assert!(err.contains("Web.woff2"), "{err}");

        let junk = dir.join("Junk.ttf");
        fs::write(&junk, b"not a font").unwrap();
        let err = Font::load(&junk).unwrap_err();
        assert!(matches!(err, FoundryError::Import(_)), "{err}");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicate_names_get_the_glyph_index() {
        let mut taken = BTreeMap::new();
        assert_eq!(unique_name("a".into(), 3, &mut taken), "a");
        assert_eq!(unique_name("a".into(), 7, &mut taken), "a.7");
    }

    #[test]
    fn path_kinds() {
        assert!(is_font_binary_path(Path::new("A.TTF")));
        assert!(is_font_binary_path(Path::new("a.otc")));
        assert!(is_font_binary_path(Path::new("a.woff")));
        assert!(is_font_binary_path(Path::new("a.woff2")));
        assert!(!is_font_binary_path(Path::new("a.json")));
        assert!(!is_read_only_path(Path::new("a.ttf")));
        assert!(is_read_only_path(Path::new("a.otf")));
        assert!(is_read_only_path(Path::new("a.woff")));
        assert!(is_read_only_path(Path::new("a.woff2")));
    }
}
