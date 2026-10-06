//! Installable TrueType written by [`Font::save`] when the path ends in `.ttf`.
//!
//! Cubics become quadratics. An open contour gains a closing edge, because a
//! TrueType contour is closed. Glyphs keep their order after a synthetic
//! `.notdef`. Unicode above the Basic Multilingual Plane is refused.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::FoundryError;
use crate::font::{Contour, Font, Glyph, Point, PointKind};

const MAC_EPOCH_OFFSET: u64 = 2_082_844_800;
const QUAD_ERROR: f64 = 1.0;
const MAX_SPLIT: u32 = 8;

pub(crate) fn is_ttf_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf"))
}

pub(crate) fn save_ttf(font: &Font, path: &Path) -> Result<(), FoundryError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|err| FoundryError::Io(err.to_string()))?;
    }
    let bytes = write_ttf(font)?;
    fs::write(path, bytes).map_err(|err| FoundryError::Io(err.to_string()))
}

pub(crate) fn write_ttf(font: &Font) -> Result<Vec<u8>, FoundryError> {
    let glyphs = build_glyphs(font)?;
    let bounds = font_bounds(&glyphs);
    let encoded: Vec<EncodedGlyph> = glyphs.iter().map(encode_glyph).collect();
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for glyph in &encoded {
        push_u32(&mut loca, u32::try_from(glyf.len()).unwrap_or(u32::MAX));
        glyf.extend_from_slice(&glyph.bytes);
        // A glyph starts on a word boundary, so every loca offset stays even.
        if glyf.len() % 2 == 1 {
            glyf.push(0);
        }
    }
    push_u32(&mut loca, u32::try_from(glyf.len()).unwrap_or(u32::MAX));

    let advances: Vec<u16> = glyphs.iter().map(|glyph| glyph.advance).collect();
    let lsbs: Vec<i16> = glyphs.iter().map(|glyph| glyph.lsb).collect();
    let codes = cmap_pairs(&glyphs)?;
    let now = mac_time();

    let mut tables = BTreeMap::new();
    tables.insert(*b"cmap", cmap_table(&codes));
    tables.insert(*b"glyf", glyf);
    tables.insert(*b"head", head_table(font, bounds, now));
    tables.insert(*b"hhea", hhea_table(font, &glyphs, bounds));
    tables.insert(*b"hmtx", hmtx_table(&advances, &lsbs));
    tables.insert(*b"loca", loca);
    tables.insert(*b"maxp", maxp_table(&glyphs));
    tables.insert(*b"name", name_table(font));
    tables.insert(*b"OS/2", os2_table(font, &glyphs, &codes, bounds));
    tables.insert(*b"post", post_table(&glyphs, font.style.italic_angle));
    let pairs = crate::kerning::resolved_pairs(font)?;
    if let Some(kern) = kern_table(&glyphs, &pairs)? {
        tables.insert(*b"kern", kern);
    }
    if let Some(kerning) = &font.kerning
        && !kerning.ligatures.is_empty()
        && let Some(gsub) = gsub_table(&glyphs, &kerning.ligatures)?
    {
        tables.insert(*b"GSUB", gsub);
    }

    assemble(tables)
}

struct BuiltGlyph {
    advance: u16,
    lsb: i16,
    contours: Vec<Vec<DrawPoint>>,
    unicode: Option<u32>,
    name: String,
}

struct DrawPoint {
    x: i16,
    y: i16,
    on: bool,
}

struct EncodedGlyph {
    bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Bounds {
    x_min: i16,
    y_min: i16,
    x_max: i16,
    y_max: i16,
}

fn build_glyphs(font: &Font) -> Result<Vec<BuiltGlyph>, FoundryError> {
    let mut glyphs = Vec::new();
    if let Some(notdef) = font.glyphs.iter().find(|glyph| glyph.name == ".notdef") {
        glyphs.push(build_glyph(notdef)?);
    } else {
        glyphs.push(BuiltGlyph {
            advance: fit_advance(f64::from(font.upm) / 2.0, ".notdef")?,
            lsb: 0,
            contours: Vec::new(),
            unicode: None,
            name: ".notdef".to_string(),
        });
    }
    for glyph in &font.glyphs {
        if glyph.name == ".notdef" {
            continue;
        }
        glyphs.push(build_glyph(glyph)?);
    }
    if glyphs.len() > usize::from(u16::MAX) {
        return Err(FoundryError::Ttf(
            "a TrueType font cannot hold more than 65535 glyphs".to_string(),
        ));
    }
    Ok(glyphs)
}

fn build_glyph(glyph: &Glyph) -> Result<BuiltGlyph, FoundryError> {
    let mut contours = Vec::new();
    for contour in &glyph.contours {
        contours.push(draw_contour(contour, &glyph.name)?);
    }
    let lsb = contours
        .iter()
        .flat_map(|contour| contour.iter())
        .map(|point| point.x)
        .min()
        .unwrap_or(0);
    Ok(BuiltGlyph {
        advance: fit_advance(glyph.advance, &glyph.name)?,
        lsb,
        contours,
        unicode: glyph.unicode,
        name: glyph.name.clone(),
    })
}

fn draw_contour(contour: &Contour, glyph_name: &str) -> Result<Vec<DrawPoint>, FoundryError> {
    let on_indexes = on_indexes(contour);
    if on_indexes.is_empty() {
        return Err(FoundryError::Ttf(format!(
            "glyph {glyph_name} has a contour with no on-curve points"
        )));
    }
    if !contour.closed
        && (on_indexes[0] != 0 || *on_indexes.last().unwrap() != contour.points.len() - 1)
    {
        return Err(FoundryError::Ttf(format!(
            "glyph {glyph_name} has an open contour that starts or ends off-curve"
        )));
    }
    let mut drawn = vec![draw_on(&contour.points[on_indexes[0]], glyph_name)?];
    let segments = if contour.closed {
        on_indexes.len()
    } else {
        on_indexes.len() - 1
    };
    for segment in 0..segments {
        let from = on_indexes[segment % on_indexes.len()];
        let to = on_indexes[(segment + 1) % on_indexes.len()];
        let closing = contour.closed && segment + 1 == segments;
        let offs = between(contour, from, to);
        match offs.len() {
            0 => {}
            1 => drawn.push(draw_off(offs[0], glyph_name)?),
            2 => drawn.extend(approximate_cubic(
                &contour.points[from],
                offs[0],
                offs[1],
                &contour.points[to],
                glyph_name,
            )?),
            count => {
                return Err(FoundryError::Ttf(format!(
                    "glyph {glyph_name} has a segment with {count} off-curve points"
                )));
            }
        }
        if !closing {
            drawn.push(draw_on(&contour.points[to], glyph_name)?);
        }
    }
    if drawn.is_empty() {
        return Err(FoundryError::Ttf(format!(
            "glyph {glyph_name} produced an empty contour"
        )));
    }
    Ok(drawn)
}

fn approximate_cubic(
    start: &Point,
    control_a: &Point,
    control_b: &Point,
    end: &Point,
    glyph_name: &str,
) -> Result<Vec<DrawPoint>, FoundryError> {
    let points = flatten_cubic(
        (start.x, start.y),
        (control_a.x, control_a.y),
        (control_b.x, control_b.y),
        (end.x, end.y),
        0,
    );
    points
        .into_iter()
        .map(|(x, y, on)| {
            Ok(DrawPoint {
                x: fit_i16(x, glyph_name)?,
                y: fit_i16(y, glyph_name)?,
                on,
            })
        })
        .collect()
}

fn flatten_cubic(
    p0: (f64, f64),
    c1: (f64, f64),
    c2: (f64, f64),
    p3: (f64, f64),
    depth: u32,
) -> Vec<(f64, f64, bool)> {
    let mid = cubic_at(p0, c1, c2, p3, 0.5);
    let control = (
        2.0 * mid.0 - 0.5 * (p0.0 + p3.0),
        2.0 * mid.1 - 0.5 * (p0.1 + p3.1),
    );
    let error = dist(
        cubic_at(p0, c1, c2, p3, 0.25),
        quad_at(p0, control, p3, 0.25),
    )
    .max(dist(
        cubic_at(p0, c1, c2, p3, 0.75),
        quad_at(p0, control, p3, 0.75),
    ));
    if error <= QUAD_ERROR || depth >= MAX_SPLIT {
        return vec![(control.0, control.1, false)];
    }
    let (left, right) = split_cubic(p0, c1, c2, p3);
    let mut points = flatten_cubic(left.0, left.1, left.2, left.3, depth + 1);
    points.push((mid.0, mid.1, true));
    points.extend(flatten_cubic(right.0, right.1, right.2, right.3, depth + 1));
    points
}

type Cubic = ((f64, f64), (f64, f64), (f64, f64), (f64, f64));

fn split_cubic(p0: (f64, f64), c1: (f64, f64), c2: (f64, f64), p3: (f64, f64)) -> (Cubic, Cubic) {
    let p01 = mid(p0, c1);
    let p12 = mid(c1, c2);
    let p23 = mid(c2, p3);
    let p012 = mid(p01, p12);
    let p123 = mid(p12, p23);
    let p0123 = mid(p012, p123);
    ((p0, p01, p012, p0123), (p0123, p123, p23, p3))
}

fn cubic_at(p0: (f64, f64), c1: (f64, f64), c2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let uu = u * u;
    let tt = t * t;
    (
        uu * u * p0.0 + 3.0 * uu * t * c1.0 + 3.0 * u * tt * c2.0 + tt * t * p3.0,
        uu * u * p0.1 + 3.0 * uu * t * c1.1 + 3.0 * u * tt * c2.1 + tt * t * p3.1,
    )
}

fn quad_at(p0: (f64, f64), control: (f64, f64), p2: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    (
        u * u * p0.0 + 2.0 * u * t * control.0 + t * t * p2.0,
        u * u * p0.1 + 2.0 * u * t * control.1 + t * t * p2.1,
    )
}

fn mid(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    dx.hypot(dy)
}

fn on_indexes(contour: &Contour) -> Vec<usize> {
    contour
        .points
        .iter()
        .enumerate()
        .filter(|(_, point)| point.kind == PointKind::On)
        .map(|(index, _)| index)
        .collect()
}

fn between(contour: &Contour, from: usize, to: usize) -> Vec<&Point> {
    let mut points = Vec::new();
    let len = contour.points.len();
    if len == 0 {
        return points;
    }
    let mut index = (from + 1) % len;
    while index != to {
        points.push(&contour.points[index]);
        index = (index + 1) % len;
        if points.len() > len {
            break;
        }
    }
    points
}

fn draw_on(point: &Point, glyph_name: &str) -> Result<DrawPoint, FoundryError> {
    Ok(DrawPoint {
        x: fit_i16(point.x, glyph_name)?,
        y: fit_i16(point.y, glyph_name)?,
        on: true,
    })
}

fn draw_off(point: &Point, glyph_name: &str) -> Result<DrawPoint, FoundryError> {
    Ok(DrawPoint {
        x: fit_i16(point.x, glyph_name)?,
        y: fit_i16(point.y, glyph_name)?,
        on: false,
    })
}

fn encode_glyph(glyph: &BuiltGlyph) -> EncodedGlyph {
    if glyph.contours.is_empty() {
        return EncodedGlyph { bytes: Vec::new() };
    }
    let points: Vec<&DrawPoint> = glyph.contours.iter().flatten().collect();
    let x_min = points.iter().map(|point| point.x).min().unwrap_or(0);
    let y_min = points.iter().map(|point| point.y).min().unwrap_or(0);
    let x_max = points.iter().map(|point| point.x).max().unwrap_or(0);
    let y_max = points.iter().map(|point| point.y).max().unwrap_or(0);
    let mut bytes = Vec::new();
    push_i16(
        &mut bytes,
        i16::try_from(glyph.contours.len()).unwrap_or(i16::MAX),
    );
    push_i16(&mut bytes, x_min);
    push_i16(&mut bytes, y_min);
    push_i16(&mut bytes, x_max);
    push_i16(&mut bytes, y_max);
    let mut end = 0u16;
    for contour in &glyph.contours {
        end = end.saturating_add(u16::try_from(contour.len()).unwrap_or(u16::MAX) - 1);
        push_u16(&mut bytes, end);
        end = end.saturating_add(1);
    }
    push_u16(&mut bytes, 0);
    let mut x = 0i16;
    let mut y = 0i16;
    let mut flags = Vec::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for point in points {
        let dx = point.x.wrapping_sub(x);
        let dy = point.y.wrapping_sub(y);
        x = point.x;
        y = point.y;
        let (x_flag, x_bytes) = delta_bytes(dx);
        let (y_flag, y_bytes) = delta_bytes(dy);
        let mut flag = x_flag | (y_flag << 1);
        if point.on {
            flag |= 0x01;
        }
        flags.push(flag);
        xs.extend(x_bytes);
        ys.extend(y_bytes);
    }
    bytes.extend(flags);
    bytes.extend(xs);
    bytes.extend(ys);
    EncodedGlyph { bytes }
}

fn delta_bytes(delta: i16) -> (u8, Vec<u8>) {
    if delta == 0 {
        return (0x10, Vec::new());
    }
    let abs = delta.unsigned_abs();
    if abs <= 255 {
        let sign = if delta > 0 { 0x10 } else { 0 };
        return (0x02 | sign, vec![abs as u8]);
    }
    (0, delta.to_be_bytes().to_vec())
}

fn font_bounds(glyphs: &[BuiltGlyph]) -> Bounds {
    let points: Vec<&DrawPoint> = glyphs
        .iter()
        .flat_map(|glyph| glyph.contours.iter().flatten())
        .collect();
    if points.is_empty() {
        return Bounds {
            x_min: 0,
            y_min: 0,
            x_max: 0,
            y_max: 0,
        };
    }
    Bounds {
        x_min: points.iter().map(|point| point.x).min().unwrap_or(0),
        y_min: points.iter().map(|point| point.y).min().unwrap_or(0),
        x_max: points.iter().map(|point| point.x).max().unwrap_or(0),
        y_max: points.iter().map(|point| point.y).max().unwrap_or(0),
    }
}

fn cmap_pairs(glyphs: &[BuiltGlyph]) -> Result<Vec<(u32, u16)>, FoundryError> {
    let mut pairs = Vec::new();
    for (index, glyph) in glyphs.iter().enumerate() {
        let Some(code) = glyph.unicode else {
            continue;
        };
        if code > 0xFFFE {
            return Err(FoundryError::Ttf(format!(
                "glyph {} has unicode U+{code:04X}, which is outside the Basic Multilingual Plane",
                glyph.name
            )));
        }
        if let Some(other) = glyphs
            .iter()
            .find(|other| other.unicode == Some(code) && other.name != glyph.name)
        {
            return Err(FoundryError::DuplicateUnicode {
                code,
                glyph: glyph.name.clone(),
                other: other.name.clone(),
            });
        }
        pairs.push((code, u16::try_from(index).unwrap_or(0)));
    }
    pairs.sort_by_key(|(code, _)| *code);
    Ok(pairs)
}

fn assemble(tables: BTreeMap<[u8; 4], Vec<u8>>) -> Result<Vec<u8>, FoundryError> {
    let count = u16::try_from(tables.len()).unwrap_or(0);
    let (pow, log, _) = power_of_two(count);
    let mut directory = Vec::new();
    push_u32(&mut directory, 0x0001_0000);
    push_u16(&mut directory, count);
    push_u16(&mut directory, pow.saturating_mul(16));
    push_u16(&mut directory, log);
    push_u16(
        &mut directory,
        count.saturating_mul(16) - pow.saturating_mul(16),
    );

    let mut offset = 12 + tables.len() * 16;
    let mut records = Vec::new();
    let mut blobs = Vec::new();
    let mut head_at = None;
    for (tag, data) in &tables {
        let padded = pad4(data);
        let checksum = checksum(&padded);
        let mut record = Vec::new();
        record.extend(tag);
        push_u32(&mut record, checksum);
        push_u32(&mut record, u32::try_from(offset).unwrap_or(0));
        push_u32(&mut record, u32::try_from(data.len()).unwrap_or(0));
        if tag == b"head" {
            head_at = Some(offset + 8);
        }
        records.extend(record);
        blobs.extend(padded);
        offset += pad4(data).len();
    }
    directory.extend(records);
    directory.extend(blobs);
    let head_at = head_at.ok_or_else(|| FoundryError::Ttf("missing head table".to_string()))?;
    let sum = checksum(&directory);
    let adjustment = 0xB1B0_AFBA_u32.wrapping_sub(sum);
    directory[head_at..head_at + 4].copy_from_slice(&adjustment.to_be_bytes());
    Ok(directory)
}

fn head_table(font: &Font, bounds: Bounds, now: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    push_u16(&mut bytes, 1);
    push_u16(&mut bytes, 0);
    push_u32(&mut bytes, font.info.font_revision());
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0x5F0F_3CF5);
    push_u16(&mut bytes, 0x000B);
    push_u16(&mut bytes, font.upm);
    push_u64(&mut bytes, now);
    push_u64(&mut bytes, now);
    push_i16(&mut bytes, bounds.x_min);
    push_i16(&mut bytes, bounds.y_min);
    push_i16(&mut bytes, bounds.x_max);
    push_i16(&mut bytes, bounds.y_max);
    let (bold, italic) = style_bits(font);
    push_u16(&mut bytes, u16::from(bold) | (u16::from(italic) << 1));
    push_u16(&mut bytes, 8);
    push_i16(&mut bytes, 2);
    push_i16(&mut bytes, 1);
    push_i16(&mut bytes, 0);
    bytes
}

fn hhea_table(font: &Font, glyphs: &[BuiltGlyph], bounds: Bounds) -> Vec<u8> {
    let ascender = fit_metric(font.metrics.ascender);
    let descender = fit_metric(font.metrics.descender);
    let advance_max = glyphs.iter().map(|glyph| glyph.advance).max().unwrap_or(0);
    let min_lsb = glyphs.iter().map(|glyph| glyph.lsb).min().unwrap_or(0);
    let min_rsb = glyphs
        .iter()
        .map(|glyph| {
            let x_max = glyph
                .contours
                .iter()
                .flatten()
                .map(|point| point.x)
                .max()
                .unwrap_or(0);
            i16::try_from(i32::from(glyph.advance) - i32::from(x_max)).unwrap_or(i16::MIN)
        })
        .min()
        .unwrap_or(0);
    let mut bytes = Vec::new();
    push_u16(&mut bytes, 1);
    push_u16(&mut bytes, 0);
    push_i16(&mut bytes, ascender);
    push_i16(&mut bytes, descender);
    push_i16(&mut bytes, 0);
    push_u16(&mut bytes, advance_max);
    push_i16(&mut bytes, min_lsb);
    push_i16(&mut bytes, min_rsb);
    push_i16(&mut bytes, bounds.x_max);
    // Caret slope: upright is rise 1, run 0. An italic leans with its angle.
    let (rise, run) = if font.style.italic_angle == 0.0 {
        (1, 0)
    } else {
        let run = (-font.style.italic_angle).to_radians().tan() * 1000.0;
        (1000, run.round().clamp(-32768.0, 32767.0) as i16)
    };
    push_i16(&mut bytes, rise);
    push_i16(&mut bytes, run);
    push_i16(&mut bytes, 0);
    push_i16(&mut bytes, 0);
    push_i16(&mut bytes, 0);
    push_i16(&mut bytes, 0);
    push_i16(&mut bytes, 0);
    push_i16(&mut bytes, 0);
    push_u16(&mut bytes, u16::try_from(glyphs.len()).unwrap_or(0));
    bytes
}

fn hmtx_table(advances: &[u16], lsbs: &[i16]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (advance, lsb) in advances.iter().zip(lsbs) {
        push_u16(&mut bytes, *advance);
        push_i16(&mut bytes, *lsb);
    }
    bytes
}

fn maxp_table(glyphs: &[BuiltGlyph]) -> Vec<u8> {
    let max_points = glyphs
        .iter()
        .map(|glyph| glyph.contours.iter().map(Vec::len).sum::<usize>())
        .max()
        .unwrap_or(0);
    let max_contours = glyphs
        .iter()
        .map(|glyph| glyph.contours.len())
        .max()
        .unwrap_or(0);
    let mut bytes = Vec::new();
    push_u32(&mut bytes, 0x0001_0000);
    push_u16(&mut bytes, u16::try_from(glyphs.len()).unwrap_or(0));
    push_u16(&mut bytes, u16::try_from(max_points).unwrap_or(u16::MAX));
    push_u16(&mut bytes, u16::try_from(max_contours).unwrap_or(u16::MAX));
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 2);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    bytes
}

/// Legacy family and style (IDs 1 and 2) group up to four styles in older apps. The typographic
/// pair (IDs 16 and 17) carries the real family and style, so every style lands in one family.
///
/// ID 3 is the unique font identifier. Windows Font Viewer and GDI reject a TrueType file that
/// does not have one, even when every outline is valid. ID 5 is the version string.
fn name_table(font: &Font) -> Vec<u8> {
    let (legacy_family, legacy_style) = font.legacy_names();
    let postscript = postscript_name(&font.file_stem());
    let mut records = vec![
        (1u16, legacy_family),
        (2, legacy_style),
        (3, font.info.export_unique_id(&postscript)),
        (4, font.full_name()),
        (5, font.info.version_label()),
        (6, postscript),
        (16, font.style.family.clone()),
        (17, font.style.name.clone()),
    ];
    let optional = [
        (0u16, &font.info.copyright),
        (9, &font.info.designer),
        (13, &font.info.license),
        (14, &font.info.license_url),
    ];
    for (id, text) in optional {
        if !text.trim().is_empty() {
            records.push((id, text.trim().to_string()));
        }
    }
    records.sort_by_key(|(id, _)| *id);
    let encoded: Vec<(u16, Vec<u8>)> = records
        .iter()
        .map(|(id, text)| (*id, utf16_be(text)))
        .collect();
    let mut bytes = Vec::new();
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, u16::try_from(encoded.len()).unwrap_or(0));
    push_u16(
        &mut bytes,
        u16::try_from(6 + encoded.len() * 12).unwrap_or(0),
    );
    let mut storage = Vec::new();
    for (id, string) in &encoded {
        push_u16(&mut bytes, 3);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 0x0409);
        push_u16(&mut bytes, *id);
        push_u16(&mut bytes, u16::try_from(string.len()).unwrap_or(0));
        push_u16(&mut bytes, u16::try_from(storage.len()).unwrap_or(0));
        storage.extend_from_slice(string);
    }
    bytes.extend(storage);
    bytes
}

fn os2_table(font: &Font, glyphs: &[BuiltGlyph], codes: &[(u32, u16)], bounds: Bounds) -> Vec<u8> {
    let average = if glyphs.is_empty() {
        0
    } else {
        let sum: u32 = glyphs.iter().map(|glyph| u32::from(glyph.advance)).sum();
        u16::try_from(sum / u32::try_from(glyphs.len()).unwrap_or(1)).unwrap_or(0)
    };
    let (first, last) = codes
        .iter()
        .fold((u16::MAX, 0u16), |(first, last), (code, _)| {
            let code = *code as u16;
            (first.min(code), last.max(code))
        });
    let (first, last) = if codes.is_empty() {
        (0xFFFF, 0xFFFF)
    } else {
        (first, last)
    };
    let ascender = fit_metric(font.metrics.ascender);
    let descender = fit_metric(font.metrics.descender);
    let win_ascent =
        u16::try_from(i32::from(ascender).max(i32::from(bounds.y_max)).max(0)).unwrap_or(u16::MAX);
    let descent_floor = i32::from(descender).min(i32::from(bounds.y_min)).min(0);
    let win_descent = u16::try_from(descent_floor.unsigned_abs()).unwrap_or(u16::MAX);
    let scale = f64::from(font.upm) / 1000.0;
    let mut bytes = vec![0; 96];
    write_u16(&mut bytes, 0, 4);
    write_i16(&mut bytes, 2, i16::try_from(average).unwrap_or(0));
    write_u16(&mut bytes, 4, font.style.weight);
    write_u16(&mut bytes, 6, font.style.width);
    write_i16(&mut bytes, 10, fit_metric(650.0 * scale));
    write_i16(&mut bytes, 12, fit_metric(700.0 * scale));
    write_i16(&mut bytes, 16, fit_metric(140.0 * scale));
    write_i16(&mut bytes, 18, fit_metric(650.0 * scale));
    write_i16(&mut bytes, 20, fit_metric(700.0 * scale));
    write_i16(&mut bytes, 24, fit_metric(480.0 * scale));
    write_i16(&mut bytes, 26, fit_metric(50.0 * scale));
    write_i16(&mut bytes, 28, fit_metric(300.0 * scale));
    bytes[58..62].copy_from_slice(&font.info.vendor_tag());
    // fsSelection: italic is bit 0, bold bit 5, regular bit 6. Bit 7 says to use typo metrics.
    let (bold, italic) = style_bits(font);
    let regular = !bold && !italic;
    let selection = 0x0080 | u16::from(italic) | (u16::from(bold) << 5) | (u16::from(regular) << 6);
    write_u16(&mut bytes, 62, selection);
    write_u16(&mut bytes, 64, first);
    write_u16(&mut bytes, 66, last);
    write_i16(&mut bytes, 68, ascender);
    write_i16(&mut bytes, 70, descender);
    write_u16(&mut bytes, 74, win_ascent);
    write_u16(&mut bytes, 76, win_descent);
    write_i16(&mut bytes, 86, fit_metric(font.metrics.x_height));
    write_i16(&mut bytes, 88, fit_metric(font.metrics.cap_height));
    write_u16(&mut bytes, 92, 32);
    bytes
}

/// Format 2 carries glyph names, so a saved `.ttf` opens with the same names. When a name
/// cannot be stored (non-ASCII or longer than 63 bytes), format 3 is written with no names.
fn post_table(glyphs: &[BuiltGlyph], italic_angle: f64) -> Vec<u8> {
    let storable = glyphs.iter().all(|glyph| {
        !glyph.name.is_empty()
            && glyph.name.len() <= 63
            && glyph.name.bytes().all(|byte| byte.is_ascii_graphic())
    });
    let mut bytes = Vec::new();
    push_u32(&mut bytes, if storable { 0x0002_0000 } else { 0x0003_0000 });
    // italicAngle is a 16.16 fixed-point number.
    let fixed = (italic_angle * 65536.0)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    push_u32(&mut bytes, (fixed as i32) as u32);
    push_i16(&mut bytes, -100);
    push_i16(&mut bytes, 50);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    if storable {
        // Index 0 is the standard `.notdef`. Every other name is a custom string, which starts
        // at index 258, after the standard Macintosh names.
        push_u16(&mut bytes, u16::try_from(glyphs.len()).unwrap_or(u16::MAX));
        let mut strings = Vec::new();
        let mut next = 258u16;
        for glyph in glyphs {
            if glyph.name == ".notdef" {
                push_u16(&mut bytes, 0);
                continue;
            }
            push_u16(&mut bytes, next);
            next = next.saturating_add(1);
            strings.push(u8::try_from(glyph.name.len()).unwrap_or(63));
            strings.extend_from_slice(glyph.name.as_bytes());
        }
        bytes.extend_from_slice(&strings);
    }
    bytes
}

fn cmap_table(codes: &[(u32, u16)]) -> Vec<u8> {
    let format4 = cmap_format4(codes);
    let mut bytes = Vec::new();
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 2);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 3);
    push_u32(&mut bytes, 20);
    push_u16(&mut bytes, 3);
    push_u16(&mut bytes, 1);
    push_u32(&mut bytes, 20);
    bytes.extend(format4);
    bytes
}

fn cmap_format4(codes: &[(u32, u16)]) -> Vec<u8> {
    let mut ends = Vec::new();
    let mut starts = Vec::new();
    let mut deltas = Vec::new();
    for (code, glyph) in codes {
        let code = *code as u16;
        ends.push(code);
        starts.push(code);
        deltas.push(glyph.wrapping_sub(code) as i16);
    }
    ends.push(0xFFFF);
    starts.push(0xFFFF);
    deltas.push(1);
    let seg_count = u16::try_from(ends.len()).unwrap_or(0);
    let (pow, log, _) = power_of_two(seg_count);
    let mut body = Vec::new();
    push_u16(&mut body, seg_count.saturating_mul(2));
    push_u16(&mut body, pow.saturating_mul(2));
    push_u16(&mut body, log);
    push_u16(
        &mut body,
        seg_count.saturating_mul(2) - pow.saturating_mul(2),
    );
    for end in &ends {
        push_u16(&mut body, *end);
    }
    push_u16(&mut body, 0);
    for start in &starts {
        push_u16(&mut body, *start);
    }
    for delta in &deltas {
        push_i16(&mut body, *delta);
    }
    for _ in &ends {
        push_u16(&mut body, 0);
    }
    let mut bytes = Vec::new();
    push_u16(&mut bytes, 4);
    push_u16(&mut bytes, u16::try_from(body.len() + 6).unwrap_or(0));
    push_u16(&mut bytes, 0);
    bytes.extend(body);
    bytes
}

fn fit_i16(value: f64, glyph_name: &str) -> Result<i16, FoundryError> {
    if !value.is_finite() {
        return Err(FoundryError::NonFinite);
    }
    let rounded = value.round();
    if !(f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&rounded) {
        return Err(FoundryError::Ttf(format!(
            "glyph {glyph_name} has a coordinate {value} that does not fit in TrueType"
        )));
    }
    Ok(rounded as i16)
}

fn fit_advance(value: f64, glyph_name: &str) -> Result<u16, FoundryError> {
    if !value.is_finite() {
        return Err(FoundryError::NonFinite);
    }
    let rounded = value.round();
    if !(0.0..=f64::from(u16::MAX)).contains(&rounded) {
        return Err(FoundryError::Ttf(format!(
            "glyph {glyph_name} has an advance {value} that does not fit in TrueType"
        )));
    }
    Ok(rounded as u16)
}

fn fit_metric(value: f64) -> i16 {
    if !value.is_finite() {
        return 0;
    }
    let rounded = value.round();
    if rounded < f64::from(i16::MIN) {
        i16::MIN
    } else if rounded > f64::from(i16::MAX) {
        i16::MAX
    } else {
        rounded as i16
    }
}

/// Bold and italic as the legacy style links them: bold only at weight 700.
fn style_bits(font: &Font) -> (bool, bool) {
    (font.style.weight == 700, font.style.italic)
}

fn postscript_name(name: &str) -> String {
    let mut cleaned: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .take(63)
        .collect();
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", ".");
    }
    let trimmed = cleaned.trim_matches(|ch| ch == '-' || ch == '.');
    if trimmed.is_empty() {
        "Font".to_string()
    } else {
        trimmed.to_string()
    }
}

fn glyph_index(glyphs: &[BuiltGlyph], name: &str) -> Result<u16, FoundryError> {
    glyphs
        .iter()
        .position(|glyph| glyph.name == name)
        .and_then(|index| u16::try_from(index).ok())
        .ok_or_else(|| {
            FoundryError::Ttf(format!(
                "{name} is not in the font, so it cannot be kerned or ligated"
            ))
        })
}

fn fit_kern(value: f64) -> Result<i16, FoundryError> {
    if !value.is_finite() {
        return Err(FoundryError::NonFinite);
    }
    let rounded = value.round();
    if !(f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&rounded) {
        return Err(FoundryError::Ttf(format!(
            "kerning value {value} does not fit in a TrueType kern table"
        )));
    }
    Ok(rounded as i16)
}

/// OpenType `kern` format 0. `None` when there is nothing to write.
fn kern_table(
    glyphs: &[BuiltGlyph],
    pairs: &[(String, String, f64)],
) -> Result<Option<Vec<u8>>, FoundryError> {
    let mut encoded = Vec::new();
    for (left, right, value) in pairs {
        let left_id = glyph_index(glyphs, left)?;
        let right_id = glyph_index(glyphs, right)?;
        let packed = (u32::from(left_id) << 16) | u32::from(right_id);
        encoded.push((packed, fit_kern(*value)?));
    }
    encoded.sort_by_key(|(packed, _)| *packed);
    encoded.dedup_by_key(|(packed, _)| *packed);
    if encoded.is_empty() {
        return Ok(None);
    }
    let count = u16::try_from(encoded.len()).map_err(|_| {
        FoundryError::Ttf("too many kerning pairs for a TrueType kern table".into())
    })?;
    let (pow, log, _) = power_of_two(count);
    let search_range = pow.saturating_mul(6);
    let range_shift = count.saturating_mul(6).saturating_sub(search_range);
    let length = 14u16.saturating_add(count.saturating_mul(6));
    let mut bytes = Vec::new();
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 1);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, length);
    bytes.push(0);
    bytes.push(1);
    push_u16(&mut bytes, count);
    push_u16(&mut bytes, search_range);
    push_u16(&mut bytes, log);
    push_u16(&mut bytes, range_shift);
    for (packed, value) in encoded {
        push_u16(&mut bytes, (packed >> 16) as u16);
        push_u16(&mut bytes, packed as u16);
        push_i16(&mut bytes, value);
    }
    Ok(Some(bytes))
}

/// A minimal GSUB with one `liga` lookup. `None` when no ligature can be written.
fn gsub_table(
    glyphs: &[BuiltGlyph],
    ligatures: &[crate::font::Ligature],
) -> Result<Option<Vec<u8>>, FoundryError> {
    use std::collections::BTreeMap;
    /// Components after the first glyph, then the ligature glyph id.
    type LigaSet = Vec<(Vec<u16>, u16)>;
    let mut grouped: BTreeMap<u16, LigaSet> = BTreeMap::new();
    for liga in ligatures {
        if liga.glyphs.len() < 2 {
            continue;
        }
        let first = glyph_index(glyphs, &liga.glyphs[0])?;
        let mut rest = Vec::new();
        for name in liga.glyphs.iter().skip(1) {
            rest.push(glyph_index(glyphs, name)?);
        }
        let target = glyph_index(glyphs, &liga.name)?;
        grouped.entry(first).or_default().push((rest, target));
    }
    if grouped.is_empty() {
        return Ok(None);
    }
    let entries: Vec<(u16, LigaSet)> = grouped.into_iter().collect();
    let coverage_count = u16::try_from(entries.len()).unwrap_or(u16::MAX);

    let mut coverage = Vec::new();
    push_u16(&mut coverage, 1);
    push_u16(&mut coverage, coverage_count);
    for (gid, _) in &entries {
        push_u16(&mut coverage, *gid);
    }

    let mut sets = Vec::new();
    for (_, ligs) in &entries {
        let mut blobs = Vec::new();
        for (comps, lig_id) in ligs {
            let mut lig = Vec::new();
            push_u16(&mut lig, *lig_id);
            let count = u16::try_from(comps.len() + 1).unwrap_or(u16::MAX);
            push_u16(&mut lig, count);
            for component in comps {
                push_u16(&mut lig, *component);
            }
            blobs.push(lig);
        }
        let mut body = Vec::new();
        let header = 2 + 2 * blobs.len();
        push_u16(&mut body, u16::try_from(blobs.len()).unwrap_or(0));
        let mut cursor = header;
        for blob in &blobs {
            push_u16(&mut body, u16::try_from(cursor).unwrap_or(0));
            cursor += blob.len();
        }
        for blob in blobs {
            body.extend(blob);
        }
        sets.push(body);
    }

    let header_len = 6 + 2 * sets.len();
    let mut sub = Vec::new();
    push_u16(&mut sub, 1);
    push_u16(&mut sub, u16::try_from(header_len).unwrap_or(0));
    push_u16(&mut sub, coverage_count);
    let mut cursor = header_len + coverage.len();
    let mut offsets = Vec::new();
    for set in &sets {
        offsets.push(cursor);
        cursor += set.len();
    }
    for offset in offsets {
        push_u16(&mut sub, u16::try_from(offset).unwrap_or(0));
    }
    sub.extend(coverage);
    for set in sets {
        sub.extend(set);
    }

    let script_list_len = 20u16;
    let feature_list_len = 14u16;
    let gsub_header = 10u16;
    let script_off = gsub_header;
    let feature_off = script_off + script_list_len;
    let lookup_off = feature_off + feature_list_len;

    let mut out = Vec::new();
    push_u16(&mut out, 1);
    push_u16(&mut out, 0);
    push_u16(&mut out, script_off);
    push_u16(&mut out, feature_off);
    push_u16(&mut out, lookup_off);
    push_u16(&mut out, 1);
    out.extend(b"DFLT");
    push_u16(&mut out, 8);
    push_u16(&mut out, 4);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0xFFFF);
    push_u16(&mut out, 1);
    push_u16(&mut out, 0);
    push_u16(&mut out, 1);
    out.extend(b"liga");
    push_u16(&mut out, 8);
    push_u16(&mut out, 0);
    push_u16(&mut out, 1);
    push_u16(&mut out, 0);
    push_u16(&mut out, 1);
    push_u16(&mut out, 4);
    push_u16(&mut out, 4);
    push_u16(&mut out, 0);
    push_u16(&mut out, 1);
    push_u16(&mut out, 8);
    out.extend(sub);
    Ok(Some(out))
}

fn utf16_be(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .flat_map(|unit| unit.to_be_bytes())
        .collect()
}

fn mac_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().saturating_add(MAC_EPOCH_OFFSET))
        .unwrap_or(MAC_EPOCH_OFFSET)
}

pub(crate) fn power_of_two(count: u16) -> (u16, u16, u16) {
    let mut pow = 1u16;
    let mut log = 0u16;
    while pow.saturating_mul(2) <= count {
        pow = pow.saturating_mul(2);
        log += 1;
    }
    (pow, log, count.saturating_sub(pow))
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn pad4(data: &[u8]) -> Vec<u8> {
    let mut padded = data.to_vec();
    while !padded.len().is_multiple_of(4) {
        padded.push(0);
    }
    padded
}

#[cfg(test)]
fn loca_offsets(bytes: &[u8]) -> Vec<u32> {
    let count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let mut loca = None;
    let mut head = None;
    let mut maxp = None;
    for index in 0..count {
        let rec = 12 + index * 16;
        let tag = &bytes[rec..rec + 4];
        let offset = u32::from_be_bytes(bytes[rec + 8..rec + 12].try_into().unwrap()) as usize;
        let length = u32::from_be_bytes(bytes[rec + 12..rec + 16].try_into().unwrap()) as usize;
        let table = &bytes[offset..offset + length];
        match tag {
            b"loca" => loca = Some(table),
            b"head" => head = Some(table),
            b"maxp" => maxp = Some(table),
            _ => {}
        }
    }
    let head = head.unwrap();
    let maxp = maxp.unwrap();
    let loca = loca.unwrap();
    let long = i16::from_be_bytes([head[50], head[51]]) == 1;
    let glyphs = u16::from_be_bytes([maxp[4], maxp[5]]) as usize;
    (0..=glyphs)
        .map(|index| {
            if long {
                let at = index * 4;
                u32::from_be_bytes(loca[at..at + 4].try_into().unwrap())
            } else {
                let at = index * 2;
                u32::from(u16::from_be_bytes(loca[at..at + 2].try_into().unwrap())) * 2
            }
        })
        .collect()
}

#[cfg(all(test, windows))]
fn windows_accepts(path: &std::path::Path) -> bool {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn AddFontResourceExW(
            name: *const u16,
            flags: u32,
            reserved: *mut core::ffi::c_void,
        ) -> i32;
        fn RemoveFontResourceExW(
            name: *const u16,
            flags: u32,
            reserved: *mut core::ffi::c_void,
        ) -> i32;
    }

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    const PRIVATE: u32 = 0x10;
    unsafe {
        let added = AddFontResourceExW(wide.as_ptr(), PRIVATE, core::ptr::null_mut());
        if added > 0 {
            RemoveFontResourceExW(wide.as_ptr(), PRIVATE, core::ptr::null_mut());
        }
        added > 0
    }
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend(value.to_be_bytes());
}

fn push_i16(bytes: &mut Vec<u8>, value: i16) {
    bytes.extend(value.to_be_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_be_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend(value.to_be_bytes());
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_i16(bytes: &mut [u8], offset: usize, value: i16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{Glyph, Point, PointKind};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use ttf_parser::{Face, OutlineBuilder};

    static TEMP_IDS: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = TEMP_IDS.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("typefoundry-ttf-{tick}-{id}"));
        fs::create_dir_all(&path).unwrap();
        path
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

    struct Ink {
        points: Vec<(f32, f32)>,
        curves: usize,
    }

    impl OutlineBuilder for Ink {
        fn move_to(&mut self, x: f32, y: f32) {
            self.points.push((x, y));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.points.push((x, y));
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            self.points.push((x1, y1));
            self.points.push((x, y));
        }
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
            self.curves += 1;
        }
        fn close(&mut self) {}
    }

    #[test]
    fn saves_a_square_ttf_that_parses() {
        let dir = temp_dir();
        let path = dir.join("square.ttf");
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
        font.save(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let face = Face::parse(&bytes, 0).unwrap();
        assert_eq!(checksum(&bytes), 0xB1B0_AFBA);
        assert!(loca_offsets(&bytes).iter().all(|offset| offset % 2 == 0));
        let mut saw_unique = false;
        let mut saw_version = false;
        for name in face.names() {
            if name.name_id == ttf_parser::name_id::UNIQUE_ID {
                let text = name.to_string().unwrap();
                assert!(!text.contains("Havelin"), "{text}");
                saw_unique = true;
            }
            if name.name_id == ttf_parser::name_id::VERSION {
                let text = name.to_string().unwrap();
                assert!(text.starts_with("Version "), "{text}");
                saw_version = true;
            }
        }
        assert!(saw_unique, "Windows rejects a font with no unique name");
        assert!(saw_version);
        #[cfg(windows)]
        assert!(
            windows_accepts(&path),
            "Windows Font Viewer would reject this file"
        );
        assert_eq!(face.number_of_glyphs(), 2);
        assert_eq!(face.units_per_em(), 1000);
        let id = face.glyph_index('H').unwrap();
        assert_eq!(face.glyph_hor_advance(id), Some(400));
        let mut ink = Ink {
            points: Vec::new(),
            curves: 0,
        };
        face.outline_glyph(id, &mut ink).unwrap();
        assert_eq!(ink.curves, 0);
        for expected in [(40.0, 0.0), (120.0, 0.0), (120.0, 120.0), (40.0, 120.0)] {
            assert!(
                ink.points
                    .iter()
                    .any(|point| (point.0 - expected.0).abs() < 0.1
                        && (point.1 - expected.1).abs() < 0.1),
                "{expected:?} missing from {:?}",
                ink.points
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn width_vendor_and_version_dots_are_written() {
        let dir = temp_dir();
        let path = dir.join("vostok.ttf");
        let mut font = Font::new("Vostok", 1000).unwrap();
        font.style.family = "VostokSerifv4.5".into();
        font.style.name = "Compressed".into();
        font.style.width = 2;
        font.info.vendor = "VSTK".into();
        font.info.unique_id = "vostok-4.5".into();
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
        font.save(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let face = Face::parse(&bytes, 0).unwrap();
        assert_eq!(face.width().to_number(), 2);
        let os2 = face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"OS/2"))
            .unwrap();
        assert_eq!(&os2[58..62], b"VSTK");
        let mut saw_postscript = false;
        let mut saw_unique = false;
        for name in face.names() {
            let Some(text) = name.to_string() else {
                continue;
            };
            if name.name_id == ttf_parser::name_id::POST_SCRIPT_NAME {
                assert!(text.contains("v4.5"), "{text}");
                saw_postscript = true;
            }
            if name.name_id == ttf_parser::name_id::UNIQUE_ID {
                assert_eq!(text, "vostok-4.5");
                saw_unique = true;
            }
        }
        assert!(saw_postscript);
        assert!(saw_unique);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_odd_glyph_still_lands_on_an_even_offset() {
        let mut font = Font::new("Odd", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "l".into(),
            unicode: Some(u32::from('l')),
            advance: 200.0,
            contours: vec![Contour {
                closed: true,
                points: vec![on(40.0, 0.0), on(80.0, 30.0)],
            }],
        })
        .unwrap();
        let bytes = write_ttf(&font).unwrap();
        let offsets = loca_offsets(&bytes);
        assert!(offsets.iter().all(|offset| offset % 2 == 0), "{offsets:?}");
        let end = offsets.last().copied().unwrap();
        assert_eq!(end % 2, 0);
        assert!(end > 0, "{offsets:?}");
    }

    #[test]
    fn converts_a_cubic_into_quadratics() {
        let mut font = Font::new("Curve", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "C".into(),
            unicode: Some(u32::from('C')),
            advance: 500.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    on(0.0, 0.0),
                    off(0.0, 200.0),
                    off(200.0, 200.0),
                    on(200.0, 0.0),
                ],
            }],
        })
        .unwrap();
        let bytes = write_ttf(&font).unwrap();
        let face = Face::parse(&bytes, 0).unwrap();
        let id = face.glyph_index('C').unwrap();
        let mut ink = Ink {
            points: Vec::new(),
            curves: 0,
        };
        face.outline_glyph(id, &mut ink).unwrap();
        assert_eq!(ink.curves, 0);
        assert!(ink.points.len() > 2, "{:?}", ink.points);
        let top = ink
            .points
            .iter()
            .map(|point| point.1)
            .fold(0.0_f32, f32::max);
        assert!(top > 140.0, "curve peak {top} is too flat");
    }

    #[test]
    fn refuses_a_character_outside_the_bmp() {
        let mut font = Font::new("Wide", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "uni1F600".into(),
            unicode: Some(0x1F600),
            advance: 600.0,
            contours: vec![Contour {
                closed: true,
                points: vec![on(0.0, 0.0), on(100.0, 0.0), on(100.0, 100.0)],
            }],
        })
        .unwrap();
        let error = write_ttf(&font).unwrap_err();
        assert!(
            error.to_string().contains("Basic Multilingual Plane"),
            "{error}"
        );
    }
}
