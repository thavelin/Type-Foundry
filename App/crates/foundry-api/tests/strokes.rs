use foundry_api::Session;
use serde_json::{Value, json};
fn run(s: &mut Session, v: Value) -> foundry_api::Response {
    s.execute_line(&v.to_string()).unwrap()
}
fn seed() -> Session {
    let mut s = Session::new();
    assert!(run(&mut s, json!({"op":"create","name":"Stroke Test"})).ok);
    assert!(
        run(
            &mut s,
            json!({"op":"put_glyph","glyph":{"name":"test","advance":600,"contours":[]}})
        )
        .ok
    );
    s
}
fn stroke() -> Value {
    json!({"centerline":{"closed":false,"points":[{"x":0,"y":0,"kind":"on","smooth":false},{"x":500,"y":0,"kind":"on","smooth":false}]},"widths":[{"t":0,"left":40,"right":40},{"t":1,"left":40,"right":40}]})
}
#[test]
fn edits_preview_undo_and_errors() {
    let mut s = seed();
    assert!(
        run(
            &mut s,
            json!({"op":"stroke_add","name":"test","stroke":stroke()})
        )
        .ok
    );
    let before = s.font().unwrap().clone();
    let r = run(&mut s, json!({"op":"stroke_preview"}));
    assert!(r.ok, "{r:?}");
    assert_eq!(r.data.unwrap()["points"], 4);
    assert_eq!(s.font(), Some(&before));
    assert!(run(&mut s,json!({"op":"stroke_width","name":"test","index":0,"points":[{"t":0.5,"left":60,"right":60}]})).ok);
    assert!(run(&mut s,json!({"op":"stroke_width","name":"test","index":0,"from_t":0.5,"points":[{"t":0.7,"left":60,"right":60}]})).ok);
    assert!(
        run(
            &mut s,
            json!({"op":"stroke_set","name":"test","index":0,"stroke":stroke()})
        )
        .ok
    );
    assert!(run(&mut s, json!({"op":"stroke_expand","fixed_structure":true})).ok);
    assert!(s.font().unwrap().glyphs[0].strokes.is_empty());
    assert!(run(&mut s, json!({"op":"undo"})).ok);
    assert_eq!(s.font().unwrap().glyphs[0].strokes.len(), 1);
    assert!(run(&mut s, json!({"op":"redo"})).ok);
    assert!(run(&mut s, json!({"op":"undo"})).ok);
    assert!(
        run(
            &mut s,
            json!({"op":"stroke_remove","name":"test","index":0})
        )
        .ok
    );
    let r = run(
        &mut s,
        json!({"op":"stroke_remove","name":"test","index":0}),
    );
    assert!(!r.ok);
    assert_eq!(r.data.unwrap()["contours"], 0);
    let r = run(&mut s, json!({"op":"stroke_set"}));
    assert!(!r.ok);
    assert_eq!(r.data.unwrap()["points"], 0);
}
#[test]
fn save_open_overwrite_protection() {
    let mut s = seed();
    assert!(
        run(
            &mut s,
            json!({"op":"stroke_add","name":"test","stroke":stroke()})
        )
        .ok
    );
    let dir = std::env::temp_dir().join(format!(
        "typefoundry-stroke-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("font.json");
    assert!(run(&mut s, json!({"op":"save","path":path})).ok);
    let bytes = std::fs::read(&path).unwrap();
    assert!(!run(&mut s, json!({"op":"save","path":path})).ok);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(run(&mut s, json!({"op":"save","path":path,"force":true})).ok);
    assert!(dir.join("font.json.bak").exists());
    assert!(run(&mut s, json!({"op":"open","path":path})).ok);
    assert_eq!(s.font().unwrap().glyphs[0].strokes.len(), 1);
    assert!(!run(&mut s, json!({"op":"save","path":dir.join("font.ttf")})).ok);
    assert!(!dir.join("font.ttf").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn multi_glyph_expansion_atomic() {
    let mut s = seed();
    assert!(
        run(
            &mut s,
            json!({"op":"stroke_add","name":"test","stroke":stroke()})
        )
        .ok
    );
    let before = s.font().unwrap().clone();
    assert!(
        !run(
            &mut s,
            json!({"op":"stroke_expand","names":["test","missing"]})
        )
        .ok
    );
    assert_eq!(s.font(), Some(&before));
}

#[test]
fn variable_fixed_preview_expand_and_master_compatibility() {
    let mut masters = vec![];
    let mut keys = vec![];
    let mut counts = vec![];
    for width in [80.0, 120.0, 180.0] {
        let mut s = seed();
        let mut st = stroke();
        st["ease"] = json!("smooth");
        st["cap"] = json!("round");
        st["widths"] = json!([{"t":0,"left":width*0.2,"right":width*0.3},{"t":0.4,"left":width*0.6,"right":width*0.4},{"t":1,"left":width*0.3,"right":width*0.2}]);
        assert!(run(&mut s, json!({"op":"stroke_add","name":"test","stroke":st})).ok);
        let before = s.font().unwrap().clone();
        let preview = run(
            &mut s,
            json!({"op":"stroke_preview","fixed_structure":true}),
        );
        assert!(preview.ok, "{preview:?}");
        assert_eq!(s.font(), Some(&before));
        let expanded = run(&mut s, json!({"op":"stroke_expand","fixed_structure":true}));
        assert!(expanded.ok, "{expanded:?}");
        let data = expanded.data.unwrap();
        assert_eq!(data["compatible"], true);
        assert_eq!(data["glyphs"], preview.data.unwrap()["glyphs"]);
        assert_eq!(data["glyphs"][0]["structure_key"]["version"], 3);
        keys.push(data["glyphs"][0]["structure_key"].clone());
        counts.push(data["points"].clone());
        masters.push(s.font().unwrap().clone());
    }
    assert!(keys.windows(2).all(|w| w[0] == w[1]));
    assert!(counts.windows(2).all(|w| w[0] == w[1]));
    for pair in masters.windows(2) {
        assert!(foundry_core::compatibility(&pair[0], &pair[1]).is_empty());
        assert!(foundry_core::blend_fonts(&pair[0], &pair[1], 0.5).is_ok());
    }
}
