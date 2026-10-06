//! A folder of SVG glyphs becomes one font.
//!
//! Each file is named with the Unicode scalar as four hex digits: `0041.svg` is A.
//! `U+0041.svg` and `uni0041.svg` are accepted too. The drawings are usually tight crops
//! from one scale, so the importer puts flat letters on a shared baseline, centers round
//! letters so their overshoot is split, and hangs descenders below that baseline. The
//! x-height of the flat lowercase becomes 500 units in a 1000-unit em.

use std::fs;
use std::path::Path;

use crate::error::FoundryError;
use crate::font::{Contour, Font, Glyph, Point, PointKind};

const UPM: u16 = 1000;
const X_HEIGHT: f64 = 500.0;
const CAP_HEIGHT: f64 = 700.0;
const ASCENDER: f64 = 800.0;
const DESCENDER: f64 = -200.0;
const SIDEBEARING: f64 = 40.0;
const SPACE_ADVANCE: f64 = 250.0;

#[derive(Clone, Copy)]
struct Ink {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl Ink {
    fn width(self) -> f64 {
        self.max_x - self.min_x
    }

    fn height(self) -> f64 {
        self.max_y - self.min_y
    }

    fn mid_y(self) -> f64 {
        (self.min_y + self.max_y) / 2.0
    }
}

struct Drawn {
    unicode: u32,
    contours: Vec<Contour>,
    canvas_width: f64,
    canvas_height: f64,
    ink: Option<Ink>,
    advance: f64,
}

pub(crate) fn is_svg_font_dir(path: &Path) -> bool {
    read_dir_svgs(path).is_ok_and(|entries| entries.iter().any(|(_, file)| is_svg(file)))
}

pub(crate) fn load_svg_dir(path: &Path) -> Result<Font, FoundryError> {
    let entries = read_dir_svgs(path)?;
    let mut sources = Vec::new();
    let mut rejected = Vec::new();
    for (name, file) in entries {
        if !is_svg(&file) {
            continue;
        }
        let Some(stem) = file.file_stem().and_then(|stem| stem.to_str()) else {
            rejected.push(name);
            continue;
        };
        match codepoint(stem) {
            Some(unicode) => {
                let text = fs::read_to_string(&file)
                    .map_err(|err| FoundryError::Import(format!("{}: {err}", file.display())))?;
                sources.push((unicode, text));
            }
            None => rejected.push(name),
        }
    }
    if !rejected.is_empty() {
        rejected.sort();
        return Err(FoundryError::Import(format!(
            "{} is not a four-digit Unicode name. Name each SVG with four hex digits, like 0041.svg for A.",
            rejected.join(", ")
        )));
    }
    if sources.is_empty() {
        return Err(FoundryError::Import(format!(
            "{} has no SVG glyphs. Name each file with four hex digits, like 0041.svg for A.",
            path.display()
        )));
    }
    font_from_svgs(&font_name(path), &sources)
}

fn font_from_svgs(name: &str, sources: &[(u32, String)]) -> Result<Font, FoundryError> {
    let mut seen = Vec::new();
    let mut drawn = Vec::new();
    for (unicode, text) in sources {
        if seen.contains(unicode) {
            return Err(FoundryError::Import(format!(
                "U+{unicode:04X} is in the folder more than once"
            )));
        }
        seen.push(*unicode);
        drawn.push(draw_svg(*unicode, text)?);
    }
    drawn.sort_by_key(|glyph| glyph.unicode);
    match fit_mode(&drawn) {
        Fit::Shared => fit_shared(&mut drawn),
        Fit::Tight => fit_tight(&mut drawn, false),
        Fit::Filled => fit_tight(&mut drawn, true),
    }
    if !drawn.iter().any(|glyph| glyph.unicode == u32::from(' ')) {
        drawn.push(Drawn {
            unicode: u32::from(' '),
            contours: Vec::new(),
            canvas_width: SPACE_ADVANCE,
            canvas_height: 0.0,
            ink: None,
            advance: SPACE_ADVANCE,
        });
        drawn.sort_by_key(|glyph| glyph.unicode);
    }
    let metrics = metrics_of(&drawn);
    let mut font = Font::new(name, UPM)?;
    font.metrics.ascender = metrics.0;
    font.metrics.cap_height = metrics.1;
    font.metrics.x_height = metrics.2;
    font.metrics.baseline = 0.0;
    font.metrics.descender = metrics.3;
    for glyph in drawn {
        font.insert_glyph(Glyph {
            name: glyph_name(glyph.unicode),
            unicode: Some(glyph.unicode),
            advance: glyph.advance,
            contours: glyph.contours,
        })?;
    }
    Ok(font)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fit {
    Shared,
    Tight,
    Filled,
}

fn fit_mode(glyphs: &[Drawn]) -> Fit {
    if glyphs.len() < 2 {
        return Fit::Tight;
    }
    let mut heights: Vec<f64> = glyphs.iter().map(|glyph| glyph.canvas_height).collect();
    let Some(board) = median(&mut heights) else {
        return Fit::Tight;
    };
    let tolerance = (board * 0.01).max(0.5);
    if heights
        .iter()
        .any(|height| (height - board).abs() > tolerance)
    {
        return Fit::Tight;
    }
    match (
        ink_heights(glyphs, flat_x_height),
        ink_heights(glyphs, flat_cap),
    ) {
        (Some(x_height), Some(cap)) if cap < x_height * 1.12 => Fit::Filled,
        _ => Fit::Shared,
    }
}

fn ink_heights(glyphs: &[Drawn], pred: fn(u32) -> bool) -> Option<f64> {
    let mut heights: Vec<f64> = glyphs
        .iter()
        .filter(|glyph| pred(glyph.unicode))
        .filter_map(|glyph| glyph.ink.map(Ink::height))
        .collect();
    median(&mut heights)
}

/// One artboard. Scale it to the em, then move the flat baseline to y = 0.
fn fit_shared(glyphs: &mut [Drawn]) {
    let mut heights: Vec<f64> = glyphs.iter().map(|glyph| glyph.canvas_height).collect();
    let board = median(&mut heights).unwrap_or(f64::from(UPM));
    let scale = f64::from(UPM) / board.max(1.0);
    for glyph in glyphs.iter_mut() {
        scale_drawn(glyph, scale);
        glyph.advance = glyph.canvas_width;
    }
    let mut bottoms: Vec<f64> = glyphs
        .iter()
        .filter(|glyph| flat_baseline(glyph.unicode))
        .filter_map(|glyph| glyph.ink.map(|ink| ink.min_y))
        .collect();
    if bottoms.is_empty() {
        bottoms = glyphs
            .iter()
            .filter_map(|glyph| glyph.ink.map(|ink| ink.min_y))
            .collect();
    }
    let baseline = median(&mut bottoms).unwrap_or(0.0);
    for glyph in glyphs.iter_mut() {
        shift(glyph, 0.0, -baseline);
    }
}

/// Separate crops drawn at one scale, or letters each stretched to their own box.
fn fit_tight(glyphs: &mut [Drawn], filled: bool) {
    if filled {
        for glyph in glyphs.iter_mut() {
            let target = filled_target(glyph.unicode).unwrap_or(X_HEIGHT);
            let height = glyph.ink.map(Ink::height).unwrap_or(target).max(0.01);
            scale_drawn(glyph, target / height);
        }
    } else if let Some(reference) = x_height_source(glyphs) {
        let scale = X_HEIGHT / reference.max(0.01);
        for glyph in glyphs.iter_mut() {
            scale_drawn(glyph, scale);
        }
    }
    place_bottoms(glyphs, flat_x_height, 0.0);
    place_bottoms(glyphs, flat_cap, 0.0);
    place_bottoms(glyphs, ascender_letter, 0.0);
    place_bottoms(glyphs, flat_digit, 0.0);
    let x_height = line_top(glyphs, flat_x_height).unwrap_or(X_HEIGHT);
    let cap = line_top(glyphs, flat_cap).unwrap_or(CAP_HEIGHT);
    let figures = line_top(glyphs, flat_digit).unwrap_or(cap);
    center_overshoot(glyphs, round_x_height, x_height);
    center_overshoot(glyphs, round_cap, cap);
    center_overshoot(glyphs, round_digit, figures);
    let round_top = line_top(glyphs, round_x_height).unwrap_or(x_height);
    place_tops(glyphs, round_descender, round_top);
    place_tops(glyphs, |code| code == u32::from('y'), x_height);
    place_tops(glyphs, |code| code == u32::from('J'), cap);
    let cap_top = line_top(glyphs, round_cap).unwrap_or(cap);
    place_tops(glyphs, |code| code == u32::from('Q'), cap_top);
    let mut descender_bottoms: Vec<f64> = glyphs
        .iter()
        .filter(|glyph| round_descender(glyph.unicode) || glyph.unicode == u32::from('y'))
        .filter_map(|glyph| glyph.ink.map(|ink| ink.min_y))
        .collect();
    let descender = median(&mut descender_bottoms).unwrap_or(DESCENDER);
    place_bottoms(glyphs, |code| code == u32::from('j'), descender);
    place_bottoms(glyphs, baseline_mark, 0.0);
    let period_top = line_top(glyphs, |code| code == u32::from('.')).unwrap_or(x_height * 0.2);
    place_tops(glyphs, |code| code == u32::from(','), period_top);
    let colon_top = line_top(glyphs, |code| code == u32::from(':')).unwrap_or(x_height * 0.67);
    place_tops(glyphs, |code| code == u32::from(';'), colon_top);
    place_tops(glyphs, cap_top_mark, cap);
    place_centers(glyphs, x_height_center, x_height / 2.0);
    place_centers(glyphs, body_center, cap / 2.0);
    for glyph in glyphs.iter_mut() {
        if glyph.ink.is_none() {
            if glyph.advance == 0.0 {
                glyph.advance = if glyph.canvas_width > 1.0 {
                    glyph.canvas_width
                } else {
                    SPACE_ADVANCE
                };
            }
            continue;
        }
        if !placed(glyph.unicode) {
            place_bottom(glyph, 0.0);
        }
        bear(glyph);
    }
}

fn placed(code: u32) -> bool {
    flat_x_height(code)
        || round_x_height(code)
        || flat_cap(code)
        || round_cap(code)
        || ascender_letter(code)
        || flat_digit(code)
        || round_digit(code)
        || round_descender(code)
        || matches!(code, 0x79 | 0x6A | 0x4A | 0x51) // y j J Q
        || baseline_mark(code)
        || code == u32::from(',')
        || code == u32::from(';')
        || cap_top_mark(code)
        || x_height_center(code)
        || body_center(code)
}

fn x_height_source(glyphs: &[Drawn]) -> Option<f64> {
    ink_heights(glyphs, flat_x_height)
        .or_else(|| ink_heights(glyphs, round_x_height))
        .or_else(|| ink_heights(glyphs, flat_cap).map(|cap| cap * X_HEIGHT / CAP_HEIGHT))
        .or_else(|| {
            let mut heights: Vec<f64> = glyphs
                .iter()
                .filter_map(|glyph| glyph.ink.map(Ink::height))
                .collect();
            median(&mut heights)
        })
}

fn filled_target(code: u32) -> Option<f64> {
    if flat_x_height(code) || round_x_height(code) {
        Some(X_HEIGHT)
    } else if flat_cap(code) || round_cap(code) || flat_digit(code) || round_digit(code) {
        Some(CAP_HEIGHT)
    } else if ascender_letter(code) {
        Some(ASCENDER)
    } else if round_descender(code) || code == u32::from('y') || code == u32::from('j') {
        Some(X_HEIGHT - DESCENDER)
    } else {
        None
    }
}

fn place_bottoms(glyphs: &mut [Drawn], pred: fn(u32) -> bool, y: f64) {
    for glyph in glyphs.iter_mut().filter(|glyph| pred(glyph.unicode)) {
        place_bottom(glyph, y);
    }
}

fn place_tops(glyphs: &mut [Drawn], pred: fn(u32) -> bool, y: f64) {
    for glyph in glyphs.iter_mut().filter(|glyph| pred(glyph.unicode)) {
        place_top(glyph, y);
    }
}

fn place_centers(glyphs: &mut [Drawn], pred: fn(u32) -> bool, y: f64) {
    for glyph in glyphs.iter_mut().filter(|glyph| pred(glyph.unicode)) {
        place_center(glyph, y);
    }
}

fn center_overshoot(glyphs: &mut [Drawn], pred: fn(u32) -> bool, band: f64) {
    for glyph in glyphs.iter_mut().filter(|glyph| pred(glyph.unicode)) {
        let Some(ink) = glyph.ink else {
            continue;
        };
        let extra = ink.height() - band;
        if extra > 0.0 {
            place_bottom(glyph, -extra / 2.0);
        } else {
            place_bottom(glyph, 0.0);
        }
    }
}

fn line_top(glyphs: &[Drawn], pred: fn(u32) -> bool) -> Option<f64> {
    let mut tops: Vec<f64> = glyphs
        .iter()
        .filter(|glyph| pred(glyph.unicode))
        .filter_map(|glyph| glyph.ink.map(|ink| ink.max_y))
        .collect();
    median(&mut tops)
}

fn metrics_of(glyphs: &[Drawn]) -> (f64, f64, f64, f64) {
    let x_height = line_top(glyphs, flat_x_height)
        .or_else(|| line_top(glyphs, round_x_height))
        .unwrap_or(X_HEIGHT);
    let cap = line_top(glyphs, flat_cap)
        .or_else(|| line_top(glyphs, round_cap))
        .unwrap_or(CAP_HEIGHT);
    let ascender = line_top(glyphs, ascender_letter)
        .unwrap_or(cap)
        .max(cap)
        .max(x_height);
    let mut lows: Vec<f64> = glyphs
        .iter()
        .filter(|glyph| {
            round_descender(glyph.unicode) || matches!(glyph.unicode, 0x79 | 0x6A | 0x4A | 0x51)
        })
        .filter_map(|glyph| glyph.ink.map(|ink| ink.min_y))
        .filter(|low| *low < -1.0)
        .collect();
    let descender = median(&mut lows).unwrap_or(DESCENDER).min(0.0);
    (ascender, cap, x_height, descender)
}

fn place_bottom(glyph: &mut Drawn, y: f64) {
    if let Some(ink) = glyph.ink {
        shift(glyph, 0.0, y - ink.min_y);
    }
}

fn place_top(glyph: &mut Drawn, y: f64) {
    if let Some(ink) = glyph.ink {
        shift(glyph, 0.0, y - ink.max_y);
    }
}

fn place_center(glyph: &mut Drawn, y: f64) {
    if let Some(ink) = glyph.ink {
        shift(glyph, 0.0, y - ink.mid_y());
    }
}

fn bear(glyph: &mut Drawn) {
    let Some(ink) = glyph.ink else {
        return;
    };
    let width = ink.width();
    shift(glyph, SIDEBEARING - ink.min_x, 0.0);
    glyph.advance = SIDEBEARING + width + SIDEBEARING;
}

fn shift(glyph: &mut Drawn, dx: f64, dy: f64) {
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.x += dx;
            point.y += dy;
        }
    }
    if let Some(ink) = &mut glyph.ink {
        ink.min_x += dx;
        ink.max_x += dx;
        ink.min_y += dy;
        ink.max_y += dy;
    }
}

fn scale_drawn(glyph: &mut Drawn, factor: f64) {
    for contour in &mut glyph.contours {
        for point in &mut contour.points {
            point.x *= factor;
            point.y *= factor;
        }
    }
    glyph.canvas_width *= factor;
    glyph.canvas_height *= factor;
    glyph.advance *= factor;
    if let Some(ink) = &mut glyph.ink {
        ink.min_x *= factor;
        ink.max_x *= factor;
        ink.min_y *= factor;
        ink.max_y *= factor;
    }
}

fn flat_x_height(code: u32) -> bool {
    matches!(
        code,
        0x61 | 0x6D | 0x6E | 0x72 | 0x75 | 0x76 | 0x77 | 0x78 | 0x7A
    )
}

fn round_x_height(code: u32) -> bool {
    matches!(code, 0x63 | 0x65 | 0x6F | 0x73)
}

fn flat_cap(code: u32) -> bool {
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

fn round_cap(code: u32) -> bool {
    matches!(char::from_u32(code), Some('C' | 'G' | 'O' | 'S' | 'U'))
}

fn ascender_letter(code: u32) -> bool {
    matches!(
        char::from_u32(code),
        Some('b' | 'd' | 'f' | 'h' | 'i' | 'k' | 'l' | 't')
    )
}

fn flat_digit(code: u32) -> bool {
    matches!(char::from_u32(code), Some('1' | '4' | '5' | '7'))
}

fn round_digit(code: u32) -> bool {
    matches!(
        char::from_u32(code),
        Some('0' | '2' | '3' | '6' | '8' | '9')
    )
}

fn round_descender(code: u32) -> bool {
    matches!(char::from_u32(code), Some('g' | 'p' | 'q'))
}

fn flat_baseline(code: u32) -> bool {
    flat_x_height(code) || flat_cap(code) || ascender_letter(code) || flat_digit(code)
}

fn baseline_mark(code: u32) -> bool {
    matches!(char::from_u32(code), Some('!' | '.' | ':' | '?'))
}

fn cap_top_mark(code: u32) -> bool {
    matches!(char::from_u32(code), Some('"' | '*' | '^'))
}

fn x_height_center(code: u32) -> bool {
    matches!(char::from_u32(code), Some('+' | '<' | '=' | '>' | '~'))
}

fn body_center(code: u32) -> bool {
    matches!(
        char::from_u32(code),
        Some('(' | ')' | '/' | '[' | '\\' | ']' | '{' | '|' | '}')
    )
}

fn draw_svg(unicode: u32, text: &str) -> Result<Drawn, FoundryError> {
    let label = format!("U+{unicode:04X}");
    let tree = usvg::Tree::from_str(text, &usvg::Options::default()).map_err(|err| {
        FoundryError::Import(format!(
            "{label} is not an SVG this importer can read ({err})"
        ))
    })?;
    let height = f64::from(tree.size().height());
    let width = f64::from(tree.size().width());
    let mut contours = Vec::new();
    let mut stroked = false;
    collect_paths(tree.root(), height, &mut contours, &mut stroked);
    if contours.is_empty() {
        if unicode == u32::from(' ') {
            return Ok(Drawn {
                unicode,
                contours,
                canvas_width: width,
                canvas_height: height,
                ink: None,
                advance: width,
            });
        }
        let why = if stroked {
            "is a stroke. Export the letter as a filled path."
        } else {
            "has no filled paths. Export the letter as outlines."
        };
        return Err(FoundryError::Import(format!("{label} {why}")));
    }
    let ink = ink_of(&contours);
    Ok(Drawn {
        unicode,
        contours,
        canvas_width: width,
        canvas_height: height,
        ink,
        advance: width,
    })
}

fn collect_paths(
    group: &usvg::Group,
    canvas_height: f64,
    contours: &mut Vec<Contour>,
    stroked: &mut bool,
) {
    for node in group.children() {
        match node {
            usvg::Node::Group(group) => collect_paths(group, canvas_height, contours, stroked),
            usvg::Node::Path(path) => {
                if path.fill().is_none() {
                    if path.stroke().is_some() {
                        *stroked = true;
                    }
                    continue;
                }
                contours.extend(contours_of(path, canvas_height));
            }
            usvg::Node::Image(_) | usvg::Node::Text(_) => {}
        }
    }
}

fn contours_of(path: &usvg::Path, canvas_height: f64) -> Vec<Contour> {
    let transform = path.abs_transform();
    let map = |x: f32, y: f32| -> (f64, f64) {
        let mut point = usvg::tiny_skia_path::Point::from_xy(x, y);
        transform.map_point(&mut point);
        (f64::from(point.x), canvas_height - f64::from(point.y))
    };
    let mut contours = Vec::new();
    let mut current: Vec<Point> = Vec::new();
    let mut closed = false;
    let flush = |current: &mut Vec<Point>, closed: &mut bool, contours: &mut Vec<Contour>| {
        if current.len() >= 2 && near_point(&current[0], current.last().expect("len checked")) {
            current.pop();
            *closed = true;
        }
        if current.len() >= 2 {
            contours.push(Contour {
                closed: *closed,
                points: std::mem::take(current),
            });
        } else {
            current.clear();
        }
        *closed = false;
    };
    for segment in path.data().segments() {
        match segment {
            usvg::tiny_skia_path::PathSegment::MoveTo(point) => {
                flush(&mut current, &mut closed, &mut contours);
                let (x, y) = map(point.x, point.y);
                current.push(on(x, y));
            }
            usvg::tiny_skia_path::PathSegment::LineTo(point) => {
                let (x, y) = map(point.x, point.y);
                current.push(on(x, y));
            }
            usvg::tiny_skia_path::PathSegment::QuadTo(control, point) => {
                let (x, y) = map(control.x, control.y);
                current.push(off(x, y));
                let (x, y) = map(point.x, point.y);
                current.push(on(x, y));
            }
            usvg::tiny_skia_path::PathSegment::CubicTo(first, second, point) => {
                let (x, y) = map(first.x, first.y);
                current.push(off(x, y));
                let (x, y) = map(second.x, second.y);
                current.push(off(x, y));
                let (x, y) = map(point.x, point.y);
                current.push(on(x, y));
            }
            usvg::tiny_skia_path::PathSegment::Close => {
                closed = true;
                flush(&mut current, &mut closed, &mut contours);
            }
        }
    }
    flush(&mut current, &mut closed, &mut contours);
    contours
}

fn ink_of(contours: &[Contour]) -> Option<Ink> {
    let mut ink: Option<Ink> = None;
    for point in contours.iter().flat_map(|contour| &contour.points) {
        ink = Some(match ink {
            Some(ink) => Ink {
                min_x: ink.min_x.min(point.x),
                min_y: ink.min_y.min(point.y),
                max_x: ink.max_x.max(point.x),
                max_y: ink.max_y.max(point.y),
            },
            None => Ink {
                min_x: point.x,
                min_y: point.y,
                max_x: point.x,
                max_y: point.y,
            },
        });
    }
    ink
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

fn near_point(a: &Point, b: &Point) -> bool {
    (a.x - b.x).abs() <= 0.05 && (a.y - b.y).abs() <= 0.05
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        Some((values[mid - 1] + values[mid]) / 2.0)
    } else {
        Some(values[mid])
    }
}

fn codepoint(stem: &str) -> Option<u32> {
    let stem = stem.trim();
    let hex = if let Some(rest) = stem.strip_prefix("U+").or_else(|| stem.strip_prefix("u+")) {
        rest
    } else if let Some(rest) = stem
        .strip_prefix("uni")
        .or_else(|| stem.strip_prefix("UNI"))
    {
        rest
    } else if let Some(rest) = stem.strip_prefix('u').or_else(|| stem.strip_prefix('U')) {
        if rest.len() == 4 { rest } else { stem }
    } else {
        stem
    };
    if hex.len() != 4 || !hex.chars().all(|char| char.is_ascii_hexdigit()) {
        return None;
    }
    let code = u32::from_str_radix(hex, 16).ok()?;
    char::from_u32(code).map(|_| code)
}

fn glyph_name(code: u32) -> String {
    match code {
        0x20 => "space".to_string(),
        0x21..=0x7E => char::from_u32(code).expect("ascii").to_string(),
        _ => format!("uni{code:04X}"),
    }
}

fn font_name(path: &Path) -> String {
    let generic = [
        "svg",
        "svgs",
        "glyphs",
        "glyph",
        "chars",
        "characters",
        "export",
        "exports",
    ];
    let folder = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled");
    let source = if generic.iter().any(|name| folder.eq_ignore_ascii_case(name)) {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or(folder)
    } else {
        folder
    };
    let name = source
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if name.is_empty() {
        "Untitled".to_string()
    } else {
        name
    }
}

fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
}

fn read_dir_svgs(path: &Path) -> Result<Vec<(String, std::path::PathBuf)>, FoundryError> {
    let entries = fs::read_dir(path).map_err(|err| FoundryError::Io(err.to_string()))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| FoundryError::Io(err.to_string()))?;
        let file = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        files.push((name, file));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 1.5
    }

    fn span(font: &Font, name: &str) -> (f64, f64) {
        let glyph = font.glyph(name).unwrap();
        let mut min_y = f64::MAX;
        let mut max_y = f64::MIN;
        for point in glyph.contours.iter().flat_map(|contour| &contour.points) {
            min_y = min_y.min(point.y);
            max_y = max_y.max(point.y);
        }
        (min_y, max_y)
    }

    fn box_svg(width: f64, height: f64) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}"><path fill="black" d="M 0 0 H {width} V {height} H 0 Z"/></svg>"#
        )
    }

    #[test]
    fn four_hex_digits_name_the_character() {
        assert_eq!(codepoint("0041"), Some(0x41));
        assert_eq!(codepoint("0078"), Some(0x78));
        assert_eq!(codepoint("U+0067"), Some(0x67));
        assert_eq!(codepoint("uni0048"), Some(0x48));
        assert_eq!(codepoint("notes"), None);
        assert_eq!(codepoint("41"), None);
        assert_eq!(glyph_name(0x41), "A");
        assert_eq!(glyph_name(0x20), "space");
    }

    #[test]
    fn tight_crops_share_a_baseline_and_x_height() {
        let font = font_from_svgs(
            "Sample",
            &[
                (0x78, box_svg(10.0, 25.0)),
                (0x48, box_svg(20.0, 37.0)),
                (0x6F, box_svg(12.0, 26.0)),
                (0x70, box_svg(12.0, 37.0)),
                (0x2E, box_svg(5.0, 5.0)),
            ],
        )
        .unwrap();
        assert_eq!(font.upm, 1000);
        assert!(near(font.metrics.baseline, 0.0));
        assert!(
            near(font.metrics.x_height, 500.0),
            "{}",
            font.metrics.x_height
        );
        let (x0, x1) = span(&font, "x");
        assert!(near(x0, 0.0), "{x0}");
        assert!(near(x1, 500.0), "{x1}");
        let (h0, h1) = span(&font, "H");
        assert!(near(h0, 0.0), "{h0}");
        assert!(near(h1, 740.0), "{h1}");
        assert!(near(font.metrics.cap_height, 740.0));
        let (o0, o1) = span(&font, "o");
        assert!(o0 < -1.0, "{o0}");
        assert!(o1 > font.metrics.x_height);
        let (p0, p1) = span(&font, "p");
        assert!(p0 < -50.0, "{p0}");
        assert!(near(p1, o1));
        let (dot0, dot1) = span(&font, ".");
        assert!(near(dot0, 0.0), "{dot0}");
        assert!(dot1 < font.metrics.x_height / 2.0, "{dot1}");
        assert!(font.glyph("space").unwrap().advance > 0.0);
        assert_eq!(font.glyph("x").unwrap().unicode, Some(0x78));
    }

    #[test]
    fn a_shared_artboard_keeps_its_sidebearings() {
        let board = |body: &str| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path fill="black" d="{body}"/></svg>"#
            )
        };
        // SVG y grows down. Baseline sits at y = 80, so font y = 20 before the shift.
        let font = font_from_svgs(
            "Board",
            &[
                (0x48, board("M 10 10 H 40 V 80 H 10 Z")),
                (0x78, board("M 15 30 H 35 V 80 H 15 Z")),
            ],
        )
        .unwrap();
        let (h0, h1) = span(&font, "H");
        let (x0, x1) = span(&font, "x");
        assert!(near(h0, 0.0), "{h0}");
        assert!(near(h1, 700.0), "{h1}");
        assert!(near(x0, 0.0), "{x0}");
        assert!(near(x1, 500.0), "{x1}");
        assert!(near(font.glyph("H").unwrap().advance, 1000.0));
        assert!(near(
            font.glyph("H").unwrap().contours[0].points[0].x,
            100.0
        ));
    }

    #[test]
    fn a_cubic_keeps_two_off_curve_points() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 25"><path fill="black" d="M 0 25 C 0 0 10 0 10 25 Z"/></svg>"#;
        let font = font_from_svgs("Curve", &[(0x78, svg.to_string())]).unwrap();
        let offs = font.glyph("x").unwrap().contours[0]
            .points
            .iter()
            .filter(|point| point.kind == PointKind::Off)
            .count();
        assert_eq!(offs, 2);
    }

    #[test]
    fn a_stroke_is_refused() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path fill="none" stroke="black" d="M 0 0 L 10 10"/></svg>"#;
        let error = font_from_svgs("Stroke", &[(0x41, svg.to_string())]).unwrap_err();
        assert!(error.to_string().contains("filled path"), "{error}");
    }

    #[test]
    fn an_svg_folder_uses_the_parent_name() {
        let root = std::env::temp_dir().join(format!("typefoundry-svg-{}", std::process::id()));
        let dir = root.join("Example-Family").join("SVG");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("0078.svg"), box_svg(10.0, 25.0)).unwrap();
        fs::write(dir.join("0048.svg"), box_svg(20.0, 37.0)).unwrap();
        let font = Font::load(&dir).unwrap();
        assert_eq!(font.name, "Example Family");
        assert!(font.glyph("x").is_some());
        assert!(font.glyph("H").is_some());
        let _ = fs::remove_dir_all(&root);
    }
}
