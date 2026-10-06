//! Draw a proof to a PNG or a PDF: the words, the vertical metrics, and each glyph's advance box.
//!
//! A `.pdf` path writes vector outlines with real curves, so the proof can be scaled. Any other
//! path writes a PNG. The PNG encoder is the file format itself. `flate2` is already used for WOFF
//! and compresses the PDF content stream, so this adds no crate.

use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::error::FoundryError;
use crate::font::{Contour, Font, Glyph, Point, PointKind};
use crate::kerning::resolved_pairs;

const MARGIN: f64 = 24.0;

#[derive(Debug, Clone)]
pub struct ProofOptions {
    pub text: String,
    pub pixel_size: f64,
    pub guides: bool,
    pub boxes: bool,
    /// When set, the second font is drawn under the first, in the same positions.
    pub compare: Option<Font>,
    /// Replace an existing proof and keep the previous file as `name.bak`.
    pub force: bool,
}

impl Default for ProofOptions {
    fn default() -> Self {
        Self {
            text: String::new(),
            pixel_size: 72.0,
            guides: true,
            boxes: true,
            compare: None,
            force: false,
        }
    }
}

/// Write `path` as an RGBA PNG, or as a PDF when the path ends in `.pdf`. Returns the width and
/// height in pixels, or in points for a PDF.
pub fn write_proof(
    font: &Font,
    path: &Path,
    options: &ProofOptions,
) -> Result<(u32, u32), FoundryError> {
    crate::save::prepare_write(path, options.force)?;
    if !options.pixel_size.is_finite() || options.pixel_size < 8.0 || options.pixel_size > 400.0 {
        return Err(FoundryError::Edit(
            "proof pixel size must be from 8 to 400".into(),
        ));
    }
    let text = proof_text(font, options);
    if is_pdf(path) {
        return write_pdf(font, path, &text, options);
    }
    let scale = options.pixel_size / f64::from(font.upm);
    let line = layout(font, &text)?;
    let width_units = line
        .last()
        .map(|placed| placed.x + placed.advance)
        .unwrap_or(1.0)
        .max(1.0);
    let width = (MARGIN * 2.0 + width_units * scale).ceil() as u32;
    let height = (MARGIN * 2.0 + options.pixel_size * 1.4).ceil() as u32;
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    fill(&mut pixels, width, [0xF3, 0xEE, 0xE4, 0xFF]);
    let baseline = MARGIN + font.metrics.ascender * scale;
    if options.guides {
        for metric in [
            font.metrics.ascender,
            font.metrics.cap_height,
            font.metrics.x_height,
            0.0,
            font.metrics.descender,
        ] {
            let y = baseline - metric * scale;
            hline(&mut pixels, width, height, y, [0x3D, 0x8B, 0xFF, 0xFF]);
        }
    }
    if let Some(compare) = &options.compare {
        let other = layout(compare, &text).unwrap_or_default();
        draw_line(
            &mut pixels,
            width,
            height,
            &other,
            Draw {
                scale,
                baseline,
                color: [0xC4, 0x5C, 0x26, 0x88],
                boxes: false,
            },
        );
    }
    draw_line(
        &mut pixels,
        width,
        height,
        &line,
        Draw {
            scale,
            baseline,
            color: [0x14, 0x12, 0x0C, 0xFF],
            boxes: options.boxes,
        },
    );
    write_png(path, width, height, &pixels)?;
    Ok((width, height))
}

fn charset(font: &Font) -> String {
    let mut text = String::new();
    for glyph in &font.glyphs {
        if let Some(ch) = glyph.unicode.and_then(char::from_u32) {
            text.push(ch);
        }
    }
    if text.is_empty() {
        text.push(' ');
    }
    text
}

fn proof_text(font: &Font, options: &ProofOptions) -> String {
    if options.text.is_empty() {
        charset(font)
    } else {
        options.text.clone()
    }
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
}

struct Placed {
    x: f64,
    advance: f64,
    contours: Vec<Vec<(f64, f64)>>,
    outlines: Vec<Vec<Seg>>,
}

fn layout(font: &Font, text: &str) -> Result<Vec<Placed>, FoundryError> {
    let kerning = resolved_pairs(font)?;
    let mut placed: Vec<Placed> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    for (index, ch) in chars.iter().enumerate() {
        let code = u32::from(*ch);
        let Some(glyph) = font.glyphs.iter().find(|glyph| glyph.unicode == Some(code)) else {
            continue;
        };
        let mut shift = 0.0;
        if index > 0
            && let Some(previous) = font
                .glyphs
                .iter()
                .find(|glyph| glyph.unicode == Some(u32::from(chars[index - 1])))
        {
            shift = kerning
                .iter()
                .find(|(left, right, _)| left == &previous.name && right == &glyph.name)
                .map(|(_, _, value)| *value)
                .unwrap_or(0.0);
        }
        let cursor = placed
            .last()
            .map(|item| item.x + item.advance)
            .unwrap_or(0.0)
            + shift;
        placed.push(Placed {
            x: cursor,
            advance: glyph.advance,
            contours: contours_at(glyph, cursor),
            outlines: outlines_at(glyph, cursor),
        });
    }
    Ok(placed)
}

/// One drawing step in font units, with the glyph's origin already added.
enum Seg {
    Move((f64, f64)),
    Line((f64, f64)),
    Cubic((f64, f64), (f64, f64), (f64, f64)),
    Close,
}

fn outlines_at(glyph: &Glyph, origin: f64) -> Vec<Vec<Seg>> {
    glyph
        .contours
        .iter()
        .filter_map(|contour| contour_segments(contour, origin))
        .collect()
}

/// The curve steps of one contour. Off-curve runs follow the same rules as the model: one point
/// is a quadratic, two are a cubic, and more chain through implied on-curve midpoints.
fn contour_segments(contour: &Contour, origin: f64) -> Option<Vec<Seg>> {
    let count = contour.points.len();
    let first = contour
        .points
        .iter()
        .position(|point| point.kind == PointKind::On)?;
    let at = |point: &Point| (origin + point.x, point.y);
    let start = &contour.points[first];
    let mut segs = vec![Seg::Move(at(start))];
    let mut current = start;
    let mut offs: Vec<&Point> = Vec::new();
    // A closed contour walks back to its start, which is an on-curve point.
    let steps = if contour.closed { count } else { count - 1 };
    for step in 1..=steps {
        let point = &contour.points[(first + step) % count];
        if point.kind == PointKind::Off {
            offs.push(point);
            continue;
        }
        push_run(&mut segs, current, &offs, point, origin);
        offs.clear();
        current = point;
    }
    if contour.closed {
        segs.push(Seg::Close);
    }
    Some(segs)
}

fn push_run(segs: &mut Vec<Seg>, from: &Point, offs: &[&Point], to: &Point, origin: f64) {
    let at = |point: &Point| (origin + point.x, point.y);
    match offs.len() {
        0 => segs.push(Seg::Line(at(to))),
        2 => segs.push(Seg::Cubic(at(offs[0]), at(offs[1]), at(to))),
        _ => {
            let mut start = at(from);
            for pair in offs.windows(2) {
                let control = at(pair[0]);
                let mid = midpoint(control, at(pair[1]));
                push_quad(segs, start, control, mid);
                start = mid;
            }
            push_quad(segs, start, at(offs[offs.len() - 1]), at(to));
        }
    }
}

fn midpoint(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

/// A quadratic as a cubic. The two handles sit two thirds of the way to the control point.
fn push_quad(segs: &mut Vec<Seg>, from: (f64, f64), control: (f64, f64), to: (f64, f64)) {
    let toward = |end: (f64, f64)| {
        (
            end.0 + (control.0 - end.0) * 2.0 / 3.0,
            end.1 + (control.1 - end.1) * 2.0 / 3.0,
        )
    };
    segs.push(Seg::Cubic(toward(from), toward(to), to));
}

fn contours_at(glyph: &Glyph, origin: f64) -> Vec<Vec<(f64, f64)>> {
    glyph
        .contours
        .iter()
        .map(|contour| {
            let mut samples = Vec::new();
            if contour.points.is_empty() {
                return samples;
            }
            let last = if contour.closed {
                contour.points.len()
            } else {
                contour.points.len() - 1
            };
            for index in 0..last {
                let start = &contour.points[index];
                let end = &contour.points[(index + 1) % contour.points.len()];
                let steps = if start.kind == PointKind::Off || end.kind == PointKind::Off {
                    6
                } else {
                    1
                };
                for step in 0..steps {
                    let t = step as f64 / steps as f64;
                    samples.push((
                        origin + start.x + (end.x - start.x) * t,
                        start.y + (end.y - start.y) * t,
                    ));
                }
            }
            if let Some(last) = contour.points.last() {
                samples.push((origin + last.x, last.y));
            }
            samples
        })
        .collect()
}

struct Draw {
    scale: f64,
    baseline: f64,
    color: [u8; 4],
    boxes: bool,
}

fn draw_line(pixels: &mut [u8], width: u32, height: u32, line: &[Placed], draw: Draw) {
    let Draw {
        scale,
        baseline,
        color,
        boxes,
    } = draw;
    for placed in line {
        if boxes {
            let x = MARGIN + placed.x * scale;
            let right = MARGIN + (placed.x + placed.advance) * scale;
            vline(pixels, width, height, x, [0x5B, 0x4A, 0x2D, 0xFF]);
            vline(pixels, width, height, right, [0x5B, 0x4A, 0x2D, 0xFF]);
        }
        for contour in &placed.contours {
            fill_contour(pixels, width, height, contour, scale, baseline, color);
        }
    }
}

fn fill_contour(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    contour: &[(f64, f64)],
    scale: f64,
    baseline: f64,
    color: [u8; 4],
) {
    if contour.len() < 3 {
        return;
    }
    let mut mapped: Vec<(f64, f64)> = contour
        .iter()
        .map(|(x, y)| (MARGIN + *x * scale, baseline - *y * scale))
        .collect();
    if mapped.first() != mapped.last() {
        mapped.push(*mapped.first().unwrap());
    }
    let min_y = mapped
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min)
        .floor() as i32;
    let max_y = mapped
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil() as i32;
    for y in min_y.max(0)..=max_y.min(height as i32 - 1) {
        let scan = y as f64 + 0.5;
        let mut hits = Vec::new();
        for index in 0..mapped.len() - 1 {
            let a = mapped[index];
            let b = mapped[index + 1];
            if (a.1 > scan) == (b.1 > scan) || (b.1 - a.1).abs() < 1e-9 {
                continue;
            }
            let t = (scan - a.1) / (b.1 - a.1);
            hits.push(a.0 + (b.0 - a.0) * t);
        }
        hits.sort_by(f64::total_cmp);
        let mut index = 0;
        while index + 1 < hits.len() {
            let left = hits[index].ceil() as i32;
            let right = hits[index + 1].floor() as i32;
            for x in left.max(0)..=right.min(width as i32 - 1) {
                blend(pixels, width, x as u32, y as u32, color);
            }
            index += 2;
        }
    }
}

fn fill(pixels: &mut [u8], width: u32, color: [u8; 4]) {
    let height = pixels.len() / (width as usize * 4);
    for y in 0..height {
        for x in 0..width as usize {
            put(pixels, width, x as u32, y as u32, color);
        }
    }
}

fn hline(pixels: &mut [u8], width: u32, height: u32, y: f64, color: [u8; 4]) {
    let y = y.round() as i32;
    if y < 0 || y >= height as i32 {
        return;
    }
    for x in 0..width {
        blend(pixels, width, x, y as u32, color);
    }
}

fn vline(pixels: &mut [u8], width: u32, height: u32, x: f64, color: [u8; 4]) {
    let x = x.round() as i32;
    if x < 0 || x >= width as i32 {
        return;
    }
    for y in 0..height {
        blend(pixels, width, x as u32, y, color);
    }
}

fn blend(pixels: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4]) {
    let index = (y as usize * width as usize + x as usize) * 4;
    if index + 3 >= pixels.len() {
        return;
    }
    let alpha = f64::from(color[3]) / 255.0;
    for channel in 0..3 {
        let src = f64::from(color[channel]);
        let dst = f64::from(pixels[index + channel]);
        pixels[index + channel] = (src * alpha + dst * (1.0 - alpha)).round() as u8;
    }
    pixels[index + 3] = 255;
}

fn put(pixels: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4]) {
    let index = (y as usize * width as usize + x as usize) * 4;
    if index + 3 < pixels.len() {
        pixels[index..index + 4].copy_from_slice(&color);
    }
}

fn write_png(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<(), FoundryError> {
    if let Some(parent) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|err| FoundryError::Io(err.to_string()))?;
    }
    let mut raw = Vec::with_capacity(pixels.len() + height as usize);
    let stride = width as usize * 4;
    for y in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&pixels[y * stride..(y + 1) * stride]);
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder
        .write_all(&raw)
        .map_err(|err| FoundryError::Io(err.to_string()))?;
    let compressed = encoder
        .finish()
        .map_err(|err| FoundryError::Io(err.to_string()))?;
    let mut file = Vec::new();
    file.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut file, b"IHDR", &ihdr);
    chunk(&mut file, b"IDAT", &compressed);
    chunk(&mut file, b"IEND", &[]);
    fs::write(path, file).map_err(|err| FoundryError::Io(err.to_string()))
}

fn chunk(file: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    file.extend(u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
    file.extend_from_slice(kind);
    file.extend_from_slice(data);
    let mut crc_data = kind.to_vec();
    crc_data.extend_from_slice(data);
    file.extend(crc32(&crc_data).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = if crc & 1 == 1 { 0xEDB8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
        }
    }
    !crc
}

const PAPER: [u8; 4] = [0xF3, 0xEE, 0xE4, 0xFF];
const INK: [u8; 4] = [0x14, 0x12, 0x0C, 0xFF];
const GUIDE: [u8; 4] = [0x3D, 0x8B, 0xFF, 0xFF];
const BOX: [u8; 4] = [0x5B, 0x4A, 0x2D, 0xFF];
const COMPARE: [u8; 4] = [0xC4, 0x5C, 0x26, 0x88];

/// One page of vector outlines. No font is embedded, because the glyphs are paths. The content
/// stream is zlib-compressed, which is what `FlateDecode` expects.
fn write_pdf(
    font: &Font,
    path: &Path,
    text: &str,
    options: &ProofOptions,
) -> Result<(u32, u32), FoundryError> {
    let scale = options.pixel_size / f64::from(font.upm);
    let line = layout(font, text)?;
    let other = match &options.compare {
        Some(compare) => layout(compare, text).unwrap_or_default(),
        None => Vec::new(),
    };
    let width_units = line
        .last()
        .map(|placed| placed.x + placed.advance)
        .unwrap_or(1.0)
        .max(1.0);
    let width = (MARGIN * 2.0 + width_units * scale).ceil();
    let height = (MARGIN * 2.0 + options.pixel_size * 1.4).ceil();
    // PDF counts up from the bottom edge. The PNG baseline counts down from the top.
    let baseline = height - (MARGIN + font.metrics.ascender * scale);

    let mut page = String::new();
    let _ = writeln!(
        page,
        "{} rg 0 0 {width} {height} re f",
        colour(PAPER, PAPER)
    );
    if options.guides {
        let _ = writeln!(page, "{} RG 0.5 w", colour(GUIDE, PAPER));
        for metric in [
            font.metrics.ascender,
            font.metrics.cap_height,
            font.metrics.x_height,
            0.0,
            font.metrics.descender,
        ] {
            let y = baseline + metric * scale;
            let _ = writeln!(page, "0 {y:.3} m {width} {y:.3} l S");
        }
    }
    if options.boxes {
        let _ = writeln!(page, "{} RG 0.5 w", colour(BOX, PAPER));
        for placed in &line {
            for x in [placed.x, placed.x + placed.advance] {
                let x = MARGIN + x * scale;
                let _ = writeln!(page, "{x:.3} 0 m {x:.3} {height} l S");
            }
        }
    }
    fill_outlines(&mut page, &other, scale, baseline, &colour(COMPARE, PAPER));
    fill_outlines(&mut page, &line, scale, baseline, &colour(INK, PAPER));

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(page.as_bytes())
        .map_err(|err| FoundryError::Io(err.to_string()))?;
    let packed = encoder
        .finish()
        .map_err(|err| FoundryError::Io(err.to_string()))?;
    let mut stream = format!(
        "<< /Length {} /Filter /FlateDecode >>\nstream\n",
        packed.len()
    )
    .into_bytes();
    stream.extend_from_slice(&packed);
    stream.extend_from_slice(b"\nendstream");

    let objects: [Vec<u8>; 4] = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents 4 0 R /Resources << >> >>"
        )
        .into_bytes(),
        stream,
    ];
    let mut file = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, body) in objects.iter().enumerate() {
        offsets.push(file.len());
        file.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        file.extend_from_slice(body);
        file.extend_from_slice(b"\nendobj\n");
    }
    let xref = file.len();
    let count = objects.len() + 1;
    file.extend_from_slice(format!("xref\n0 {count}\n0000000000 65535 f \n").as_bytes());
    for offset in offsets {
        file.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    file.extend_from_slice(
        format!("trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );

    if let Some(parent) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|err| FoundryError::Io(err.to_string()))?;
    }
    fs::write(path, &file).map_err(|err| FoundryError::Io(err.to_string()))?;
    Ok((width as u32, height as u32))
}

/// A PDF colour triple for `color`, mixed over `backdrop` by its alpha.
fn colour(color: [u8; 4], backdrop: [u8; 4]) -> String {
    let alpha = f64::from(color[3]) / 255.0;
    let mix = |index: usize| {
        (f64::from(color[index]) * alpha + f64::from(backdrop[index]) * (1.0 - alpha)) / 255.0
    };
    format!("{:.3} {:.3} {:.3}", mix(0), mix(1), mix(2))
}

fn fill_outlines(page: &mut String, line: &[Placed], scale: f64, baseline: f64, rgb: &str) {
    if line.is_empty() {
        return;
    }
    let _ = writeln!(page, "{rgb} rg");
    let to_pdf = |point: &(f64, f64)| (MARGIN + point.0 * scale, baseline + point.1 * scale);
    for placed in line {
        for contour in &placed.outlines {
            for seg in contour {
                let _ = match seg {
                    Seg::Move(p) => {
                        let (x, y) = to_pdf(p);
                        writeln!(page, "{x:.3} {y:.3} m")
                    }
                    Seg::Line(p) => {
                        let (x, y) = to_pdf(p);
                        writeln!(page, "{x:.3} {y:.3} l")
                    }
                    Seg::Cubic(a, b, c) => {
                        let (ax, ay) = to_pdf(a);
                        let (bx, by) = to_pdf(b);
                        let (cx, cy) = to_pdf(c);
                        writeln!(page, "{ax:.3} {ay:.3} {bx:.3} {by:.3} {cx:.3} {cy:.3} c")
                    }
                    Seg::Close => writeln!(page, "h"),
                };
            }
        }
    }
    page.push_str("f\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{Contour, Glyph, Point, PointKind};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn writes_a_png_proof() {
        let mut font = Font::new("Proof", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "H".into(),
            unicode: Some(u32::from('H')),
            advance: 400.0,
            contours: vec![Contour {
                closed: true,
                points: [(80.0, 0.0), (180.0, 0.0), (180.0, 700.0), (80.0, 700.0)]
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
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("typefoundry-proof-{tick}.png"));
        let (width, height) = write_proof(
            &font,
            &path,
            &ProofOptions {
                text: "H".into(),
                ..ProofOptions::default()
            },
        )
        .unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert!(width > 10 && height > 10);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn writes_a_pdf_proof_with_curves_and_a_valid_xref() {
        use std::io::Read;

        let mut font = Font::new("Proof", 1000).unwrap();
        // A quadratic bowl: on, off, on, then a cubic back to the start.
        let on = |x: f64, y: f64| Point {
            x,
            y,
            kind: PointKind::On,
            smooth: false,
        };
        let off = |x: f64, y: f64| Point {
            x,
            y,
            kind: PointKind::Off,
            smooth: false,
        };
        font.insert_glyph(Glyph {
            name: "O".into(),
            unicode: Some(u32::from('O')),
            advance: 600.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    on(0.0, 0.0),
                    off(0.0, 350.0),
                    on(0.0, 700.0),
                    off(300.0, 700.0),
                    off(600.0, 700.0),
                    on(600.0, 0.0),
                ],
            }],
        })
        .unwrap();
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("typefoundry-proof-{tick}.pdf"));
        let (width, height) = write_proof(
            &font,
            &path,
            &ProofOptions {
                text: "O".into(),
                ..ProofOptions::default()
            },
        )
        .unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(bytes.ends_with(b"%%EOF\n"));
        // The header and the stream are binary, so every offset here is a byte offset.
        let find = |needle: &[u8]| {
            bytes
                .windows(needle.len())
                .rposition(|w| w == needle)
                .unwrap()
        };
        let media = format!("/MediaBox [0 0 {width} {height}]");
        assert!(bytes.windows(media.len()).any(|w| w == media.as_bytes()));

        // startxref must point at the xref keyword, or a viewer cannot find the objects.
        let marker = find(b"startxref\n") + "startxref\n".len();
        let digits = &bytes[marker..];
        let line_end = digits.iter().position(|&byte| byte == b'\n').unwrap();
        let offset: usize = std::str::from_utf8(&digits[..line_end])
            .unwrap()
            .parse()
            .unwrap();
        assert!(bytes[offset..].starts_with(b"xref\n0 5\n"));

        // The content stream is zlib-compressed. Inflate it and look for the path operators.
        let start = bytes.windows(7).position(|w| w == b"stream\n").unwrap() + "stream\n".len();
        let end = find(b"\nendstream");
        let mut content = String::new();
        flate2::read::ZlibDecoder::new(&bytes[start..end])
            .read_to_string(&mut content)
            .unwrap();
        assert!(
            content.contains(" c\n"),
            "a quadratic becomes a cubic: {content}"
        );
        assert!(
            content.contains("h\nf\n") || content.contains("h\n"),
            "{content}"
        );
        assert!(content.ends_with("f\n"), "{content}");
        let _ = fs::remove_file(path);
    }
}
