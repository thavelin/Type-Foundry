use foundry_api::Session;
use serde_json::json;

#[test]
fn save_reports_conversion_and_validates_tolerance_before_writing() {
    let dir = std::env::temp_dir().join(format!(
        "typefoundry-conversion-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = dir.join("curve.ttf");
    let mut session = Session::new();
    for command in [
        json!({"op":"create","name":"Conversion"}),
        json!({"op":"put_glyph","glyph":{"name":"a","unicode":97,"advance":500,"contours":[{"closed":true,"points":[
            {"x":0,"y":0,"kind":"on","smooth":false}, {"x":0,"y":200,"kind":"off","smooth":false},
            {"x":200,"y":200,"kind":"off","smooth":false}, {"x":200,"y":0,"kind":"on","smooth":false}
        ]}]}}),
    ] {
        let result = session.execute_line(&command.to_string()).unwrap();
        assert!(result.ok, "{result:?}");
    }
    let before = session.font().unwrap().clone();
    let invalid = json!({"op":"save","path":path,"ttf_tolerance":0});
    assert!(!session.execute_line(&invalid.to_string()).unwrap().ok);
    assert!(!path.exists());
    let command = json!({"op":"save","path":path,"ttf_tolerance":0.5});
    let saved = session.execute_line(&command.to_string()).unwrap();
    assert!(saved.ok, "{saved:?}");
    let report = &saved.data.unwrap()["ttf"];
    assert_eq!(report["split_segments"], 1);
    assert_eq!(report["tolerance"], 0.5);
    assert!(report["max_error"].as_f64().unwrap() <= 0.5);
    assert_eq!(session.font(), Some(&before));
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], &[0, 1, 0, 0]);
    let overwrite = json!({"op":"save","path":path,"ttf_tolerance":0.0,"force":true});
    assert!(!session.execute_line(&overwrite.to_string()).unwrap().ok);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!dir.join("curve.ttf.bak").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stroke_weight_family_exports_blend_compatible_ttfs() {
    let dir = std::env::temp_dir().join(format!(
        "typefoundry-stroke-export-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut masters = Vec::new();
    for width in [80.0, 120.0, 180.0] {
        let mut session = Session::new();
        for command in [
            json!({"op":"create","name":"Stroke Export"}),
            json!({"op":"put_glyph","glyph":{"name":"a","unicode":97,"advance":600,"contours":[]}}),
            json!({"op":"stroke_add","name":"a","stroke":{
                "centerline":{"closed":false,"points":[{"x":0,"y":0,"kind":"on","smooth":false},{"x":500,"y":0,"kind":"on","smooth":false}]},
                "ease":"smooth","cap":"round",
                "widths":[{"t":0,"left":width*0.2,"right":width*0.3},{"t":0.4,"left":width*0.6,"right":width*0.4},{"t":1,"left":width*0.3,"right":width*0.2}]
            }}),
            json!({"op":"stroke_expand","fixed_structure":true}),
        ] {
            let result = session.execute_line(&command.to_string()).unwrap();
            assert!(result.ok, "{result:?}");
        }
        let font = session
            .font()
            .unwrap()
            .derive_style(&format!("Weight{width}"), Some(width as u16 * 4), None, 0.0)
            .unwrap();
        masters.push(font);
    }
    let refs: Vec<_> = masters.iter().collect();
    let paths =
        foundry_core::export_family(&refs, &dir, foundry_core::ExportFormat::Ttf, false).unwrap();
    let loaded: Vec<_> = paths
        .iter()
        .map(|p| foundry_core::Font::load(p).unwrap())
        .collect();
    for pair in loaded.windows(2) {
        let issues = foundry_core::compatibility(&pair[0], &pair[1]);
        assert!(issues.is_empty(), "{issues:?}");
        assert!(foundry_core::blend_fonts(&pair[0], &pair[1], 0.5).is_ok());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
