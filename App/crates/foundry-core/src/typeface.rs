//! Facetype / Three.js typeface JSON, the shape published by json-fonts.
//!
//! A glyph outline is a string of `m`, `l`, `q`, `b`, and `z` commands.
//! `q` is one off-curve point. `b` is two. The result is a Type Foundry font.

use serde_json::Value;

use crate::error::FoundryError;
use crate::font::{Contour, Font, Glyph, Point, PointKind};

pub(crate) fn is_typeface(value: &Value) -> bool {
    let Some(glyphs) = value.get("glyphs").and_then(Value::as_object) else {
        return false;
    };
    value.get("resolution").is_some()
        || glyphs
            .values()
            .any(|glyph| glyph.get("o").is_some() || glyph.get("ha").is_some())
}

pub(crate) fn load_value(value: &Value) -> Result<Font, FoundryError> {
    let resolution = value
        .get("resolution")
        .and_then(Value::as_f64)
        .ok_or_else(|| FoundryError::Import("typeface JSON has no resolution".to_string()))?;
    if !resolution.is_finite() || (resolution - resolution.round()).abs() > 0.001 {
        return Err(FoundryError::Import(format!(
            "typeface resolution {resolution} is not a whole number"
        )));
    }
    let rounded = resolution.round();
    if !(16.0..=16384.0).contains(&rounded) {
        return Err(FoundryError::Import(format!(
            "typeface resolution {rounded} is outside 16-16384"
        )));
    }
    let upm = rounded as u16;
    let name = value
        .get("familyName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Imported");
    let mut font = Font::new(name, upm)?;
    let scale = f64::from(upm) / 1000.0;
    font.metrics.ascender = number(value, "ascender").unwrap_or(800.0 * scale);
    font.metrics.descender = number(value, "descender").unwrap_or(-200.0 * scale);

    let glyphs = value
        .get("glyphs")
        .and_then(Value::as_object)
        .ok_or_else(|| FoundryError::Import("typeface JSON has no glyphs".to_string()))?;
    let mut parsed = Vec::with_capacity(glyphs.len());
    for (key, glyph) in glyphs {
        parsed.push(parse_glyph(key, glyph)?);
    }
    parsed.sort_by(|a, b| a.unicode.cmp(&b.unicode).then_with(|| a.name.cmp(&b.name)));
    for glyph in parsed {
        font.insert_glyph(glyph)?;
    }
    apply_heights(&mut font, scale);
    font.validate()?;
    Ok(font)
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}

fn parse_glyph(key: &str, value: &Value) -> Result<Glyph, FoundryError> {
    let (name, unicode) = name_and_unicode(key);
    if name.trim().is_empty() {
        return Err(FoundryError::Import(
            "a typeface glyph has an empty name".to_string(),
        ));
    }
    let advance = value.get("ha").and_then(Value::as_f64).unwrap_or(0.0);
    if !advance.is_finite() {
        return Err(FoundryError::Import(format!(
            "glyph {name} has an advance that is not a finite number"
        )));
    }
    let outline = value.get("o").and_then(Value::as_str).unwrap_or("");
    Ok(Glyph {
        strokes: Vec::new(),
        name: name.clone(),
        unicode,
        advance,
        contours: parse_outline(&name, outline)?,
    })
}

fn name_and_unicode(key: &str) -> (String, Option<u32>) {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return (String::new(), None);
    };
    if chars.next().is_some() {
        return (key.to_string(), None);
    }
    let code = u32::from(first);
    let name = if first == ' ' {
        "space".to_string()
    } else if first.is_control() || first.is_whitespace() {
        format!("uni{code:04X}")
    } else {
        first.to_string()
    };
    (name, Some(code))
}

fn parse_outline(glyph: &str, text: &str) -> Result<Vec<Contour>, FoundryError> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut index = 0;
    let mut contours = Vec::new();
    let mut current = Vec::new();
    let mut closed = false;
    let mut started = false;

    while index < tokens.len() {
        let command = tokens[index];
        index += 1;
        match command {
            "m" => {
                finish(glyph, &mut current, &mut closed, &mut contours)?;
                let (x, y) = take_pair(glyph, &tokens, &mut index)?;
                current.push(on(x, y));
                started = true;
            }
            "l" => {
                require_move(glyph, started)?;
                let (x, y) = take_pair(glyph, &tokens, &mut index)?;
                current.push(on(x, y));
            }
            // typeface.js writes the end point first, then the control points:
            // `q x y cx cy` and `b x y c1x c1y c2x c2y`.
            "q" => {
                require_move(glyph, started)?;
                let (x, y) = take_pair(glyph, &tokens, &mut index)?;
                let (cx, cy) = take_pair(glyph, &tokens, &mut index)?;
                current.push(off(cx, cy));
                current.push(on(x, y));
            }
            "b" => {
                require_move(glyph, started)?;
                let (x, y) = take_pair(glyph, &tokens, &mut index)?;
                let (c1x, c1y) = take_pair(glyph, &tokens, &mut index)?;
                let (c2x, c2y) = take_pair(glyph, &tokens, &mut index)?;
                current.push(off(c1x, c1y));
                current.push(off(c2x, c2y));
                current.push(on(x, y));
            }
            "z" => {
                require_move(glyph, started)?;
                closed = true;
                finish(glyph, &mut current, &mut closed, &mut contours)?;
                started = false;
            }
            other => {
                return Err(FoundryError::Import(format!(
                    "glyph {glyph} has an unknown path command {other}"
                )));
            }
        }
    }
    finish(glyph, &mut current, &mut closed, &mut contours)?;
    Ok(contours)
}

fn require_move(glyph: &str, started: bool) -> Result<(), FoundryError> {
    if started {
        Ok(())
    } else {
        Err(FoundryError::Import(format!(
            "glyph {glyph} has a path command before a move"
        )))
    }
}

fn finish(
    glyph: &str,
    current: &mut Vec<Point>,
    closed: &mut bool,
    contours: &mut Vec<Contour>,
) -> Result<(), FoundryError> {
    if current.is_empty() {
        *closed = false;
        return Ok(());
    }
    if current.len() >= 2 && same_point(&current[0], current.last().expect("len checked")) {
        current.pop();
        *closed = true;
    }
    if current.is_empty() {
        return Err(FoundryError::Import(format!(
            "glyph {glyph} has a contour with no points"
        )));
    }
    contours.push(Contour {
        closed: *closed,
        points: std::mem::take(current),
    });
    *closed = false;
    Ok(())
}

fn take_pair(glyph: &str, tokens: &[&str], index: &mut usize) -> Result<(f64, f64), FoundryError> {
    if *index + 1 >= tokens.len() {
        return Err(FoundryError::Import(format!(
            "glyph {glyph} has a path command that is missing a number"
        )));
    }
    let x = parse_coord(glyph, tokens[*index])?;
    let y = parse_coord(glyph, tokens[*index + 1])?;
    *index += 2;
    Ok((x, y))
}

fn parse_coord(glyph: &str, token: &str) -> Result<f64, FoundryError> {
    let value: f64 = token.parse().map_err(|_| {
        FoundryError::Import(format!(
            "glyph {glyph} has a path number that is not numeric"
        ))
    })?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(FoundryError::Import(format!(
            "glyph {glyph} has a coordinate that is not a finite number"
        )))
    }
}

fn apply_heights(font: &mut Font, scale: f64) {
    font.metrics.cap_height = top_of(font, "H").unwrap_or(700.0 * scale);
    font.metrics.x_height = top_of(font, "x").unwrap_or(500.0 * scale);
    font.metrics.baseline = 0.0;
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

    #[test]
    fn reads_a_square_a_quadratic_and_a_cubic() {
        let text = r#"{
            "familyName": "Tiny",
            "resolution": 1000,
            "ascender": 800,
            "descender": -200,
            "glyphs": {
                "A": {"ha": 500, "o": "m 10 0 l 90 0 l 90 80 l 10 80 z"},
                "B": {"ha": 400, "o": "m 0 0 q 100 0 50 100 z"},
                "C": {"ha": 300, "o": "m 0 0 b 80 0 0 80 80 80 z"}
            }
        }"#;
        let font = load_text(text).unwrap();
        assert_eq!(font.name, "Tiny");
        assert_eq!(font.upm, 1000);
        assert_eq!(font.metrics.ascender, 800.0);
        let a = font.glyph("A").unwrap();
        assert_eq!(a.unicode, Some(u32::from('A')));
        assert_eq!(a.advance, 500.0);
        assert!(a.contours[0].closed);
        assert_eq!(a.contours[0].points.len(), 4);
        assert_eq!(a.contours[0].points[0].x, 10.0);
        let b = font.glyph("B").unwrap();
        assert_eq!(b.contours[0].points[1].kind, PointKind::Off);
        assert_eq!(
            (b.contours[0].points[1].x, b.contours[0].points[1].y),
            (50.0, 100.0),
            "the control point is the second pair"
        );
        assert_eq!(
            b.contours[0].points[2].x, 100.0,
            "the end point is the first pair"
        );
        let c = font.glyph("C").unwrap();
        assert_eq!(
            c.contours[0].points[1].y, 80.0,
            "first control is the second pair"
        );
        assert_eq!(
            c.contours[0].points[3].x, 80.0,
            "the end point is the first pair"
        );
        assert_eq!(
            c.contours[0]
                .points
                .iter()
                .filter(|point| point.kind == PointKind::Off)
                .count(),
            2
        );
    }

    #[test]
    fn roboto_english_and_cyrillic_keep_only_those_scripts() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Fonts");
        let english = Font::load(&root.join("Roboto-English.json")).unwrap();
        let cyrillic = Font::load(&root.join("Roboto-Cyrillic.json")).unwrap();
        assert_eq!(english.name, "Roboto English");
        assert_eq!(cyrillic.name, "Roboto Cyrillic");
        assert_eq!(english.upm, 1000);
        assert_eq!(english.glyph("A").unwrap().advance, 899.0);
        assert_eq!(english.glyph("A").unwrap().contours[0].points[0].x, 657.0);
        assert!(english.glyph("А").is_none());
        assert_eq!(cyrillic.glyph("А").unwrap().unicode, Some(0x0410));
        assert!(cyrillic.glyph("A").is_none());
        assert!(english.glyphs.iter().all(|glyph| {
            glyph
                .unicode
                .is_some_and(|code| (0x20..=0x7E).contains(&code))
        }));
        assert!(cyrillic.glyphs.iter().all(|glyph| {
            glyph
                .unicode
                .is_some_and(|code| (0x0400..=0x04FF).contains(&code))
        }));
        assert_eq!(english.glyphs.len(), 95);
        assert_eq!(cyrillic.glyphs.len(), 255);
    }

    #[test]
    #[ignore = "regenerate Fonts/Roboto-English.json and Fonts/Roboto-Cyrillic.json"]
    fn export_roboto_english_and_cyrillic() {
        let src = std::env::var("TYPEFOUNDRY_ROBOTO_SRC").unwrap();
        let font = Font::load(std::path::Path::new(&src)).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Fonts");
        std::fs::create_dir_all(&root).unwrap();
        write_subset(
            &font,
            &root.join("Roboto-English.json"),
            "Roboto English",
            |code| (0x20..=0x7E).contains(&code),
        );
        write_subset(
            &font,
            &root.join("Roboto-Cyrillic.json"),
            "Roboto Cyrillic",
            |code| (0x0400..=0x04FF).contains(&code),
        );
    }

    fn write_subset(font: &Font, path: &std::path::Path, name: &str, keep: impl Fn(u32) -> bool) {
        let mut subset = font.clone();
        subset.name = name.to_string();
        subset
            .glyphs
            .retain(|glyph| glyph.unicode.is_some_and(&keep));
        subset.save(path).unwrap();
    }

    #[test]
    fn names_a_space_and_refuses_a_broken_command() {
        let text = r#"{
            "familyName": "Tiny",
            "resolution": 1000,
            "glyphs": { " ": {"ha": 250, "o": ""} }
        }"#;
        let font = load_text(text).unwrap();
        assert_eq!(font.glyph("space").unwrap().unicode, Some(32));

        let broken = r#"{"resolution":1000,"glyphs":{"A":{"ha":1,"o":"l 1 2"}}}"#;
        let error = load_text(broken).unwrap_err();
        assert!(error.to_string().contains("before a move"));
    }
}
