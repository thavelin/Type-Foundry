//! View settings. They change how the window draws, never the font, and persist between launches.

use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::preview::ReviewView;

/// How many files File > Open recent keeps.
pub const RECENT_LIMIT: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub handle_size: f32,
    pub fill: bool,
    pub outline: bool,
    pub metrics: bool,
    pub point_numbers: bool,
    pub coordinates: bool,
    pub snap: bool,
    pub cell_size: f32,
    /// Force the glyph list on in Overview. Editor and Split show it either way.
    /// Renamed from `show_glyph_list` so older persisted `true` defaults do not stick.
    pub glyph_list_in_overview: bool,
    pub show_inspector: bool,
    pub show_preview: bool,
    /// Draw the previous and next glyphs beside the one being edited, with no handles.
    pub onion_skin: bool,
    /// User guides. They are not part of the font.
    pub show_guides: bool,
    /// Black editor paper with white fill. Off keeps the bone paper and black fill.
    pub dark_canvas: bool,
    /// Fonts opened or saved, newest first. File > Open recent reads it.
    #[serde(default)]
    pub recent: Vec<PathBuf>,
    /// Overview and editor share the main area.
    pub split_main: bool,
    /// Fraction of the main area given to the overview when split.
    pub split_ratio: f32,
    pub review_place: ReviewPlace,
    /// The review pane's set, size, and guides.
    pub review: ReviewView,
}

/// Where the review sheet sits. A window can be moved and resized on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPlace {
    #[default]
    Bottom,
    Right,
    Float,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            handle_size: 4.5,
            fill: true,
            outline: false,
            metrics: true,
            point_numbers: false,
            coordinates: true,
            snap: true,
            cell_size: 96.0,
            glyph_list_in_overview: false,
            show_inspector: true,
            show_preview: true,
            onion_skin: false,
            show_guides: true,
            dark_canvas: false,
            split_main: false,
            split_ratio: 0.42,
            review_place: ReviewPlace::Bottom,
            review: ReviewView::default(),
            recent: Vec::new(),
        }
    }
}

impl Settings {
    /// Put `path` first. An entry already in the list moves up instead of repeating.
    pub fn remember_recent(&mut self, path: &Path) {
        self.recent.retain(|known| known != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(RECENT_LIMIT);
    }

    /// The settings window body. Returns true when a change needs thumbnails redrawn.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        let before_cell = self.cell_size;
        ui.heading("Canvas");
        ui.checkbox(&mut self.fill, "Fill outlines");
        ui.checkbox(&mut self.outline, "Stroke outlines");
        ui.checkbox(&mut self.metrics, "Metric lines");
        ui.checkbox(&mut self.point_numbers, "Point numbers");
        ui.checkbox(&mut self.coordinates, "Coordinates of the selected point");
        ui.checkbox(&mut self.snap, "Snap moves to whole units");
        ui.add(egui::Slider::new(&mut self.handle_size, 2.5..=9.0).text("Handle size"));
        ui.add_space(8.0);
        ui.heading("Overview");
        ui.add(egui::Slider::new(&mut self.cell_size, 48.0..=200.0).text("Cell size"));
        ui.add_space(8.0);
        ui.heading("Panels");
        ui.checkbox(&mut self.glyph_list_in_overview, "Glyph tree in Overview")
            .on_hover_text(
                "Force the glyph tree on in Overview too. Editor, Review, and Split show it either way.",
            );
        ui.checkbox(&mut self.show_inspector, "Inspector");
        ui.checkbox(&mut self.show_preview, "Review sheet");
        ui.horizontal(|ui| {
            ui.label("Place");
            ui.selectable_value(&mut self.review_place, ReviewPlace::Bottom, "Bottom");
            ui.selectable_value(&mut self.review_place, ReviewPlace::Right, "Right");
            ui.selectable_value(&mut self.review_place, ReviewPlace::Float, "Window");
        });
        ui.checkbox(&mut self.split_main, "Overview and editor side by side");
        ui.add_space(8.0);
        ui.heading("Editor");
        ui.checkbox(&mut self.onion_skin, "Onion skin")
            .on_hover_text("The previous and next glyphs sit beside this one, outlines only.");
        ui.checkbox(&mut self.show_guides, "Guides").on_hover_text(
            "Lines you draw for yourself. They show on every glyph and are not part of the font.",
        );
        ui.horizontal(|ui| {
            ui.label("Background");
            ui.selectable_value(&mut self.dark_canvas, false, "White");
            ui.selectable_value(&mut self.dark_canvas, true, "Black");
        });
        ui.add_space(8.0);
        if ui.button("Reset to defaults").clicked() {
            *self = Self::default();
        }
        (self.cell_size - before_cell).abs() > f32::EPSILON
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_settings_stay_on_the_white_canvas() {
        let settings: Settings = serde_json::from_str(r#"{"show_guides":true}"#).unwrap();
        assert!(!settings.dark_canvas);
        assert!(settings.show_guides);
        assert!(settings.recent.is_empty());
        // Missing key uses the pyramid default: Overview does not force the list on.
        assert!(!settings.glyph_list_in_overview);
    }

    #[test]
    fn glyph_list_override_defaults_off() {
        assert!(!Settings::default().glyph_list_in_overview);
    }

    #[test]
    fn older_show_glyph_list_true_does_not_stick() {
        // Pre-pyramid saves wrote show_glyph_list:true as the old always-on default.
        // The renamed field must ignore that key so Overview stays clear.
        let settings: Settings =
            serde_json::from_str(r#"{"show_glyph_list":true,"show_guides":true}"#).unwrap();
        assert!(!settings.glyph_list_in_overview);
        assert!(settings.show_guides);
    }

    #[test]
    fn recent_moves_a_repeat_to_the_top_and_caps_the_list() {
        let mut settings = Settings::default();
        settings.remember_recent(Path::new("a.json"));
        settings.remember_recent(Path::new("b.ufo"));
        settings.remember_recent(Path::new("a.json"));
        assert_eq!(
            settings.recent,
            vec![PathBuf::from("a.json"), PathBuf::from("b.ufo")]
        );

        for index in 0..(RECENT_LIMIT + 5) {
            settings.remember_recent(Path::new(&format!("f{index}.ttf")));
        }
        assert_eq!(settings.recent.len(), RECENT_LIMIT);
        assert_eq!(
            settings.recent[0],
            PathBuf::from(format!("f{}.ttf", RECENT_LIMIT + 4))
        );
    }
}
