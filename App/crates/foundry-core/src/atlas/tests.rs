use super::*;
use crate::analyze::{Classification, Metric};
use std::sync::atomic::{AtomicU64, Ordering};
static SEQ: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "foundry-atlas-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn report(&self, name: &str, fonts: &[Study]) -> String {
        let p = self.0.join(name);
        fs::write(&p,serde_json::to_vec(&json!({"format":"typefoundry.collection-study","version":2,"fonts":fonts,"errors":[]})).unwrap()).unwrap();
        p.to_string_lossy().into_owned()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn study(name: &str, index: usize, weight: u16) -> Study {
    let mut metrics = BTreeMap::new();
    for (i, (key, _, _, _, transform)) in embedding::SPECS.iter().enumerate() {
        let value =
            0.2 + (index % 4) as f64 * 0.15 + (index / 4) as f64 * 0.1 + (i % 7) as f64 * 0.03;
        let units = if *transform == "length" {
            "font_units"
        } else {
            "ratio"
        };
        metrics.insert(
            (*key).into(),
            Metric {
                value: Some(if *transform == "log" {
                    1. + value
                } else {
                    value
                }),
                units: units.into(),
                x_height_ratio: if *transform == "length" {
                    Some(value)
                } else {
                    None
                },
                cap_height_ratio: None,
                method: "synthetic unit-test metric".into(),
                glyphs: vec!["n".into()],
                reason: None,
            },
        );
    }
    Study {
        format: "typefoundry.study".into(),
        version: 2,
        algorithm: crate::analyze::ALGORITHM.into(),
        source: Source {
            path: format!("/synthetic/{index}-{weight}.ttf"),
            sha256: hash(format!("{index}:{name}:{weight}").as_bytes()),
            face: 0,
            instance: "default".into(),
            axes: BTreeMap::new(),
        },
        metadata: json!({"family":name,"style":if weight==400{"Regular"}else{"Bold"},"weight_class":weight,"width_class":5,"italic_angle":0,"license":"Original synthetic test evidence"}),
        metrics,
        glyphs: BTreeMap::new(),
        classification: Classification {
            class: "geometric sans".into(),
            confidence: 0.66,
            reasons: vec!["synthetic fixture".into(); 3],
            serif_type: Some("none".into()),
            method: "test".into(),
        },
        nearest: vec![],
        outliers: vec![],
        warnings: vec![],
        terminals: BTreeMap::new(),
    }
}
#[test]
fn build_deduplicates_faces_and_preserves_source_aliases() {
    let t = Temp::new();
    let a = study("Alpha", 0, 400);
    let mut b = a.clone();
    b.source.path = "/other/Alpha.ttf".into();
    let p = t.report("input.json", &[a, b]);
    let atlas = build(&[p], &Options::default()).unwrap();
    assert_eq!(atlas.faces.len(), 1);
    assert_eq!(atlas.faces[0].sources.len(), 2);
    assert_eq!(atlas.families.len(), 1);
    assert_eq!(atlas.dimensions.len(), 33);
}
#[test]
fn more_weights_do_not_shift_family_embedding_fit() {
    let t = Temp::new();
    let mut sources = vec![
        study("Alpha", 0, 400),
        study("Beta", 4, 400),
        study("Gamma", 8, 400),
    ];
    let p = t.report("a.json", &sources);
    let a = build(&[p], &Options::default()).unwrap();
    for i in 0..10 {
        sources.push(study("Alpha", 99 + i, 500 + i as u16 * 30));
    }
    let p = t.report("b.json", &sources);
    let b = build(&[p], &Options::default()).unwrap();
    assert_eq!(
        a.dimensions.iter().map(|d| d.center).collect::<Vec<_>>(),
        b.dimensions.iter().map(|d| d.center).collect::<Vec<_>>()
    );
}
#[test]
fn missing_is_masked_not_zero_and_not_a_perfect_neighbor() {
    let t = Temp::new();
    let mut a = study("Alpha", 0, 400);
    let b = study("Beta", 4, 400);
    a.metrics.clear();
    let p = t.report("a.json", &[a, b]);
    let atlas = build(&[p], &Options::default()).unwrap();
    let a = atlas.families.iter().find(|f| f.name == "Alpha").unwrap();
    assert!(a.mask.iter().all(|v| !*v));
    assert!(a.neighbors.is_empty());
    assert!(a.vector.iter().all(|v| *v == 0.));
}
#[test]
fn snapshots_deterministic_and_manual_review_has_provenance() {
    let t = Temp::new();
    let sources: Vec<_> = (0..12)
        .map(|i| study(&format!("Family {i}"), i, 400))
        .collect();
    let p = t.report("a.json", &sources);
    let a = build(std::slice::from_ref(&p), &Options::default()).unwrap();
    let mut reverse = sources.clone();
    reverse.reverse();
    let q = t.report("b.json", &reverse);
    let b = build(&[q], &Options::default()).unwrap();
    assert_eq!(
        serde_json::to_value(a).unwrap(),
        serde_json::to_value(b).unwrap()
    );
    let labels = t.0.join("labels.json");
    fs::write(
        &labels,
        r#"{"Family 0":{"class":"humanist sans","reason":"manual curvature review"}}"#,
    )
    .unwrap();
    let c = build(
        &[p],
        &Options {
            labels: Some(labels),
            ..Options::default()
        },
    )
    .unwrap();
    let f = c.families.iter().find(|f| f.name == "Family 0").unwrap();
    assert_eq!(f.class, "humanist sans");
    assert!(f.label_source.contains("manual curvature review"));
}
#[test]
fn no_outline_keys_and_html_escapes_source_names() {
    let t = Temp::new();
    let p = t.report(
        "a.json",
        &[study("</script><img src=x onerror=alert(1)>", 1, 400)],
    );
    let a = build(&[p], &Options::default()).unwrap();
    let html = viewer::render(&a).unwrap();
    assert!(!html.contains("</script><img src=x"));
    let text = serde_json::to_string(&a).unwrap();
    assert!(!text.contains("\"contours\""));
    assert!(!text.contains("\"points\""));
    assert!(!html.contains("src=\"https://"));
    assert!(html.contains("\\u003c/script\\u003e"));
}
#[test]
fn persist_query_and_briefs_roundtrip() {
    let t = Temp::new();
    let p = t.report("a.json", &[study("Alpha", 1, 400), study("Beta", 5, 400)]);
    let out = t.0.join("atlas");
    let options = Options {
        output: Some(out.clone()),
        ..Options::default()
    };
    let a = build(std::slice::from_ref(&p), &options).unwrap();
    build(&[p], &options).unwrap();
    let latest: Value =
        serde_json::from_slice(&fs::read(out.join("latest.json")).unwrap()).unwrap();
    let snapshot = out.join(latest["snapshot"].as_str().unwrap());
    let q = query(&snapshot, "Alpha", 5, 12).unwrap();
    assert_eq!(q["family"]["name"], "Alpha");
    assert_eq!(q["nearest"][0]["family"], "Beta");
    assert!(query(&snapshot, "not present", 5, 12).is_err());
    assert_eq!(
        briefs(&snapshot, None, 5).unwrap()["briefs"]
            .as_array()
            .unwrap()
            .len(),
        a.gaps.len().min(5)
    );
}
#[test]
fn pca_degeneracy_is_finite_and_low_confidence_is_not_a_cluster() {
    let t = Temp::new();
    let mut ss = Vec::new();
    for i in 0..4 {
        let mut s = study(&format!("Unknown {i}"), 0, 400);
        s.classification.confidence = 0.15;
        ss.push(s);
    }
    let p = t.report("a.json", &ss);
    let a = build(&[p], &Options::default()).unwrap();
    assert!(a.clusters.is_empty());
    assert!(a.gaps.is_empty());
    assert!(
        a.families
            .iter()
            .all(|f| f.class == "unclassified" && f.projection.iter().all(|v| v.is_finite()))
    );
}
#[test]
fn gaps_are_supported_in_full_space_and_have_no_outlines() {
    let t = Temp::new();
    let ss: Vec<_> = (0..8)
        .map(|i| {
            let mut s = study(&format!("Cube {i}"), i, 400);
            for (j, (_, m)) in s.metrics.iter_mut().enumerate() {
                let v = if i & (1 << (j % 3)) == 0 { 0.5 } else { 2.5 };
                m.value = Some(v);
                if m.x_height_ratio.is_some() {
                    m.x_height_ratio = Some(v);
                }
            }
            s
        })
        .collect();
    let p = t.report("a.json", &ss);
    let a = build(
        &[p],
        &Options {
            min_shared: 12,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(!a.gaps.is_empty());
    let mut occupied = a.faces[0].clone();
    let first_gap = a.gaps[0].clone();
    occupied.id = "occupied-gap".into();
    occupied.raw = serde_json::from_value(first_gap["raw"].clone()).unwrap();
    (occupied.vector, occupied.mask) = embedding::encode(&occupied.raw, &a.dimensions);
    let mut all_faces = a.faces.clone();
    all_faces.push(occupied);
    let proposals = gaps::find(&a.families, &a.dimensions, &Options::default(), &all_faces);
    assert!(proposals.iter().all(|g| g["id"] != first_gap["id"]));
    for gap in a.gaps {
        assert!(gap["minimum_distance"].as_f64().unwrap() >= 0.25);
        assert_eq!(gap["support"].as_array().unwrap().len(), 3);
        assert!(gap["targets"].as_array().unwrap().len() >= 12);
        assert!(gap["brief"].as_str().unwrap().contains("original"));
        let raw: Vec<Option<f64>> = serde_json::from_value(gap["raw"].clone()).unwrap();
        if let Some((s, c)) = raw[7].zip(raw[8]) {
            assert!((s.hypot(c) - 1.).abs() < 1e-9);
        }
        if let Some((n, h)) = raw[10].zip(raw[11]) {
            assert!((raw[13].unwrap() - n / h).abs() < 1e-9);
        }
    }
}
#[test]
fn dimension_weights_are_balanced_by_domain() {
    let t = Temp::new();
    let p = t.report("a.json", &[study("Alpha", 1, 400), study("Beta", 5, 400)]);
    let a = build(&[p], &Options::default()).unwrap();
    let d = &a.dimensions;
    let mask = vec![true; d.len()];
    let origin = vec![0.; d.len()];
    let mut offset = origin.clone();
    let groups: BTreeSet<_> = d.iter().map(|d| d.group.clone()).collect();
    for (i, dim) in d.iter().enumerate() {
        if dim.group == "width" {
            offset[i] = 1.;
        }
    }
    let distance = embedding::distance(&origin, &mask, &offset, &mask, d, 12)
        .unwrap()
        .0;
    assert!((distance - (1. / groups.len() as f64).sqrt()).abs() < 1e-12);
}
#[test]
fn pointer_integrity_and_output_ownership_are_checked() {
    let t = Temp::new();
    let p = t.report("a.json", &[study("Alpha", 1, 400)]);
    let out = t.0.join("atlas");
    build(
        std::slice::from_ref(&p),
        &Options {
            output: Some(out.clone()),
            ..Options::default()
        },
    )
    .unwrap();
    assert!(query(&out.join("latest.json"), "Alpha", 5, 12).is_ok());
    let pointer: Value =
        serde_json::from_slice(&fs::read(out.join("latest.json")).unwrap()).unwrap();
    fs::write(out.join(pointer["snapshot"].as_str().unwrap()), "{}").unwrap();
    assert!(
        query(&out.join("latest.json"), "Alpha", 5, 12)
            .unwrap_err()
            .contains("checksum")
    );
    let foreign = t.0.join("foreign");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("explore.html"), "user page").unwrap();
    assert!(
        build(
            &[p],
            &Options {
                output: Some(foreign.clone()),
                ..Options::default()
            }
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(foreign.join("explore.html")).unwrap(),
        "user page"
    );
}
#[test]
fn a_new_font_can_be_measured_against_a_gap_brief() {
    let t = Temp::new();
    let mut ss = Vec::new();
    for i in 0..8 {
        let mut s = study(&format!("Cube {i}"), i, 400);
        for (j, (_, m)) in s.metrics.iter_mut().enumerate() {
            let v = if i & (1 << (j % 3)) == 0 { 0.5 } else { 2.5 };
            m.value = Some(v);
            if m.x_height_ratio.is_some() {
                m.x_height_ratio = Some(v);
            }
        }
        ss.push(s);
    }
    let p = t.report("input.json", &ss);
    let out = t.0.join("atlas");
    let a = build(
        &[p],
        &Options {
            output: Some(out.clone()),
            ..Options::default()
        },
    )
    .unwrap();
    assert!(!a.gaps.is_empty());
    let font =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/OriginalVariable.ttf");
    let before = fs::read(&font).unwrap();
    let result = evaluate_font(
        &font.to_string_lossy(),
        &out.join("latest.json"),
        &a.gaps[0]["id"].as_str().unwrap()[..10],
        &crate::analyze::Options {
            profile: crate::analyze::MeasurementProfile::Atlas,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result["format"], "typefoundry.brief-evaluation");
    assert_eq!(result["fonts"].as_array().unwrap().len(), 3);
    assert!(!result["fonts"][0]["metrics"].as_array().unwrap().is_empty());
    assert_eq!(fs::read(font).unwrap(), before);
}
#[test]
fn reviewed_family_aliases_group_legacy_style_names() {
    let t = Temp::new();
    let p = t.report(
        "a.json",
        &[
            study("Example", 0, 400),
            study("Example Book Italic", 1, 400),
        ],
    );
    let aliases = t.0.join("aliases.json");
    fs::write(&aliases,r#"{"Example Book Italic":{"family":"Example","reason":"shared PostScript root verified in name table"}}"#).unwrap();
    let a = build(
        &[p],
        &Options {
            family_aliases: Some(aliases),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(a.families.len(), 1);
    assert_eq!(a.families[0].name, "Example");
    assert_eq!(a.families[0].faces.len(), 2);
    assert!(
        a.families[0]
            .aliases
            .contains(&"Example Book Italic".into())
    );
    assert!(a.families[0].grouping_notes[0].contains("PostScript"));
    assert_eq!(a.space_id.len(), 64);
}

#[test]
fn revisions_group_versions_without_losing_iterations() {
    let t = Temp::new();
    let a = study("Vostok Serif v3", 1, 400);
    let b = study("Vostok Serif v4.7", 2, 400);
    let mut c = study("Quarra Sans", 3, 400);
    c.source.path = "fonts/round7a/Quarra-Regular.ttf".into();
    let mut d = study("Quarra Sans", 4, 400);
    d.source.path = "fonts/round7b/Quarra-Regular.ttf".into();
    let p = t.report("revisions.json", &[a, b, c, d]);
    let atlas = build(&[p], &Options::default()).unwrap();
    assert_eq!(atlas.families.len(), 2);
    for f in &atlas.families {
        assert_eq!(f.revisions.len(), 2);
        assert_eq!(f.faces.len(), 2);
    }
    let vostok = atlas
        .families
        .iter()
        .find(|f| f.name == "Vostok Serif")
        .unwrap();
    assert!(vostok.revisions.contains_key("v4.7"));
    assert_eq!(family_revision("Font 2000"), ("Font 2000", None));
    assert_eq!(family_revision("Font vNext"), ("Font vNext", None));
}
#[test]
fn class_briefs_search_beyond_global_shortlist() {
    let t = Temp::new();
    let sources: Vec<_> = (0..24)
        .map(|i| study(&format!("Family {i}"), i, 400))
        .collect();
    let p = t.report("gaps.json", &sources);
    let mut a = build(&[p], &Options::default()).unwrap();
    assert!(!a.gaps.is_empty());
    let class = a.gaps[0]["class"].as_str().unwrap().to_string();
    a.gaps.clear();
    let briefs = class_briefs(&a, &class, 5);
    assert!(!briefs.is_empty());
    assert!(
        briefs
            .iter()
            .all(|g| g["class"] == class && g["minimum_distance"].as_f64().unwrap() >= 0.25)
    );
}
