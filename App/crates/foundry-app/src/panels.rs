//! The glyph list and the inspector. Every field change is a command.

use eframe::egui;
use serde_json::{Value, json};

use crate::app::{FoundryWindow, Mode, Scope, parse_unicode};

impl FoundryWindow {
    pub fn glyph_list(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.weak(format!("Glyphs · {}", self.glyphs.len()));
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filter")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let filter = self.filter.trim().to_lowercase();
        let rows: Vec<(String, Option<u32>)> = self
            .glyphs
            .iter()
            .filter(|entry| {
                filter.is_empty()
                    || entry.name.to_lowercase().contains(&filter)
                    || entry
                        .unicode
                        .and_then(char::from_u32)
                        .is_some_and(|c| c.to_lowercase().to_string() == filter)
            })
            .map(|entry| (entry.name.clone(), entry.unicode))
            .collect();
        let mut chosen = None;
        let mut open = None;
        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            row_height,
            rows.len(),
            |ui, range| {
                for (name, unicode) in &rows[range] {
                    let selected = self.current.as_deref() == Some(name.as_str());
                    let label = match unicode
                        .and_then(char::from_u32)
                        .filter(|c| !c.is_control() && *name != c.to_string())
                    {
                        Some(c) => format!("{c}   {name}"),
                        None => format!("     {name}"),
                    };
                    let response = ui.selectable_label(selected, label);
                    if response.clicked() {
                        chosen = Some(name.clone());
                    }
                    if response.double_clicked() {
                        open = Some(name.clone());
                    }
                }
            },
        );
        if let Some(name) = chosen {
            self.select_glyph(Some(name));
        }
        if let Some(name) = open {
            self.open_editor(name);
        }
    }

    pub fn inspector(&mut self, ui: &mut egui::Ui) {
        let force = self.inspector_apply_open;
        let font_open = force.then_some(self.inspector_font_open);
        let style_open = force.then_some(self.inspector_style_open);
        let glyph_open = force.then_some(self.inspector_glyph_open);
        let selection_open = force.then_some(self.inspector_selection_open);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::CollapsingHeader::new("Font")
                    .open(font_open)
                    .show(ui, |ui| self.font_section(ui));
                egui::CollapsingHeader::new("Style")
                    .open(style_open)
                    .show(ui, |ui| self.style_section(ui));
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
