//! The glyph list and the inspector. Every field change is a command.

use eframe::egui;
use foundry_api::GlyphGroup;
use foundry_app::palette::{ALERT, AMBER, MUTED, SIGNAL};
use serde_json::{Value, json};

use crate::app::{FoundryWindow, GlyphEntry, Mode, Scope, parse_unicode};
use crate::color;

impl FoundryWindow {
    /// The glyph tree: each open font, then its glyphs in groups such as Uppercase and Figures.
    /// The active font opens by default. Clicking a glyph in another font switches to that font.
    pub fn glyph_list(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        ui.weak(format!("Fonts · {}", self.tabs.len()));
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filter")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let filter = self.filter.trim().to_lowercase();
        let filtering = !filter.is_empty();

        // Read every font's glyphs before drawing, so the tree can hold them while it draws.
        let heads: Vec<(u32, String, bool)> = self
            .tabs
            .iter()
            .map(|tab| (tab.id, format!("{} · {}", tab.family, tab.style), tab.dirty))
            .collect();
        let fonts: Vec<(u32, String, bool, Vec<GlyphEntry>)> = heads
            .into_iter()
            .map(|(id, title, dirty)| (id, title, dirty, self.font_glyphs_of(id)))
            .collect();

        let active = self.active;
        let current = self.current.clone();
        let mut pick = None;
        let mut open = None;
        let mut any = false;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (id, title, dirty, glyphs) in &fonts {
                    let members: Vec<&GlyphEntry> = glyphs
                        .iter()
                        .filter(|entry| glyph_matches(&filter, entry))
                        .collect();
                    if filtering && members.is_empty() {
                        continue;
                    }
                    any = true;
                    let is_active = Some(*id) == active;
                    let heading = format!(
                        "{title}  {}{}",
                        glyphs.len(),
                        if *dirty { "  •" } else { "" }
                    );
                    egui::CollapsingHeader::new(heading)
                        .id_salt(("font", *id))
                        .default_open(is_active)
                        .open(filtering.then_some(true))
                        .show(ui, |ui| {
                            for group in GlyphGroup::ALL {
                                let rows: Vec<&GlyphEntry> = members
                                    .iter()
                                    .copied()
                                    .filter(|entry| entry.group == group)
                                    .collect();
                                if rows.is_empty() {
                                    continue;
                                }
                                let letters = matches!(
                                    group,
                                    GlyphGroup::Uppercase
                                        | GlyphGroup::Lowercase
                                        | GlyphGroup::Figures
                                );
                                egui::CollapsingHeader::new(format!(
                                    "{}  {}",
                                    group.label(),
                                    rows.len()
                                ))
                                .id_salt(("group", *id, group.label()))
                                .default_open(letters)
                                .open(filtering.then_some(true))
                                .show(ui, |ui| {
                                    for entry in rows {
                                        let selected = is_active
                                            && current.as_deref() == Some(entry.name.as_str());
                                        let response =
                                            ui.selectable_label(selected, glyph_label(entry));
                                        if response.clicked() {
                                            pick = Some((*id, entry.name.clone()));
                                        }
                                        if response.double_clicked() {
                                            open = Some((*id, entry.name.clone()));
                                        }
                                    }
                                });
                            }
                        });
                }
                if filtering && !any {
                    ui.weak("No glyphs match.");
                }
            });
        if let Some((id, name)) = pick {
            self.reach_glyph(id, name);
        }
        if let Some((id, name)) = open {
            self.reach_glyph(id, name.clone());
            self.open_editor(name);
        }
    }

    /// Makes `name` in font `id` the selected glyph, switching fonts first when needed.
    fn reach_glyph(&mut self, id: u32, name: String) {
        if Some(id) != self.active {
            self.switch_to(id);
        }
        self.select_glyph(Some(name));
    }

    pub fn inspector(&mut self, ui: &mut egui::Ui) {
        let force = self.inspector_apply_open;
        let font_open = force.then_some(self.inspector_font_open);
        let style_open = force.then_some(self.inspector_style_open);
        let glyph_open = force.then_some(self.inspector_glyph_open);
        let selection_open = force.then_some(self.inspector_selection_open);
        let genome_open = force.then_some(self.inspector_genome_open);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::CollapsingHeader::new("Font")
                    .open(font_open)
                    .show(ui, |ui| self.font_section(ui));
                egui::CollapsingHeader::new("Style")
                    .open(style_open)
                    .show(ui, |ui| self.style_section(ui));
                if self.has_font() {
                    egui::CollapsingHeader::new("Genome")
                        .open(genome_open)
                        .show(ui, |ui| self.genome_section(ui));
                }
                if self.current.is_some() {
                    egui::CollapsingHeader::new("Glyph")
                        .open(glyph_open)
                        .show(ui, |ui| self.glyph_section(ui));
                }
                let show_selection = (self.mode == Mode::Editor || self.settings.split_main)
                    && !self.selection.is_empty();
                if show_selection {
                    egui::CollapsingHeader::new("Selection")
                        .open(selection_open)
                        .show(ui, |ui| self.selection_section(ui));
                }
            });
        self.inspector_apply_open = false;
    }

    fn genome_section(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("genome_fields")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Stem tolerance");
                ui.add(
                    egui::DragValue::new(&mut self.genome_stem_tolerance)
                        .speed(0.5)
                        .range(0.0..=64.0),
                )
                .on_hover_text("Absolute units allowed from the median stem when capturing.");
                ui.end_row();
                if let Some(data) = &self.genome_snapshot {
                    if let Some(stem) = data["primary_stem"].as_f64() {
                        ui.label("Primary stem");
                        ui.label(format!("{stem:.1}"));
                        ui.end_row();
                    }
                    if let Some(side) = data["median_sidebearing"].as_f64() {
                        ui.label("Median sidebearing");
                        ui.label(format!("{side:.1}"));
                        ui.end_row();
                    }
                    if let Some(tol) = data["stem_tolerance"].as_f64() {
                        ui.label("Captured tolerance");
                        ui.label(format!("{tol:.1}"));
                        ui.end_row();
                    }
                    if let Some(samples) = data["samples"].as_array() {
                        let names: Vec<&str> = samples.iter().filter_map(Value::as_str).collect();
                        if !names.is_empty() {
                            ui.label("Samples");
                            ui.label(names.join(", "));
                            ui.end_row();
                        }
                    }
                } else {
                    ui.label("Genome");
                    ui.colored_label(color(MUTED), "Not measured yet");
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Measure")
                .on_hover_text("Live stems and sidebearings (measure)")
                .clicked()
            {
                self.measure_genome();
            }
            if ui
                .button("Capture")
                .on_hover_text("Store Style Genome on the font")
                .clicked()
            {
                self.capture_genome();
            }
            if ui
                .button("Check")
                .on_hover_text("Compare live stems to the captured genome")
                .clicked()
            {
                self.check_genome();
            }
            if ui
                .button("Audit")
                .on_hover_text("Technical outline/spacing plus design genome issues")
                .clicked()
            {
                self.run_audit();
            }
            if ui
                .button("Critique")
                .on_hover_text("Ranked suggestions with confidence from audit / genome")
                .clicked()
            {
                self.run_critique();
            }
        });
        if let Some(report) = self.audit_report.clone() {
            ui.add_space(6.0);
            ui.separator();
            genome_audit_body(ui, &report);
        }
        if !self.critique_suggestions.is_empty() {
            ui.add_space(6.0);
            ui.separator();
            ui.strong(format!(
                "Critique · {} suggestion(s)",
                self.critique_suggestions.len()
            ));
            let suggestions = self.critique_suggestions.clone();
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for suggestion in &suggestions {
                        let id = suggestion["id"].as_str().unwrap_or_default();
                        let rank = suggestion["rank"].as_u64().unwrap_or(0);
                        let confidence = suggestion["confidence"].as_f64().unwrap_or(0.0);
                        let layer = suggestion["layer"].as_str().unwrap_or("design");
                        let issue = suggestion["issue"].as_str().unwrap_or_default();
                        let observation = suggestion["observation"].as_str().unwrap_or_default();
                        let intervention =
                            suggestion["intervention"].as_str().unwrap_or_default();
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(color(AMBER), format!("#{rank}"));
                            ui.strong(issue);
                            ui.colored_label(color(MUTED), layer);
                            ui.label(format!("{:.0}%", confidence * 100.0));
                        });
                        ui.label(observation);
                        ui.colored_label(color(SIGNAL), intervention);
                        ui.horizontal(|ui| {
                            if ui.button("Accept").clicked() {
                                self.resolve_critique_suggestion(id, true);
                            }
                            if ui.button("Reject").clicked() {
                                self.resolve_critique_suggestion(id, false);
                            }
                        });
                        ui.separator();
                    }
                });
        }
    }

    fn font_section(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("font_fields")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Name");
                let response = ui.text_edit_singleline(&mut self.font_name_edit);
                if response.lost_focus()
                    && self.font_name_edit.trim() != self.font_name
                    && !self.font_name_edit.trim().is_empty()
                {
                    let name = self.font_name_edit.trim().to_string();
                    self.edit_json(json!({ "op": "rename_font", "name": name }), Scope::Font);
                }
                ui.end_row();
                ui.label("Units per em");
                ui.label(self.upm.to_string());
                ui.end_row();
                let metrics = [
                    ("Ascender", "ascender", self.ascender),
                    ("Cap height", "cap_height", self.cap_height),
                    ("x-height", "x_height", self.x_height),
                    ("Descender", "descender", self.descender),
                ];
                for (label, key, value) in metrics {
                    ui.label(label);
                    let mut edited = value;
                    let response = ui.add(egui::DragValue::new(&mut edited).speed(1.0));
                    if response.drag_started() || response.gained_focus() {
                        self.checkpoint();
                    }
                    if response.changed() && edited.is_finite() {
                        let mut command = json!({ "op": "set_metrics" });
                        command[key] = json!(edited);
                        self.edit_json(command, Scope::Font);
                    }
                    ui.end_row();
                }
            });
    }

    fn glyph_section(&mut self, ui: &mut egui::Ui) {
        let Some(name) = self.current.clone() else {
            return;
        };
        let Some(outline) = self.outline(&name) else {
            return;
        };
        if self.edits_for.as_deref() != Some(name.as_str()) {
            self.glyph_name_edit = name.clone();
            self.unicode_edit = outline
                .unicode
                .map(|code| format!("{code:04X}"))
                .unwrap_or_default();
            self.edits_for = Some(name.clone());
        }
        egui::Grid::new("glyph_fields")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Name");
                let response = ui.text_edit_singleline(&mut self.glyph_name_edit);
                if response.lost_focus() && self.glyph_name_edit.trim() != name {
                    let new_name = self.glyph_name_edit.trim().to_string();
                    if self
                        .edit_json(
                            json!({ "op": "rename_glyph", "name": name, "new_name": new_name }),
                            Scope::Structure,
                        )
                        .is_some()
                    {
                        self.current = Some(new_name.clone());
                        self.edits_for = None;
                    } else {
                        self.glyph_name_edit = name.clone();
                    }
                }
                ui.end_row();

                ui.label("Unicode");
                let response = ui
                    .text_edit_singleline(&mut self.unicode_edit)
                    .on_hover_text("Hex like 0041, or one character. Empty for none.");
                if response.lost_focus() {
                    let code = parse_unicode(&self.unicode_edit);
                    if code != outline.unicode {
                        let value = code.map_or(Value::Null, |code| json!(code));
                        self.edit_json(
                            json!({ "op": "set_unicode", "name": name, "unicode": value }),
                            Scope::Structure,
                        );
                    }
                    self.edits_for = None;
                }
                ui.end_row();

                ui.label("Advance");
                let mut advance = outline.advance;
                let response = ui.add(egui::DragValue::new(&mut advance).speed(1.0));
                if response.drag_started() || response.gained_focus() {
                    self.checkpoint();
                }
                if response.changed() && advance.is_finite() {
                    self.edit_json(
                        json!({ "op": "set_advance", "name": name, "advance": advance }),
                        Scope::Glyph(name.clone()),
                    );
                }
                ui.end_row();

                if let Some(ink) = outline.ink_bounds() {
                    ui.label("Sidebearings");
                    ui.label(format!(
                        "L {:.0}   R {:.0}",
                        ink.min.x,
                        outline.advance - ink.max.x
                    ));
                    ui.end_row();
                    ui.label("Ink box");
                    ui.label(format!("{:.0} × {:.0}", ink.width(), ink.height()));
                    ui.end_row();
                }
                ui.label("Contours");
                let points: usize = outline.contours.iter().map(|c| c.points.len()).sum();
                ui.label(format!("{} · {points} points", outline.contours.len()));
                ui.end_row();
            });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Edit").clicked() {
                self.open_editor(name.clone());
            }
            if ui
                .button("Reverse")
                .on_hover_text("Reverse contour direction")
                .clicked()
            {
                self.reverse_contours();
            }
            if ui
                .button("Round")
                .on_hover_text("Round coordinates to whole units")
                .clicked()
            {
                self.round_glyphs(false);
            }
            if ui.button("Delete").clicked() {
                self.delete_current_glyph();
            }
        });
    }

    fn selection_section(&mut self, ui: &mut egui::Ui) {
        let Some(name) = self.current.clone() else {
            return;
        };
        let Some(outline) = self.outline(&name) else {
            return;
        };
        let handles: Vec<_> = self.selection.iter().copied().collect();
        if let [handle] = handles.as_slice()
            && let Some(point) = outline.point(*handle)
        {
            let point = point.clone();
            let handle = *handle;
            egui::Grid::new("point_fields")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Point");
                    ui.label(format!("contour {} · {}", handle.contour, handle.point));
                    ui.end_row();
                    let (mut x, mut y) = (point.at.x, point.at.y);
                    ui.label("X");
                    let rx = ui.add(egui::DragValue::new(&mut x).speed(1.0));
                    ui.end_row();
                    ui.label("Y");
                    let ry = ui.add(egui::DragValue::new(&mut y).speed(1.0));
                    ui.end_row();
                    if rx.drag_started()
                        || ry.drag_started()
                        || rx.gained_focus()
                        || ry.gained_focus()
                    {
                        self.checkpoint();
                    }
                    if (rx.changed() || ry.changed()) && x.is_finite() && y.is_finite() {
                        self.edit_json(
                            json!({
                                "op": "move_point", "name": name, "contour": handle.contour,
                                "point": handle.point, "x": x, "y": y,
                            }),
                            Scope::Glyph(name.clone()),
                        );
                    }
                    ui.label("Type");
                    ui.horizontal(|ui| {
                        if ui.selectable_label(point.on, "On-curve").clicked() && !point.on {
                            self.set_kind(true);
                        }
                        if ui.selectable_label(!point.on, "Off-curve").clicked() && point.on {
                            self.set_kind(false);
                        }
                    });
                    ui.end_row();
                    if point.on {
                        ui.label("Smooth");
                        let mut smooth = point.smooth;
                        if ui.checkbox(&mut smooth, "").changed() {
                            self.set_smooth(smooth);
                        }
                        ui.end_row();
                    }
                });
        } else {
            ui.label(format!("{} points selected", handles.len()));
        }
        if handles.len() >= 2 {
            ui.add_space(4.0);
            ui.label("Align");
            ui.horizontal_wrapped(|ui| {
                for (label, how) in [
                    ("Left", crate::align::AlignTo::Left),
                    ("Center", crate::align::AlignTo::CenterX),
                    ("Right", crate::align::AlignTo::Right),
                    ("Top", crate::align::AlignTo::Top),
                    ("Middle", crate::align::AlignTo::Middle),
                    ("Bottom", crate::align::AlignTo::Bottom),
                ] {
                    if ui.button(label).clicked() {
                        self.align_selection(how);
                    }
                }
            });
            if handles.len() >= 3 {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Distribute horizontally").clicked() {
                        self.align_selection(crate::align::AlignTo::DistributeX);
                    }
                    if ui.button("Distribute vertically").clicked() {
                        self.align_selection(crate::align::AlignTo::DistributeY);
                    }
                });
            }
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Smooth").clicked() {
                self.set_smooth(true);
            }
            if ui.button("Corner").clicked() {
                self.set_smooth(false);
            }
            if ui.button("Delete").clicked() {
                self.delete_selection();
            }
            if ui.button("Transform…").clicked() {
                self.effects.open = true;
                self.effects.scope = crate::effects::Target::Selection;
            }
        });
    }
}

/// Show Foundry Audit / genome-check results under the Genome inspector actions.
fn genome_audit_body(ui: &mut egui::Ui, report: &Value) {
    let technical_count = report["technical"]["count"].as_u64().unwrap_or(0);
    let design_count = report["design"]["count"]
        .as_u64()
        .or_else(|| report["count"].as_u64())
        .unwrap_or(0);
    let total = report["count"]
        .as_u64()
        .unwrap_or(technical_count + design_count);
    if total == 0 {
        ui.colored_label(color(SIGNAL), "Audit clean — no technical or design issues.");
        return;
    }
    ui.colored_label(
        color(AMBER),
        format!("{total} issue(s) · tech {technical_count} · design {design_count}"),
    );
    ui.add_space(2.0);
    egui::ScrollArea::vertical()
        .max_height(220.0)
        .show(ui, |ui| {
            if let Some(outlines) = report["technical"]["outlines"].as_array() {
                for issue in outlines {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(
                            color(ALERT),
                            issue["code"].as_str().unwrap_or("outline"),
                        );
                        if let Some(glyph) = issue["glyph"].as_str() {
                            ui.strong(glyph);
                        }
                        ui.label(issue["detail"].as_str().unwrap_or_default());
                    });
                }
            }
            if let Some(spacing) = report["technical"]["spacing"].as_array() {
                for issue in spacing {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(
                            color(AMBER),
                            issue["code"].as_str().unwrap_or("spacing"),
                        );
                        ui.label(issue["detail"].as_str().unwrap_or_default());
                    });
                }
            }
            let design = report["design"]["issues"]
                .as_array()
                .or_else(|| report["issues"].as_array());
            if let Some(issues) = design {
                for issue in issues {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(
                            color(AMBER),
                            issue["code"].as_str().unwrap_or("design"),
                        );
                        if let Some(glyph) = issue["glyph"].as_str() {
                            ui.strong(glyph);
                        }
                        ui.label(issue["detail"].as_str().unwrap_or_default());
                    });
                }
            }
        });
}

/// The row text: the character when the glyph has one, then the glyph name.
fn glyph_label(entry: &GlyphEntry) -> String {
    match entry
        .unicode
        .and_then(char::from_u32)
        .filter(|c| !c.is_control() && entry.name != c.to_string())
    {
        Some(c) => format!("{c}   {}", entry.name),
        None => format!("     {}", entry.name),
    }
}

/// `filter` is already trimmed and lowercased. A one-character filter also matches that character.
fn glyph_matches(filter: &str, entry: &GlyphEntry) -> bool {
    filter.is_empty()
        || entry.name.to_lowercase().contains(filter)
        || entry
            .unicode
            .and_then(char::from_u32)
            .is_some_and(|c| c.to_lowercase().to_string() == filter)
}
