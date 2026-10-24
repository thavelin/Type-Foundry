//! Optional raster sheets. PDF embeds pixels only; it never stores reusable font outlines.
use super::vector::{self, Outline};
use super::*;
use crate::{Font, Glyph};
use kurbo::{ParamCurve, ParamCurveDeriv, PathEl};
use std::io::Write;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
const W: u32 = 1000;
const H: u32 = 1400;
fn line(image: &mut Pixmap, a: (f32, f32), b: (f32, f32), color: [u8; 4], width: f32) {
    let mut p = PathBuilder::new();
    p.move_to(a.0, a.1);
    p.line_to(b.0, b.1);
    if let Some(path) = p.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
        image.stroke_path(
            &path,
            &paint,
            &Stroke {
                width,
                ..Stroke::default()
            },
            Transform::identity(),
            None,
        );
    }
}
fn draw(image: &mut Pixmap, g: &Glyph, x: f64, baseline: f64, scale: f64, color: [u8; 4]) {
    let mut pb = PathBuilder::new();
    for contour in &g.contours {
        for el in vector::path(contour).elements() {
            let p = |q: kurbo::Point| ((x + q.x * scale) as f32, (baseline - q.y * scale) as f32);
            match *el {
                PathEl::MoveTo(q) => {
                    let q = p(q);
                    pb.move_to(q.0, q.1);
                }
                PathEl::LineTo(q) => {
                    let q = p(q);
                    pb.line_to(q.0, q.1);
                }
                PathEl::QuadTo(a, b) => {
                    let (a, b) = (p(a), p(b));
                    pb.quad_to(a.0, a.1, b.0, b.1);
                }
                PathEl::CurveTo(a, b, c) => {
                    let (a, b, c) = (p(a), p(b), p(c));
                    pb.cubic_to(a.0, a.1, b.0, b.1, c.0, c.1);
                }
                PathEl::ClosePath => pb.close(),
            }
        }
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
        image.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}
fn specimen(image: &mut Pixmap, font: &Font, text: &str, mut x: f64, baseline: f64, scale: f64) {
    for c in text.chars() {
        if let Some(g) = super::measure::glyph(font, c) {
            draw(image, g, x, baseline, scale, [30, 34, 40, 255]);
            x += g.advance * scale;
        } else {
            x += font.upm as f64 * scale * 0.4;
        }
        if x > W as f64 - 40. {
            break;
        }
    }
}
// Original 5x7 uppercase bitmap labels keep sheets independent of system fonts.
fn bits(c: char) -> [u8; 7] {
    match c {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 2, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        _ => [0; 7],
    }
}
fn text(image: &mut Pixmap, label: &str, x: u32, y: u32) {
    for (i, c) in label.to_uppercase().chars().take(75).enumerate() {
        for (row, b) in bits(c).into_iter().enumerate() {
            for col in 0..5 {
                if b & (1 << (4 - col)) != 0 {
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let (px, py) =
                                (x + i as u32 * 12 + col * 2 + dx, y + row as u32 * 2 + dy);
                            if px < W && py < H {
                                let at = ((py * W + px) * 4) as usize;
                                image.data_mut()[at..at + 4].copy_from_slice(&[55, 65, 75, 255]);
                            }
                        }
                    }
                }
            }
        }
    }
}
fn pdf(image: &Pixmap) -> Result<Vec<u8>, String> {
    let rgb: Vec<_> = image
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| p[..3].iter().copied())
        .collect();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&rgb).map_err(|e| e.to_string())?;
    let bytes = z.finish().map_err(|e| e.to_string())?;
    let content = b"q 500 0 0 700 0 0 cm /Image Do Q\n";
    let objects=[b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 500 700] /Resources << /XObject << /Image 4 0 R >> >> /Contents 5 0 R >>".to_vec(),[format!("<< /Type /XObject /Subtype /Image /Width {W} /Height {H} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",bytes.len()).into_bytes(),bytes,b"\nendstream".to_vec()].concat(),[format!("<< /Length {} >>\nstream\n",content.len()).into_bytes(),content.to_vec(),b"endstream".to_vec()].concat()];
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(obj);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).bytes());
    for n in offsets.iter().skip(1) {
        out.extend(format!("{n:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .bytes(),
    );
    Ok(out)
}
pub(super) fn write(
    font: &Font,
    study: &Study,
    out: &Path,
    formats: &[SheetFormat],
) -> Result<(), String> {
    if formats.is_empty() {
        return Ok(());
    }
    let mut image = Pixmap::new(W, H).ok_or("could not allocate visual sheet")?;
    image.fill(tiny_skia::Color::WHITE);
    text(&mut image, "TYPEFOUNDRY OUTLINE STUDY", 40, 26);
    text(
        &mut image,
        &format!("{} {}", font.style.family, study.source.instance),
        40,
        51,
    );
    text(&mut image, "SPECIMEN   UNSHAPED ADVANCES", 40, 86);
    specimen(
        &mut image,
        font,
        "Hamburgefontsiv 0123456789",
        40.,
        165.,
        52. / font.upm as f64,
    );
    text(
        &mut image,
        "OVERLAYS   N O LOWERCASE    H O UPPERCASE",
        40,
        205,
    );
    let xh = font.metrics.x_height.max(1.);
    let scale = 130. / xh;
    for (i, c) in ['n', 'o', 'H', 'O'].into_iter().enumerate() {
        if let Some(g) = super::measure::glyph(font, c) {
            draw(
                &mut image,
                g,
                if i < 2 { 90. } else { 540. },
                415.,
                scale,
                if i % 2 == 0 {
                    [33, 110, 170, 125]
                } else {
                    [195, 80, 60, 125]
                },
            );
        }
    }
    line(
        &mut image,
        (40., 416.),
        (960., 416.),
        [190, 198, 205, 255],
        1.,
    );
    text(&mut image, "THICKNESS   BLUE THIN TO RED THICK", 40, 455);
    for (i, c) in ['n', 'o', 'H', 'O'].into_iter().enumerate() {
        if let Some(g) = super::measure::glyph(font, c) {
            let o = Outline::new(g);
            if let Some(b) = o.bounds {
                let sc = 150. / b.height().max(1.);
                let left = 45. + i as f64 * 235.;
                draw(&mut image, g, left, 675., sc, [220, 225, 231, 255]);
                for j in 1..65 {
                    let y = b.y0 + b.height() * j as f64 / 65.;
                    for (a, z) in o.intervals(y, false) {
                        let weight = ((z - a) / xh * 3.).clamp(0., 1.);
                        line(
                            &mut image,
                            ((left + a * sc) as f32, (675. - y * sc) as f32),
                            ((left + z * sc) as f32, (675. - y * sc) as f32),
                            [(230. * weight) as u8, 75, (230. * (1. - weight)) as u8, 255],
                            1.5,
                        );
                    }
                }
            }
        }
    }
    text(&mut image, "SPACING STRIPS   X IS H   NO KERNING", 40, 720);
    for (i, s) in ["nHn", "oHo", "HHH", "OHO"].iter().enumerate() {
        specimen(
            &mut image,
            font,
            s,
            40. + i as f64 * 235.,
            850.,
            85. / font.upm as f64,
        );
    }
    text(&mut image, "GREYNESS STRIP   UNSHAPED TEXT", 40, 900);
    for i in 0..3 {
        specimen(
            &mut image,
            font,
            "The quick brown fox jumps over the lazy dog 0123456789",
            40.,
            950. + i as f64 * 22.,
            22. / font.upm as f64,
        );
    }
    text(
        &mut image,
        "INNER COUNTER CURVATURE COMBS   O AND O",
        40,
        1040,
    );
    for (i, c) in ['o', 'O'].into_iter().enumerate() {
        if let Some(g) = super::measure::glyph(font, c) {
            let o = Outline::new(g);
            let sc = 170. / xh;
            let left = 85. + i as f64 * 450.;
            draw(&mut image, g, left, 1300., sc, [220, 225, 231, 255]);
            if let Some(inner) = o.inner() {
                for seg in inner.segments() {
                    for j in 1..20 {
                        let t = j as f64 / 20.;
                        let q = seg.eval(t);
                        let d = match seg {
                            kurbo::PathSeg::Line(s) => s.deriv().eval(t).to_vec2(),
                            kurbo::PathSeg::Quad(s) => s.deriv().eval(t).to_vec2(),
                            kurbo::PathSeg::Cubic(s) => s.deriv().eval(t).to_vec2(),
                        };
                        let len = d.hypot();
                        if len < 1e-9 {
                            continue;
                        }
                        let k = vector::curvature(seg, t);
                        let normal = kurbo::Vec2::new(-d.y / len, d.x / len);
                        let comb = normal * (k * xh * xh * 0.06).clamp(-100., 100.);
                        line(
                            &mut image,
                            ((left + q.x * sc) as f32, (1300. - q.y * sc) as f32),
                            (
                                (left + (q.x + comb.x) * sc) as f32,
                                (1300. - (q.y + comb.y) * sc) as f32,
                            ),
                            [40, 125, 160, 255],
                            1.,
                        );
                    }
                }
            }
        }
    }
    text(
        &mut image,
        "MEASUREMENTS USE VECTORS   CLASSIFICATION IS HEURISTIC",
        40,
        1350,
    );
    let id = hash(&serde_json::to_vec(&study.source).map_err(|e| e.to_string())?);
    for format in formats {
        match format {
            SheetFormat::Png => stable_write(
                &out.join(format!("sheet-{id}.png")),
                &image.encode_png().map_err(|e| e.to_string())?,
            )?,
            SheetFormat::Pdf => stable_write(&out.join(format!("sheet-{id}.pdf")), &pdf(&image)?)?,
        }
    }
    Ok(())
}
