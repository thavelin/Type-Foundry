//! Production Vostok acceptance tests. Ignored unless `TF_VOSTOK_DIR` points at v4.6/v4.7 masters.

#[cfg(test)]
mod vostok {
    use std::path::PathBuf;

    use crate::{Font, OffsetOptions, check_smoothness, measure_stems_ray, offset_font_detailed};

    fn vostok_dir() -> Option<PathBuf> {
        std::env::var_os("TF_VOSTOK_DIR").map(PathBuf::from)
    }

    #[test]
    #[ignore = "requires TF_VOSTOK_DIR with v4.6/v4.7 JSON masters"]
    fn offset_o1_h_stem_grows() {
        let Some(dir) = vostok_dir() else {
            panic!("TF_VOSTOK_DIR unset");
        };
        let path = dir.join("v4.6").join("Regular.json");
        assert!(path.is_file(), "missing {path:?}; set TF_VOSTOK_DIR");
        let mut font = Font::load(&path).expect("load Regular");
        let before = measure_stems_ray(font.glyph("H").expect("H"), font.metrics.cap_height);
        let report = offset_font_detailed(
            &mut font,
            &OffsetOptions {
                horizontal: 23.0,
                vertical: 5.5,
                names: Some(vec!["H".into()]),
                ..OffsetOptions::default()
            },
        )
        .expect("offset");
        assert!(report.fallback.is_empty(), "{:?}", report.fallback);
        let after = measure_stems_ray(font.glyph("H").expect("H"), font.metrics.cap_height);
        let b = before.first().copied().unwrap_or(0.0);
        let a = after.first().copied().unwrap_or(0.0);
        assert!((b - 152.0).abs() < 2.0, "before stem {b}");
        assert!((a - 198.0).abs() < 2.0, "after stem {a}");
    }

    #[test]
    #[ignore = "requires TF_VOSTOK_DIR with v4.6/v4.7 JSON masters"]
    fn smoothness_s1_regular_self() {
        let Some(dir) = vostok_dir() else {
            panic!("TF_VOSTOK_DIR unset");
        };
        let path = dir.join("v4.6").join("Regular.json");
        let font = Font::load(&path).expect("load");
        let report =
            check_smoothness(&font, None, None, 1.5, 6.0, 8.0, 2.5, false, 10).expect("check");
        assert_eq!(report.totals.breaks, 161);
        assert!((report.totals.largest - 8.8).abs() < 0.3);
        assert!((report.totals.total - 429.0).abs() < 5.0);
    }
}
