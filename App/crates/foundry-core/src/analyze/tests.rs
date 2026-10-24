use super::*;
use crate::{Contour, Font, Glyph, Point, PointKind};
use kurbo::{Ellipse, ParamCurve, PathEl, Shape};
use std::sync::atomic::{AtomicU64, Ordering};
static SEQ: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "foundry-analyze-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn font(&self, name: &str, font: &Font) -> String {
        let p = self.0.join(name);
        font.save(&p).unwrap();
        p.to_string_lossy().into_owned()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn point(x: f64, y: f64, kind: PointKind) -> Point {
    Point {
        x,
        y,
        kind,
        smooth: false,
    }
}
fn polygon(points: &[(f64, f64)]) -> Contour {
    Contour {
        closed: true,
        points: points
            .iter()
            .map(|(x, y)| point(*x, *y, PointKind::On))
            .collect(),
    }
}
fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64, reverse: bool) -> Contour {
    let path = Ellipse::new((cx, cy), (rx, ry), 0.).to_path(0.01);
    let mut points = Vec::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => points.push(point(p.x, p.y, PointKind::On)),
            PathEl::CurveTo(a, b, c) => {
                points.push(point(a.x, a.y, PointKind::Off));
                points.push(point(b.x, b.y, PointKind::Off));
                points.push(point(c.x, c.y, PointKind::On));
            }
            _ => {}
        }
    }
    if points
        .first()
        .zip(points.last())
        .is_some_and(|(a, b)| a.x == b.x && a.y == b.y)
    {
        points.pop();
    }
    if reverse {
        points.reverse();
    }
    Contour {
        closed: true,
        points,
    }
}
fn fixture(class: &str) -> Font {
    let mut f = Font::new(class, 1000).unwrap();
    f.style.family = class.into();
    f.metrics.x_height = 500.;
    f.metrics.cap_height = 700.;
    f.metrics.ascender = 800.;
    f.metrics.descender = -200.;
    for c in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 ".chars() {
        let h = if c.is_uppercase() { 700. } else { 500. };
        let advance = if class == "mono" {
            600.
        } else if c.is_uppercase() {
            700.
        } else {
            550.
        };
        let contours = if c == ' ' {
            vec![]
        } else if c == 'o' || c == 'O' {
            let scale = if c == 'O' { 1.4 } else { 1. };
            let width = if class == "geometric sans" || class == "mono" {
                500.
            } else {
                400.
            };
            let bar = if class == "humanist sans" || class == "serif text" {
                25.
            } else {
                50.
            };
            vec![
                ellipse(
                    50. + width * scale / 2.,
                    h / 2.,
                    width * scale / 2.,
                    h / 2.,
                    false,
                ),
                ellipse(
                    50. + width * scale / 2.,
                    h / 2.,
                    width * scale / 2. - 50.,
                    h / 2. - bar,
                    true,
                ),
            ]
        } else if c == 'n' {
            if class == "serif text" {
                vec![polygon(&[
                    (20., 0.),
                    (130., 0.),
                    (130., 30.),
                    (100., 30.),
                    (100., 400.),
                    (350., 400.),
                    (350., 0.),
                    (400., 0.),
                    (400., 500.),
                    (50., 500.),
                    (50., 30.),
                    (20., 30.),
                ])]
            } else {
                vec![polygon(&[
                    (50., 0.),
                    (100., 0.),
                    (100., 400.),
                    (350., 400.),
                    (350., 0.),
                    (400., 0.),
                    (400., 500.),
                    (50., 500.),
                ])]
            }
        } else if c == 'H' || c == 'I' {
            if class == "serif text" {
                vec![polygon(&[
                    (20., 0.),
                    (130., 0.),
                    (130., 30.),
                    (100., 30.),
                    (100., 700.),
                    (50., 700.),
                    (50., 30.),
                    (20., 30.),
                ])]
            } else {
                vec![polygon(&[(50., 0.), (100., 0.), (100., h), (50., h)])]
            }
        } else {
            vec![polygon(&[(50., 0.), (100., 0.), (100., h), (50., h)])]
        };
        f.insert_glyph(Glyph {
            strokes: Vec::new(),
            name: if c == ' ' {
                "space".into()
            } else {
                c.to_string()
            },
            unicode: Some(c as u32),
            advance,
            contours,
        })
        .unwrap();
    }
    f
}
#[test]
fn four_original_classes_and_stems() {
    let dir = Temp::new();
    for class in ["geometric sans", "humanist sans", "serif text", "mono"] {
        let path = dir.font(&format!("{class}.json"), &fixture(class));
        let report = analyze_fonts(&[path], &Options::default()).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let s = &report.fonts[0];
        assert_eq!(s.classification.class, class, "{:?}", s.classification);
        assert_eq!(s.classification.reasons.len(), 3);
        assert!((s.metrics["weight.stem_x_height"].value.unwrap() - 0.1).abs() < 1e-6);
        assert!((s.metrics["proportions.x_cap"].value.unwrap() - 5. / 7.).abs() < 1e-6);
    }
}
#[test]
fn exact_bounds_ignore_control_point_hull() {
    let g = Glyph {
        strokes: Vec::new(),
        name: "arch".into(),
        unicode: None,
        advance: 1000.,
        contours: vec![Contour {
            closed: true,
            points: vec![
                point(0., 0., PointKind::On),
                point(0., 1000., PointKind::Off),
                point(1000., 1000., PointKind::Off),
                point(1000., 0., PointKind::On),
            ],
        }],
    };
    let o = vector::Outline::new(&g);
    assert!((o.bounds.unwrap().y1 - 750.).abs() < 1e-9);
    assert!((o.area().unwrap() - 600000.).abs() < 1e-6);
}
#[test]
fn missing_glyph_nulls_and_no_outline_leak() {
    let dir = Temp::new();
    let p = dir.font("empty.json", &Font::new("Empty", 1000).unwrap());
    let r = analyze_fonts(&[p], &Options::default()).unwrap();
    let m = &r.fonts[0].metrics["stems.lowercase"];
    assert!(m.value.is_none());
    assert!(m.reason.is_some());
    assert_eq!(m.glyphs, vec!["n"]);
    let text = serde_json::to_string(&r).unwrap();
    assert!(!text.contains("\"contours\""));
    assert!(!text.contains("\"kind\""));
}
#[test]
fn recursive_fault_tolerance_cache_and_read_only() {
    let dir = Temp::new();
    let input = dir.0.join("input");
    fs::create_dir_all(&input).unwrap();
    let p = input.join("font.json");
    fixture("geometric sans").save(&p).unwrap();
    let before = fs::read(&p).unwrap();
    fs::write(input.join("corrupt.ttf"), b"bad").unwrap();
    fs::write(input.join("bad.woff2"), b"wOF2").unwrap();
    fs::write(input.join("bad.eot"), b"bad").unwrap();
    let options = Options {
        cache_dir: Some(dir.0.join("cache")),
        output: Some(dir.0.join("out")),
        ..Options::default()
    };
    let paths = [input.to_string_lossy().into_owned()];
    let a = analyze_fonts(&paths, &options).unwrap();
    let b = analyze_fonts(&paths, &options).unwrap();
    assert_eq!(a.fonts.len(), 1);
    assert_eq!(a.errors.len(), 3);
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap()
    );
    assert_eq!(fs::read(&p).unwrap(), before);
    assert!(
        a.errors
            .iter()
            .any(|v| v["error"].as_str().unwrap().contains("woff2"))
    );
    assert_eq!(fs::read_dir(dir.0.join("cache")).unwrap().count(), 1);
    let cache_file = fs::read_dir(dir.0.join("cache"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(cache_file, "broken cache").unwrap();
    let mut uncached = options;
    uncached.output = None;
    let c = analyze_fonts(&paths, &uncached).unwrap();
    assert_eq!(c.fonts.len(), 1);
    assert_eq!(c.errors.len(), 3);
}
#[test]
fn variable_default_named_custom_and_matching() {
    let dir = Temp::new();
    let p = dir.0.join("variable.ttf");
    fs::write(
        &p,
        include_bytes!("../../tests/fixtures/OriginalVariable.ttf"),
    )
    .unwrap();
    let paths = [p.to_string_lossy().into_owned()];
    let a = analyze_fonts(&paths, &Options::default()).unwrap();
    assert!(a.errors.is_empty(), "{:?}", a.errors);
    assert_eq!(a.fonts.len(), 3);
    let vals: Vec<_> = a
        .fonts
        .iter()
        .map(|s| s.metrics["weight.stem_x_height"].value.unwrap())
        .collect();
    assert!(vals.iter().any(|v| (*v - 0.1).abs() < 1e-6));
    assert!(vals.iter().any(|v| (*v - 0.3).abs() < 1e-6));
    assert!(
        a.fonts[0].metadata["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "kern")
    );
    assert!(a.fonts[0].metadata["kerning"]["pairs"].as_u64().unwrap() >= 1);
    assert!(a.fonts[0].metadata["kerning"]["classes"].as_u64().unwrap() >= 2);
    let o = Options {
        axes: BTreeMap::from([("wght".into(), 500.)]),
        ..Options::default()
    };
    let custom = analyze_fonts(&paths, &o).unwrap();
    assert_eq!(custom.fonts.len(), 1);
    assert!(
        (custom.fonts[0].metrics["weight.stem_x_height"]
            .value
            .unwrap()
            - 0.2)
            .abs()
            < 0.002
    );
    let matched = analyze_fonts(
        &paths,
        &Options {
            weight_match: Some(0.29),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(matched.fonts.len(), 1);
    assert!(
        (matched.fonts[0].metrics["weight.stem_x_height"]
            .value
            .unwrap()
            - 0.3)
            .abs()
            < 1e-6
    );
    let bad = analyze_fonts(
        &paths,
        &Options {
            axes: BTreeMap::from([("wght".into(), 1000.)]),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(bad.errors.len(), 1);
}
#[test]
fn subset_csv_markdown_and_output_guard() {
    let dir = Temp::new();
    let f = fixture("geometric sans");
    let p = dir.font("font.json", &f);
    let o = Options {
        glyphs: vec!["n".into()],
        ..Options::default()
    };
    let r = analyze_fonts(std::slice::from_ref(&p), &o).unwrap();
    assert_eq!(r.fonts[0].glyphs.len(), 1);
    assert!(r.fonts[0].metrics["widths.o.advance"].value.is_none());
    assert!(
        render(&r, OutputFormat::Csv)
            .unwrap()
            .contains("glyph.n.advance")
    );
    assert!(
        render(&r, OutputFormat::Md)
            .unwrap()
            .contains("Collection ranges")
    );
    assert!(
        analyze_fonts(
            std::slice::from_ref(&p),
            &Options {
                output: Some(PathBuf::from(&p)),
                ..o
            }
        )
        .is_err()
    );
}
#[test]
fn class_comparison_and_nearest_are_deterministic() {
    let dir = Temp::new();
    let paths: Vec<_> = ["geometric sans", "humanist sans", "serif text", "mono"]
        .iter()
        .map(|c| dir.font(&format!("{c}.json"), &fixture(c)))
        .collect();
    let r = analyze_fonts(&paths, &Options::default()).unwrap();
    assert_eq!(r.collection["classes"].as_object().unwrap().len(), 4);
    assert!(!r.fonts[0].nearest.is_empty());
    let a = compare_to_class(
        &paths[0],
        "humanist sans",
        &paths[1..2],
        &Options::default(),
    )
    .unwrap();
    let metrics = a["fonts"][0]["metrics"].as_array().unwrap();
    assert!(metrics.iter().any(|v| v["in_range"] == false));
    assert!(metrics.iter().any(|v| v["in_range"] == true));
    assert!(compare_to_class(&paths[0], "absent", &paths, &Options::default()).is_err());
}
#[test]
fn overlapping_rectangles_are_flagged() {
    let c = polygon(&[(0., 0.), (100., 0.), (100., 100.), (0., 100.)]);
    let mut d = c.clone();
    for p in &mut d.points {
        p.x += 50.;
        p.y += 50.;
    }
    let g = Glyph {
        strokes: Vec::new(),
        name: "overlap".into(),
        unicode: None,
        advance: 200.,
        contours: vec![c, d],
    };
    assert!(advanced::overlap_count(&g) > 0);
}
#[test]
fn upm_invariant_collection_features() {
    let dir = Temp::new();
    let a = fixture("geometric sans");
    let mut b = a.clone();
    b.name = "Scaled".into();
    b.style.family = "Scaled".into();
    b.upm *= 2;
    b.metrics.x_height *= 2.;
    b.metrics.cap_height *= 2.;
    b.metrics.ascender *= 2.;
    b.metrics.descender *= 2.;
    for g in &mut b.glyphs {
        g.advance *= 2.;
        for c in &mut g.contours {
            for p in &mut c.points {
                p.x *= 2.;
                p.y *= 2.;
            }
        }
    }
    let paths = [dir.font("a.json", &a), dir.font("b.json", &b)];
    let r = analyze_fonts(&paths, &Options::default()).unwrap();
    let range = &r.collection["classes"]["geometric sans"]["metrics"]["stems.lowercase"];
    assert!((range["max"].as_f64().unwrap() - range["min"].as_f64().unwrap()).abs() < 1e-9);
    assert!(r.fonts[0].nearest[0]["distance"].as_f64().unwrap() < 1e-9);
}
#[test]
fn collection_all_faces_woff1_and_ufo() {
    let dir = Temp::new();
    let ttc = dir.0.join("collection.otc");
    fs::write(
        &ttc,
        include_bytes!("../../tests/fixtures/OriginalCollection.ttc"),
    )
    .unwrap();
    let report = analyze_fonts(&[ttc.to_string_lossy().into_owned()], &Options::default()).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.fonts.len(), 6);
    assert!(report.fonts.iter().any(|s| s.source.face == 1));
    let woff = dir.0.join("font.woff");
    fs::write(
        &woff,
        include_bytes!("../../tests/fixtures/OriginalVariable.woff"),
    )
    .unwrap();
    let report =
        analyze_fonts(&[woff.to_string_lossy().into_owned()], &Options::default()).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.fonts.len(), 3);
    let ufo = dir.0.join("font.ufo");
    fixture("geometric sans").save(&ufo).unwrap();
    let before = loader::analyze_path(&ufo, &Options::default()).unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].classification.class, "geometric sans");
    let o = Options {
        output: Some(ufo.join("reports")),
        ..Options::default()
    };
    assert!(analyze_fonts(&[ufo.to_string_lossy().into_owned()], &o).is_err());
}
#[test]
fn ink_area_uses_union_for_overlaps() {
    let c = polygon(&[(0., 0.), (100., 0.), (100., 100.), (0., 100.)]);
    let mut d = c.clone();
    for p in &mut d.points {
        p.x += 50.;
    }
    let g = Glyph {
        strokes: Vec::new(),
        name: "overlap".into(),
        unicode: None,
        advance: 200.,
        contours: vec![c, d],
    };
    assert!((advanced::ink_area(&g).unwrap() - 15000.).abs() < 0.1);
}
#[test]
fn alias_cmap_and_unicode_blocks_are_counted() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/OriginalVariable.ttf");
    let r = analyze_fonts(&[p.to_string_lossy().into_owned()], &Options::default()).unwrap();
    assert_eq!(
        r.fonts[0].metadata["coverage"]["blocks"]["Basic Latin"]["encoded"],
        63
    );
}
#[test]
fn duplicate_sources_with_parallel_cache_keep_both() {
    let dir = Temp::new();
    let f = fixture("geometric sans");
    let paths = [dir.font("a.json", &f), dir.font("b.json", &f)];
    let o = Options {
        cache_dir: Some(dir.0.join("cache")),
        ..Options::default()
    };
    let r = analyze_fonts(&paths, &o).unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.fonts.len(), 2);
    assert_ne!(r.fonts[0].source.path, r.fonts[1].source.path);
    let again = analyze_fonts(&paths, &o).unwrap();
    assert_eq!(
        serde_json::to_value(r).unwrap(),
        serde_json::to_value(again).unwrap()
    );
}
#[test]
fn report_folder_inside_scan_does_not_become_a_font() {
    let dir = Temp::new();
    let p = dir.font("font.json", &fixture("geometric sans"));
    let o = Options {
        output: Some(dir.0.join("reports")),
        cache_dir: Some(dir.0.join("cache")),
        ..Options::default()
    };
    let paths = [dir.0.to_string_lossy().into_owned()];
    let a = analyze_fonts(&paths, &o).unwrap();
    let b = analyze_fonts(&paths, &o).unwrap();
    assert!(a.errors.is_empty(), "{:?} {p}", a.errors);
    assert_eq!(
        serde_json::to_value(a).unwrap(),
        serde_json::to_value(b).unwrap()
    );
}
#[test]
fn outlier_cannot_define_its_own_range() {
    let dir = Temp::new();
    let mut paths = Vec::new();
    for i in 0..4 {
        let mut f = fixture("geometric sans");
        f.name = format!("Face {i}");
        f.style.family = f.name.clone();
        if i == 3 {
            f.glyph_mut("n").unwrap().advance += 500.;
        }
        paths.push(dir.font(&format!("{i}.json"), &f));
    }
    let r = analyze_fonts(&paths, &Options::default()).unwrap();
    let outlier = &r.fonts[3];
    assert!(
        outlier
            .outliers
            .iter()
            .any(|m| m["metric"] == "widths.n.advance")
    );
}
#[test]
fn raster_and_pdf_sheets_are_idempotent() {
    let dir = Temp::new();
    let p = dir.font("font.json", &fixture("geometric sans"));
    let o = Options {
        output: Some(dir.0.join("sheets")),
        sheets: vec![SheetFormat::Png, SheetFormat::Pdf],
        ..Options::default()
    };
    let a = analyze_fonts(std::slice::from_ref(&p), &o).unwrap();
    let b = analyze_fonts(&[p], &o).unwrap();
    assert_eq!(
        serde_json::to_value(a).unwrap(),
        serde_json::to_value(b).unwrap()
    );
    let files: Vec<_> = fs::read_dir(dir.0.join("sheets"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let pdf = fs::read(
        files
            .iter()
            .find(|p| p.extension().is_some_and(|s| s == "pdf"))
            .unwrap(),
    )
    .unwrap();
    assert!(pdf.starts_with(b"%PDF-1.4"));
    assert!(!pdf.windows(8).any(|s| s == b"/FontFil"));
    let png = fs::read(
        files
            .iter()
            .find(|p| p.extension().is_some_and(|s| s == "png"))
            .unwrap(),
    )
    .unwrap();
    assert!(png.starts_with(b"\x89PNG"));
}
#[test]
fn atlas_profile_defers_complex_area_without_fake_zero() {
    let dir = Temp::new();
    let mut font = fixture("geometric sans");
    let contour = font.glyph("n").unwrap().contours[0].clone();
    for _ in 0..25 {
        font.glyph_mut("n").unwrap().contours.push(contour.clone());
    }
    let path = dir.font("complex.json", &font);
    let report = analyze_fonts(
        &[path],
        &Options {
            profile: MeasurementProfile::Atlas,
            glyphs: vec!["n".into()],
            ..Options::default()
        },
    )
    .unwrap();
    let s = &report.fonts[0];
    assert_eq!(s.metadata["measurement_profile"], "atlas");
    assert!(s.glyphs["n"]["ink_area"].value.is_none());
    assert!(
        s.glyphs["n"]["ink_area"]
            .reason
            .as_ref()
            .unwrap()
            .contains("budget")
    );
    assert!(s.glyphs["n"]["quality.overlaps"].value.is_none());
    assert!(s.metrics["spacing.texture_greyness"].value.is_none());
    assert!(s.metrics["stems.lowercase"].value.is_some());
}
#[test]
fn selected_binary_outline_import_preserves_reference_measurements() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/OriginalVariable.ttf");
    let paths = [p.to_string_lossy().into_owned()];
    let full = analyze_fonts(&paths, &Options::default()).unwrap();
    let selected = analyze_fonts(
        &paths,
        &Options {
            glyphs: vec!["n".into(), "H".into()],
            ..Options::default()
        },
    )
    .unwrap();
    for (a, b) in full.fonts.iter().zip(&selected.fonts) {
        for key in [
            "proportions.x_cap",
            "weight.stem_x_height",
            "stems.uppercase",
        ] {
            assert_eq!(a.metrics[key].value, b.metrics[key].value);
        }
        assert_eq!(a.metadata, b.metadata);
    }
}

#[test]
fn greyness_is_independent_of_declared_vertical_metrics() {
    let t = Temp::new();
    let a = fixture("geometric sans");
    let mut b = a.clone();
    b.metrics.ascender = 550.;
    b.metrics.descender = -50.;
    let paths = [t.font("a.json", &a), t.font("b.json", &b)];
    let r = analyze_fonts(&paths, &Options::default()).unwrap();
    assert_eq!(r.fonts.len(), 2);
    assert_eq!(
        r.fonts[0].metrics["spacing.texture_greyness"].value,
        r.fonts[1].metrics["spacing.texture_greyness"].value
    );
    assert!(
        r.fonts[0].metrics["spacing.texture_greyness"]
            .value
            .is_some()
    );
}
#[test]
fn normal_thickness_is_constant_around_a_monoline_circle() {
    let f = fixture("geometric sans");
    let o = vector::Outline::new(f.glyph("o").unwrap());
    for seg in o.paths[0]
        .segments()
        .filter(|s| s.start().distance(s.end()) > 1e-8)
    {
        for t in [0.1, 0.3, 0.5, 0.7, 0.9] {
            let (a, b) = o.normal_span(seg, t).unwrap();
            assert!(
                (a.distance(b) - 50.).abs() < 0.05,
                "normal thickness {}",
                a.distance(b)
            );
        }
    }
    let d = Temp::new();
    let p = d.font("mono.json", &f);
    let r = analyze_fonts(&[p], &Options::default()).unwrap();
    assert!(r.fonts[0].metrics["stems.stress_angle"].value.is_none());
}
#[test]
fn nearest_uses_arch_evidence_outside_the_old_seven_features() {
    let t = Temp::new();
    let paths = [
        t.font("a.json", &fixture("geometric sans")),
        t.font("b.json", &fixture("humanist sans")),
    ];
    let mut r = analyze_fonts(&paths, &Options::default()).unwrap();
    let a = r.fonts.remove(0);
    let mut b = a.clone();
    b.metadata["family"] = json!("B");
    b.source.sha256 = "b".into();
    let mut c = b.clone();
    c.metadata["family"] = json!("C");
    c.source.sha256 = "c".into();
    // These fonts agree on all seven original features; their arch profiles distinguish them.
    for key in [
        "arch.n.thickness_10",
        "arch.n.thickness_20",
        "arch.n.thickness_50",
        "arch.n.thickness_90",
    ] {
        if let Some(m) = c.metrics.get_mut(key) {
            m.value = m.value.map(|v| v * 2.);
            m.x_height_ratio = m.x_height_ratio.map(|v| v * 2.);
        }
    }
    let near = collection::nearest(&a, &[c, b]);
    assert_eq!(near[0]["family"], "B");
    assert_eq!(near[0]["dimensions"], 33);
    assert!(near[1]["distance"].as_f64().unwrap() > 0.);
}

#[test]
fn aperture_and_terminal_evidence_separates_geometric_and_humanist_sans() {
    let t = Temp::new();
    let font = fixture("geometric sans");
    let path = t.font("base.json", &font);
    let r = analyze_fonts(&[path], &Options::default()).unwrap();
    let study = &r.fonts[0];
    // Measured feature combinations, without supplying a reference font's name.
    for (round, contrast, c, e, cut, stress, expected) in [
        (
            1.0602,
            1.1709,
            0.6124,
            0.1281,
            Some(90.),
            Some(3.97),
            "geometric sans",
        ),
        (
            1.0348,
            1.1033,
            0.5826,
            0.1374,
            Some(90.),
            None,
            "geometric sans",
        ),
        (
            0.9844,
            1.2271,
            0.6995,
            0.2671,
            Some(90.),
            Some(6.38),
            "humanist sans",
        ),
        (
            0.9230,
            1.2006,
            0.6206,
            0.2870,
            Some(90.),
            Some(4.38),
            "humanist sans",
        ),
        (
            0.9464,
            1.2565,
            0.5893,
            0.2825,
            None,
            Some(5.33),
            "humanist sans",
        ),
        (
            1.0060,
            1.0469,
            0.3853,
            0.1699,
            Some(-27.76),
            None,
            "geometric sans",
        ),
    ] {
        let mut metrics = study.metrics.clone();
        for (key, value) in [
            ("widths.o_x", Some(round)),
            ("stems.contrast", Some(contrast)),
            ("aperture.c.opening_ratio", Some(c)),
            ("aperture.e.opening_ratio", Some(e)),
            ("terminals.c.end_cut_angle", cut),
            ("stems.stress_angle", stress),
        ] {
            metrics.get_mut(key).unwrap().value = value;
        }
        let class = advanced::refine_classification(
            study.classification.clone(),
            &font,
            &metrics,
            &json!({}),
        );
        assert_eq!(class.class, expected, "{:?}", class);
        assert_ne!(class.confidence, 0.66);
    }
}
