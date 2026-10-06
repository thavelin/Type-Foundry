//! UFO directories share [`Font::load`] and [`Font::save`] with the JSON working file.
//!
//! Anchors and guidelines are ignored. Kerning groups, kerning pairs, and `features.fea` are
//! kept. A save over an existing UFO updates fontinfo and the default layer and leaves lib data,
//! images, other layers, and (when the font does not set them) kerning and features in place.
//! A component is flattened into the glyph on import: its base's contours are copied in and moved
//! by the component's transform, so the outline matches but the reference is not kept. A save
//! writes the flattened outline. A glyph with an image is refused. A quadratic segment is one
//! off-curve point.
//! A cubic segment is two. Implied-on qcurves are refused. A closed contour is rotated on import
//! so the first point is an on-curve.

use std::path::Path;

use norad::fontinfo::NonNegativeIntegerOrFloat;
use norad::fontinfo::StyleMapStyle;
use norad::{ContourPoint, PointType};

use crate::error::FoundryError;
use crate::font::{Contour, Font, Glyph, KernPair, Kerning, MAX_UPM, MIN_UPM, Point, PointKind};

pub(crate) fn is_ufo_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ufo"))
}

pub(crate) fn load_ufo(path: &Path) -> Result<Font, FoundryError> {
    let ufo = norad::Font::load(path).map_err(|err| FoundryError::Ufo(err.to_string()))?;
    let info = &ufo.font_info;
    let upm = read_upm(info)?;
    let mut font = Font::new(font_name(info, path), upm)?;
    apply_metrics(&mut font, info)?;
    apply_style(&mut font, info);
    apply_info(&mut font, info);
    font.kerning = import_kerning(&ufo);
    if !ufo.features.trim().is_empty() {
        font.features = Some(ufo.features.clone());
    }
    let mut glyphs: Vec<&norad::Glyph> = ufo.default_layer().iter().collect();
    glyphs.sort_by(|left, right| left.name().as_str().cmp(right.name().as_str()));
    let layer = ufo.default_layer();
    for glyph in glyphs {
        let contours = flatten_components(layer, glyph, 0)?;
        font.insert_glyph(import_glyph(glyph, &contours)?)?;
    }
    Ok(font)
}

/// A glyph can nest components inside components. Deeper than this is treated as a loop.
const MAX_COMPONENT_DEPTH: usize = 8;

/// The glyph's own contours followed by each component's contours, moved by its transform.
fn flatten_components(
    layer: &norad::Layer,
    glyph: &norad::Glyph,
    depth: usize,
) -> Result<Vec<norad::Contour>, FoundryError> {
    let name = glyph.name().as_str();
    let mut contours = glyph.contours.clone();
    for component in &glyph.components {
        if depth >= MAX_COMPONENT_DEPTH {
            return Err(FoundryError::Ufo(format!(
                "glyph {name} has components nested too deeply, or a component that uses itself"
            )));
        }
        let base_name = component.base.as_str();
        let base = layer.get_glyph(base_name).ok_or_else(|| {
            FoundryError::Ufo(format!(
                "glyph {name} uses component {base_name}, which is not in the font"
            ))
        })?;
        for contour in flatten_components(layer, base, depth + 1)? {
            contours.push(move_contour(&contour, &component.transform));
        }
    }
    Ok(contours)
}

/// Apply a UFO affine transform. Following the UFO spec, `x' = xScale·x + yxScale·y + xOffset` and
/// `y' = xyScale·x + yScale·y + yOffset`.
fn move_contour(contour: &norad::Contour, transform: &norad::AffineTransform) -> norad::Contour {
    let points = contour
        .points
        .iter()
        .map(|point| {
            let mut moved = point.clone();
            moved.x =
                transform.x_scale * point.x + transform.yx_scale * point.y + transform.x_offset;
            moved.y =
                transform.xy_scale * point.x + transform.y_scale * point.y + transform.y_offset;
            moved
        })
        .collect();
    norad::Contour::new(points, None)
}

pub(crate) fn save_ufo(font: &Font, path: &Path) -> Result<(), FoundryError> {
    // Load first when the folder is already there. norad's save deletes the folder, so this is
    // what keeps features, lib data, images, and layers the font does not edit.
    let mut ufo = if path.exists() {
        norad::Font::load(path).map_err(|err| FoundryError::Ufo(err.to_string()))?
    } else {
        norad::Font::new()
    };
    write_fontinfo(&mut ufo.font_info, font);
    let layer = ufo.default_layer_mut();
    layer.clear();
    for glyph in &font.glyphs {
        layer.insert_glyph(export_glyph(glyph)?);
    }
    if let Some(text) = crate::kerning::features_to_write(font) {
        ufo.features = text;
    }
    if let Some(kerning) = &font.kerning {
        write_kerning(&mut ufo, kerning)?;
    }
    ufo.save(path)
        .map_err(|err| FoundryError::Ufo(err.to_string()))
}

fn write_fontinfo(info: &mut norad::FontInfo, font: &Font) {
    info.family_name = Some(font.style.family.clone());
    info.style_name = Some(font.style.name.clone());
    let (legacy_family, legacy_style) = font.legacy_names();
    info.style_map_family_name = Some(legacy_family);
    info.style_map_style_name = Some(match legacy_style.as_str() {
        "Bold Italic" => StyleMapStyle::BoldItalic,
        "Bold" => StyleMapStyle::Bold,
        "Italic" => StyleMapStyle::Italic,
        _ => StyleMapStyle::Regular,
    });
    info.open_type_os2_weight_class = Some(u32::from(font.style.weight));
    info.open_type_os2_width_class = Some(width_class(font.style.width));
    info.italic_angle = Some(font.style.italic_angle);
    info.units_per_em = Some(NonNegativeIntegerOrFloat::from(u32::from(font.upm)));
    info.ascender = Some(font.metrics.ascender);
    info.cap_height = Some(font.metrics.cap_height);
    info.x_height = Some(font.metrics.x_height);
    info.descender = Some(font.metrics.descender);
    info.copyright = filled(&font.info.copyright);
    info.open_type_name_designer = filled(&font.info.designer);
    info.open_type_name_license = filled(&font.info.license);
    info.open_type_name_license_url = filled(&font.info.license_url);
    info.open_type_name_version = Some(font.info.version_label());
    info.open_type_name_unique_id = Some(font.info.export_unique_id(&font.file_stem()));
    let vendor = font.info.vendor_tag();
    info.open_type_os2_vendor_id = Some(String::from_utf8_lossy(&vendor).into_owned());
}

fn filled(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn width_class(width: u16) -> norad::fontinfo::Os2WidthClass {
    use norad::fontinfo::Os2WidthClass;
    match width {
        1 => Os2WidthClass::UltraCondensed,
        2 => Os2WidthClass::ExtraCondensed,
        3 => Os2WidthClass::Condensed,
        4 => Os2WidthClass::SemiCondensed,
        6 => Os2WidthClass::SemiExpanded,
        7 => Os2WidthClass::Expanded,
        8 => Os2WidthClass::ExtraExpanded,
        9 => Os2WidthClass::UltraExpanded,
        _ => Os2WidthClass::Normal,
    }
}

fn apply_info(font: &mut Font, info: &norad::FontInfo) {
    let take = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .unwrap_or("")
            .to_string()
    };
    font.info.copyright = take(&info.copyright);
    font.info.designer = take(&info.open_type_name_designer);
    font.info.license = take(&info.open_type_name_license);
    font.info.license_url = take(&info.open_type_name_license_url);
    font.info.version = take(&info.open_type_name_version);
    font.info.unique_id = take(&info.open_type_name_unique_id);
    if let Some(vendor) = &info.open_type_os2_vendor_id {
        let trimmed = vendor.trim();
        if !trimmed.is_empty() {
            font.info.vendor = trimmed.to_string();
        }
    }
}

fn import_kerning(ufo: &norad::Font) -> Option<Kerning> {
    let mut groups = std::collections::BTreeMap::new();
    for (name, members) in &ufo.groups {
        let key = name.as_str();
        if key.starts_with("public.kern1.") || key.starts_with("public.kern2.") {
            groups.insert(
                key.to_string(),
                members
                    .iter()
                    .map(|member| member.as_str().to_string())
                    .collect(),
            );
        }
    }
    let mut pairs = Vec::new();
    for (left, rights) in &ufo.kerning {
        for (right, value) in rights {
            pairs.push(KernPair {
                left: left.as_str().to_string(),
                right: right.as_str().to_string(),
                value: *value,
            });
        }
    }
    if groups.is_empty() && pairs.is_empty() {
        None
    } else {
        Some(Kerning {
            groups,
            pairs,
            ligatures: Vec::new(),
        })
    }
}

fn write_kerning(ufo: &mut norad::Font, kerning: &Kerning) -> Result<(), FoundryError> {
    ufo.groups.retain(|name, _| {
        let name = name.as_str();
        !name.starts_with("public.kern1.") && !name.starts_with("public.kern2.")
    });
    ufo.kerning.clear();
    for (name, members) in &kerning.groups {
        let key = ufo_name(name)?;
        let mut stored = Vec::new();
        for member in members {
            stored.push(ufo_name(member)?);
        }
        ufo.groups.insert(key, stored);
    }
    for pair in &kerning.pairs {
        let left = ufo_name(&pair.left)?;
        let right = ufo_name(&pair.right)?;
        ufo.kerning
            .entry(left)
            .or_default()
            .insert(right, pair.value);
    }
    Ok(())
}

fn ufo_name(name: &str) -> Result<norad::Name, FoundryError> {
    norad::Name::new(name).map_err(|err| FoundryError::Ufo(format!("UFO name '{name}': {err}")))
}

fn read_upm(info: &norad::FontInfo) -> Result<u16, FoundryError> {
    let Some(value) = info.units_per_em.as_ref() else {
        return Err(FoundryError::Ufo("the UFO has no units per em".to_string()));
    };
    let upm = value.as_f64();
    if !upm.is_finite()
        || upm.fract() != 0.0
        || !(f64::from(MIN_UPM)..=f64::from(MAX_UPM)).contains(&upm)
    {
        return Err(FoundryError::Ufo(format!(
            "units per em {upm} is outside {MIN_UPM}-{MAX_UPM}"
        )));
    }
    Ok(upm as u16)
}

fn font_name(info: &norad::FontInfo, path: &Path) -> String {
    let family = info
        .family_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let style = info
        .style_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    match (family, style) {
        (Some(family), Some(style)) => format!("{family} {style}"),
        (Some(family), None) => family.to_string(),
        (None, Some(style)) => style.to_string(),
        (None, None) => path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.trim().is_empty())
            .unwrap_or("Untitled")
            .to_string(),
    }
}

/// Family, style, weight, and italic from fontinfo. Missing values keep the Regular defaults.
fn apply_style(font: &mut Font, info: &norad::FontInfo) {
    let clean = |text: &Option<String>| {
        text.as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    if let Some(family) = clean(&info.family_name) {
        font.style.family = family;
    }
    if let Some(style) = clean(&info.style_name) {
        font.style.name = style;
    }
    if let Some(weight) = info.open_type_os2_weight_class {
        font.style.weight = u16::try_from(weight.clamp(1, 1000)).unwrap_or(400);
    }
    if let Some(width) = info.open_type_os2_width_class {
        font.style.width = u16::from(width as u8);
    }
    if let Some(angle) = info.italic_angle.filter(|angle| angle.is_finite()) {
        font.style.italic_angle = angle;
    }
    let mapped_italic = matches!(
        info.style_map_style_name,
        Some(StyleMapStyle::Italic | StyleMapStyle::BoldItalic)
    );
    font.style.italic = mapped_italic
        || font.style.italic_angle != 0.0
        || font.style.name.to_lowercase().contains("italic")
        || font.style.name.to_lowercase().contains("oblique");
}

fn apply_metrics(font: &mut Font, info: &norad::FontInfo) -> Result<(), FoundryError> {
    if let Some(value) = info.ascender {
        font.metrics.ascender = value;
    }
    if let Some(value) = info.cap_height {
        font.metrics.cap_height = value;
    }
    if let Some(value) = info.x_height {
        font.metrics.x_height = value;
    }
    if let Some(value) = info.descender {
        font.metrics.descender = value;
    }
    font.metrics.baseline = 0.0;
    if !font.metrics.ascender.is_finite()
        || !font.metrics.cap_height.is_finite()
        || !font.metrics.x_height.is_finite()
        || !font.metrics.descender.is_finite()
    {
        return Err(FoundryError::NonFinite);
    }
    Ok(())
}

fn import_glyph(glyph: &norad::Glyph, contours: &[norad::Contour]) -> Result<Glyph, FoundryError> {
    let name = glyph.name().as_str();
    if glyph.image.is_some() {
        return Err(FoundryError::Ufo(format!(
            "glyph {name} has an image, which is not imported"
        )));
    }
    let unicode = glyph.codepoints.iter().next().map(u32::from);
    let contours = contours
        .iter()
        .map(|contour| import_contour(contour, name))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Glyph {
        name: name.to_string(),
        unicode,
        advance: glyph.width,
        contours,
    })
}

fn import_contour(contour: &norad::Contour, glyph_name: &str) -> Result<Contour, FoundryError> {
    if contour.points.is_empty() {
        return Err(FoundryError::EmptyContour(glyph_name.to_string()));
    }
    if contour
        .points
        .iter()
        .skip(1)
        .any(|point| point.typ == PointType::Move)
    {
        return Err(FoundryError::Ufo(format!(
            "glyph {glyph_name} has a move point that is not first"
        )));
    }
    let on_indexes = on_indexes(&contour.points, |point| point.typ != PointType::OffCurve);
    let closed = contour.is_closed();
    check_segments(&contour.points, &on_indexes, closed, glyph_name)?;
    let mut points: Vec<Point> = contour
        .points
        .iter()
        .map(|point| {
            let kind = if point.typ == PointType::OffCurve {
                PointKind::Off
            } else {
                PointKind::On
            };
            Point {
                x: point.x,
                y: point.y,
                kind,
                smooth: kind == PointKind::On && point.smooth,
            }
        })
        .collect();
    // Export writes a closed contour's trailing off-curves in front of the first on-curve.
    // Rotate them back so a round trip keeps the start point.
    if closed
        && let Some(first_on) = points.iter().position(|point| point.kind == PointKind::On)
        && first_on > 0
    {
        points.rotate_left(first_on);
    }
    Ok(Contour { closed, points })
}

fn check_segments(
    points: &[norad::ContourPoint],
    on_indexes: &[usize],
    closed: bool,
    glyph_name: &str,
) -> Result<(), FoundryError> {
    if on_indexes.is_empty() {
        return Err(FoundryError::Ufo(format!(
            "glyph {glyph_name} has a contour with no on-curve points"
        )));
    }
    if !closed && (on_indexes[0] != 0 || *on_indexes.last().unwrap() != points.len() - 1) {
        return Err(FoundryError::Ufo(format!(
            "glyph {glyph_name} has an open contour that starts or ends off-curve"
        )));
    }
    for (nth, &on_index) in on_indexes.iter().enumerate() {
        let offs = off_count(points.len(), on_indexes, nth, closed);
        let typ = points[on_index].typ;
        let valid = match typ {
            PointType::Move => !closed && nth == 0 && offs == 0,
            PointType::Line => offs == 0,
            PointType::QCurve => offs == 1,
            PointType::Curve => offs <= 2,
            PointType::OffCurve => false,
        };
        if !valid {
            return Err(FoundryError::Ufo(format!(
                "glyph {glyph_name} has a {typ} point with {offs} off-curve points before it"
            )));
        }
    }
    Ok(())
}

fn export_glyph(glyph: &Glyph) -> Result<norad::Glyph, FoundryError> {
    if norad::Name::new(&glyph.name).is_err() {
        return Err(FoundryError::Ufo(format!(
            "glyph name {} is not a valid UFO name",
            glyph.name
        )));
    }
    let mut exported = norad::Glyph::new(&glyph.name);
    exported.width = glyph.advance;
    if let Some(code) = glyph.unicode {
        let Some(ch) = char::from_u32(code) else {
            return Err(FoundryError::Ufo(format!(
                "glyph {} has a unicode value that is not a character",
                glyph.name
            )));
        };
        exported.codepoints.insert(ch);
    }
    for contour in &glyph.contours {
        exported
            .contours
            .push(export_contour(contour, &glyph.name)?);
    }
    Ok(exported)
}

fn export_contour(contour: &Contour, glyph_name: &str) -> Result<norad::Contour, FoundryError> {
    let on_indexes = on_indexes(&contour.points, |point| point.kind == PointKind::On);
    if on_indexes.is_empty() {
        return Err(FoundryError::Ufo(format!(
            "glyph {glyph_name} has a contour with no on-curve points"
        )));
    }
    if !contour.closed
        && (on_indexes[0] != 0 || *on_indexes.last().unwrap() != contour.points.len() - 1)
    {
        return Err(FoundryError::Ufo(format!(
            "glyph {glyph_name} has an open contour that starts or ends off-curve"
        )));
    }
    let mut exported = Vec::with_capacity(contour.points.len());
    for (nth, &on_index) in on_indexes.iter().enumerate() {
        let offs = off_count(contour.points.len(), &on_indexes, nth, contour.closed);
        let typ = if !contour.closed && nth == 0 {
            PointType::Move
        } else {
            match offs {
                0 => PointType::Line,
                1 => PointType::QCurve,
                2 => PointType::Curve,
                _ => {
                    return Err(FoundryError::Ufo(format!(
                        "glyph {glyph_name} has a segment with {offs} off-curve points"
                    )));
                }
            }
        };
        for index in between_indexes(contour.points.len(), &on_indexes, nth, contour.closed) {
            exported.push(ufo_point(&contour.points[index], PointType::OffCurve));
        }
        exported.push(ufo_point(&contour.points[on_index], typ));
    }
    if exported.len() != contour.points.len() {
        return Err(FoundryError::Ufo(format!(
            "glyph {glyph_name} has a contour that could not be written"
        )));
    }
    Ok(norad::Contour::new(exported, None))
}

fn ufo_point(point: &Point, typ: PointType) -> ContourPoint {
    ContourPoint::new(
        point.x,
        point.y,
        typ,
        typ != PointType::OffCurve && point.smooth,
        None,
        None,
    )
}

fn on_indexes<T>(points: &[T], is_on: impl Fn(&T) -> bool) -> Vec<usize> {
    points
        .iter()
        .enumerate()
        .filter(|(_, point)| is_on(point))
        .map(|(index, _)| index)
        .collect()
}

fn off_count(len: usize, on_indexes: &[usize], nth: usize, closed: bool) -> usize {
    between_indexes(len, on_indexes, nth, closed).len()
}

fn between_indexes(len: usize, on_indexes: &[usize], nth: usize, closed: bool) -> Vec<usize> {
    if !closed && nth == 0 {
        return Vec::new();
    }
    let from = if nth == 0 {
        *on_indexes.last().unwrap()
    } else {
        on_indexes[nth - 1]
    };
    let to = on_indexes[nth];
    let mut indexes = Vec::new();
    if len == 0 {
        return indexes;
    }
    let mut index = (from + 1) % len;
    while index != to {
        indexes.push(index);
        index = (index + 1) % len;
        if indexes.len() > len {
            break;
        }
    }
    indexes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend_fonts;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_IDS: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = TEMP_IDS.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("typefoundry-ufo-{tick}-{id}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn line(x: f64, y: f64) -> ContourPoint {
        ContourPoint::new(x, y, PointType::Line, false, None, None)
    }

    fn write_square(path: &Path, family: &str, x: f64, advance: f64) {
        let mut font = norad::Font::new();
        font.font_info.family_name = Some(family.to_string());
        font.font_info.units_per_em = Some(NonNegativeIntegerOrFloat::from(1000_u32));
        let mut glyph = norad::Glyph::new("H");
        glyph.width = advance;
        glyph.codepoints.insert('H');
        glyph.contours.push(norad::Contour::new(
            vec![
                line(x, 0.0),
                line(x + 80.0, 0.0),
                line(x + 80.0, 120.0),
                line(x, 120.0),
            ],
            None,
        ));
        let mut later = norad::Glyph::new("Z");
        later.width = advance;
        font.default_layer_mut().insert_glyph(later);
        font.default_layer_mut().insert_glyph(glyph);
        font.save(path).unwrap();
    }

    #[test]
    fn blends_two_synthetic_ufos_at_the_midpoint() {
        let dir = temp_dir();
        let narrow = dir.join("Narrow.ufo");
        let wide = dir.join("Wide.ufo");
        let mid = dir.join("Mid.ufo");
        write_square(&narrow, "Narrow", 40.0, 400.0);
        write_square(&wide, "Wide", 140.0, 800.0);

        let left = Font::load(&narrow).unwrap();
        let right = Font::load(&wide).unwrap();
        assert_eq!(left.glyph_names(), vec!["H", "Z"]);
        let blended = blend_fonts(&left, &right, 0.5).unwrap();
        blended.save(&mid).unwrap();
        let opened = Font::load(&mid).unwrap();
        // UFO stores family and style apart; the blend of two families is its own family.
        assert_eq!(opened.style.family, "Narrow / Wide @ 0.5");
        assert_eq!(opened.style.name, "Regular");
        assert_eq!(opened.name, "Narrow / Wide @ 0.5 Regular");
        assert_eq!(opened.glyph("H").unwrap().advance, 600.0);
        assert_eq!(opened.glyph("H").unwrap().contours[0].points[0].x, 90.0);
        assert_eq!(opened.glyph("H").unwrap().unicode, Some('H' as u32));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn round_trips_a_cubic_and_an_open_quadratic() {
        let dir = temp_dir();
        let path = dir.join("curves.ufo");
        let mut font = Font::new("Curves", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "C".into(),
            unicode: Some('C' as u32),
            advance: 500.0,
            contours: vec![
                Contour {
                    closed: true,
                    points: vec![
                        Point {
                            x: 0.0,
                            y: 0.0,
                            kind: PointKind::On,
                            smooth: false,
                        },
                        Point {
                            x: 40.0,
                            y: 80.0,
                            kind: PointKind::Off,
                            smooth: false,
                        },
                        Point {
                            x: 80.0,
                            y: 80.0,
                            kind: PointKind::Off,
                            smooth: false,
                        },
                        Point {
                            x: 120.0,
                            y: 0.0,
                            kind: PointKind::On,
                            smooth: true,
                        },
                    ],
                },
                Contour {
                    closed: false,
                    points: vec![
                        Point {
                            x: 10.0,
                            y: 10.0,
                            kind: PointKind::On,
                            smooth: false,
                        },
                        Point {
                            x: 30.0,
                            y: 40.0,
                            kind: PointKind::Off,
                            smooth: false,
                        },
                        Point {
                            x: 50.0,
                            y: 10.0,
                            kind: PointKind::On,
                            smooth: false,
                        },
                    ],
                },
            ],
        })
        .unwrap();
        font.save(&path).unwrap();
        let opened = Font::load(&path).unwrap();
        assert_eq!(opened.glyph("C"), font.glyph("C"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_component_is_flattened_with_its_transform() {
        let dir = temp_dir();
        let path = dir.join("parts.ufo");
        let mut font = norad::Font::new();
        font.font_info.family_name = Some("Parts".into());
        font.font_info.units_per_em = Some(NonNegativeIntegerOrFloat::from(1000_u32));
        let mut base = norad::Glyph::new("B");
        base.contours.push(norad::Contour::new(
            vec![line(0.0, 0.0), line(10.0, 0.0), line(10.0, 10.0)],
            None,
        ));
        let mut accented = norad::Glyph::new("A");
        // Shifted right by 100, and y picks up half of x through yxScale, so (10, 10) lands at (15, 10).
        accented.components.push(norad::Component::new(
            norad::Name::new("B").unwrap(),
            norad::AffineTransform {
                x_scale: 1.0,
                xy_scale: 0.0,
                yx_scale: 0.5,
                y_scale: 1.0,
                x_offset: 100.0,
                y_offset: 0.0,
            },
            None,
        ));
        font.default_layer_mut().insert_glyph(base);
        font.default_layer_mut().insert_glyph(accented);
        font.save(&path).unwrap();

        let loaded = Font::load(&path).unwrap();
        let a = loaded.glyph("A").unwrap();
        assert_eq!(a.contours.len(), 1);
        let xs: Vec<(f64, f64)> = a.contours[0].points.iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(xs, vec![(100.0, 0.0), (110.0, 0.0), (115.0, 10.0)]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_component_that_uses_itself_is_named() {
        let dir = temp_dir();
        let path = dir.join("loop.ufo");
        let mut font = norad::Font::new();
        font.font_info.family_name = Some("Loop".into());
        font.font_info.units_per_em = Some(NonNegativeIntegerOrFloat::from(1000_u32));
        let mut looped = norad::Glyph::new("A");
        looped.components.push(norad::Component::new(
            norad::Name::new("A").unwrap(),
            norad::AffineTransform::default(),
            None,
        ));
        font.default_layer_mut().insert_glyph(looped);
        font.save(&path).unwrap();
        let error = Font::load(&path).unwrap_err();
        assert!(error.to_string().contains("glyph A"), "{error}");
        assert!(error.to_string().contains("nested too deeply"), "{error}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn saving_over_a_ufo_keeps_features_and_round_trips_kerning() {
        let dir = temp_dir();
        let path = dir.join("Kept.ufo");
        let mut font = Font::new("Kept", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "H".into(),
            unicode: Some(u32::from('H')),
            advance: 400.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    Point {
                        x: 40.0,
                        y: 0.0,
                        kind: PointKind::On,
                        smooth: false,
                    },
                    Point {
                        x: 120.0,
                        y: 0.0,
                        kind: PointKind::On,
                        smooth: false,
                    },
                    Point {
                        x: 120.0,
                        y: 120.0,
                        kind: PointKind::On,
                        smooth: false,
                    },
                    Point {
                        x: 40.0,
                        y: 120.0,
                        kind: PointKind::On,
                        smooth: false,
                    },
                ],
            }],
        })
        .unwrap();
        font.save(&path).unwrap();
        fs::write(
            path.join("features.fea"),
            "feature liga {\nsub f i by fi;\n} liga;\n",
        )
        .unwrap();
        font.save_with(&path, true).unwrap();
        let kept = fs::read_to_string(path.join("features.fea")).unwrap();
        assert!(kept.contains("fi"), "{kept}");

        let mut kerning = Kerning::default();
        kerning
            .groups
            .insert("public.kern1.round".into(), vec!["o".into(), "c".into()]);
        kerning.pairs.push(KernPair {
            left: "H".into(),
            right: "H".into(),
            value: -20.0,
        });
        font.set_kerning(kerning).unwrap();
        font.save_with(&path, true).unwrap();
        let opened = Font::load(&path).unwrap();
        let kerning = opened.kerning.as_ref().unwrap();
        assert_eq!(kerning.pairs.len(), 1);
        assert_eq!(kerning.pairs[0].left, "H");
        assert_eq!(kerning.pairs[0].value, -20.0);
        assert!(
            kerning
                .groups
                .get("public.kern1.round")
                .is_some_and(|members| members.len() == 2)
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_closed_contour_that_ends_on_off_curves_keeps_its_start() {
        let dir = temp_dir();
        let path = dir.join("start.ufo");
        let mut font = Font::new("Start", 1000).unwrap();
        font.insert_glyph(Glyph {
            name: "o".into(),
            unicode: Some(u32::from('o')),
            advance: 400.0,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    Point {
                        x: 10.0,
                        y: 0.0,
                        kind: PointKind::On,
                        smooth: false,
                    },
                    Point {
                        x: 20.0,
                        y: 0.0,
                        kind: PointKind::On,
                        smooth: false,
                    },
                    Point {
                        x: 30.0,
                        y: 10.0,
                        kind: PointKind::Off,
                        smooth: false,
                    },
                    Point {
                        x: 5.0,
                        y: 10.0,
                        kind: PointKind::Off,
                        smooth: false,
                    },
                ],
            }],
        })
        .unwrap();
        font.save(&path).unwrap();
        let opened = Font::load(&path).unwrap();
        let points = &opened.glyph("o").unwrap().contours[0].points;
        assert_eq!(points[0].kind, PointKind::On);
        assert_eq!(points[0].x, 10.0);
        assert_eq!(points[2].kind, PointKind::Off);
        let _ = fs::remove_dir_all(dir);
    }
}
