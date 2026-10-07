//! The review sheet: a small copy editor beside a format sheet set in the open font. It is a
//! pane of its own, and it can also dock beside the glyph grid.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use foundry_app::palette::{AMBER, BONE, FOCUS, GUIDE, MUTED};
use foundry_app::{Pt, Viewport};
use serde::{Deserialize, Serialize};

use crate::app::{FoundryWindow, Mode};
use crate::canvas::paint_fill;
use crate::color;

const HEADLINE_PX: f32 = 48.0;
const PARAGRAPH_PX: f32 = 15.0;
const WATERFALL: [f32; 4] = [36.0, 24.0, 16.0, 11.0];
const FALLBACK_SAMPLE: &str = "Hamburgefonts";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Headline,
    Paragraph,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub id: u64,
    pub role: Role,
    pub text: String,
}

impl Block {
    pub fn new(id: u64, role: Role, text: impl Into<String>) -> Self {
        Self {
            id,
            role,
            text: text.into(),
        }
    }
}

pub fn starter_copy() -> Vec<Block> {
    vec![
        Block::new(1, Role::Headline, "Hamburgefonts"),
        Block::new(
            2,
            Role::Paragraph,
            "The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs.",
        ),
    ]
}

/// The characters a set specimen draws, one line per row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CharSet {
    #[default]
    Alphabet,
    Figures,
    Punctuation,
    /// Repeated letters, to judge spacing by rhythm.
    Rhythm,
}

impl CharSet {
    pub const ALL: [CharSet; 4] = [
        CharSet::Alphabet,
        CharSet::Figures,
        CharSet::Punctuation,
        CharSet::Rhythm,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CharSet::Alphabet => "Alphabet",
            CharSet::Figures => "Figures",
            CharSet::Punctuation => "Punctuation",
            CharSet::Rhythm => "Rhythm",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            CharSet::Alphabet => "ABCDEFGHIJKLMNOPQRSTUVWXYZ\nabcdefghijklmnopqrstuvwxyz",
            CharSet::Figures => "0123456789",
            CharSet::Punctuation => {
                ".,:;!?-\u{2013}\u{2014}'\"\u{2018}\u{2019}\u{201C}\u{201D}()[]{}/\\&@#*_+=<>|~"
            }
            CharSet::Rhythm => {
                "nnnnnnnnnn\noooooooooo\nnononononono\nHOHOHOHOHO\nHHHHHHHHHH\nhhhhhhhhhh\nIlIlIlIlIl\nuuuuuuuuuu"
            }
        }
    }
}

/// Lines and labels drawn over each sample line. View state only; never written to the font.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Guides {
    pub ascender: bool,
    pub cap_height: bool,
    pub x_height: bool,
    pub baseline: bool,
    pub descender: bool,
    /// Each glyph's advance box, so the gaps between the boxes show the spacing.
    pub spacing: bool,
    /// Name and value beside each line.
    pub labels: bool,
}

impl Default for Guides {
    fn default() -> Self {
        Self {
            ascender: false,
            cap_height: true,
            x_height: true,
            baseline: true,
            descender: false,
            spacing: false,
            labels: true,
        }
    }
}

/// The review pane's specimen settings. They persist between launches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewView {
    pub set: CharSet,
    pub size: f32,
    pub guides: Guides,
}

impl Default for ReviewView {
    fn default() -> Self {
        Self {
            set: CharSet::Alphabet,
            size: 72.0,
            guides: Guides::default(),
        }
    }
}

/// The label for one style: its name, its weight class, and whether it is italic.
pub fn style_title(style: &str, weight: u16, italic: bool) -> String {
    let slant = if italic { " · italic" } else { "" };
    format!("{style} · weight {weight}{slant}")
}

/// One character placed on a line. `name` is missing when the font has no glyph for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub ch: char,
    pub name: Option<String>,
    pub advance: f64,
}

/// Break `text` into lines that fit `measure` font units.
///
/// A newline forces a break, including a blank line. Wrapping breaks on whitespace.
/// A space that would start a line is dropped. A word wider than `measure` stays whole
/// on its own line.
pub fn lay_text(
    text: &str,
    measure: f64,
    mut lookup: impl FnMut(char) -> (Option<String>, f64),
) -> Vec<Vec<Placed>> {
    let measure = measure.max(0.0);
    let normalized = text.replace('\r', "");
    let mut lines = Vec::new();
    for paragraph in normalized.split('\n') {
        if paragraph.is_empty() {
            lines.push(Vec::new());
            continue;
        }
        lines.extend(wrap_paragraph(paragraph, measure, &mut lookup));
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn wrap_paragraph(
    paragraph: &str,
    measure: f64,
    lookup: &mut impl FnMut(char) -> (Option<String>, f64),
) -> Vec<Vec<Placed>> {
    let mut lines = Vec::new();
    let mut line = Vec::new();
    let mut width = 0.0;
    let chars: Vec<char> = paragraph.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let space = chars[index].is_whitespace();
        let start = index;
        while index < chars.len() && chars[index].is_whitespace() == space {
            index += 1;
        }
        let token: Vec<Placed> = chars[start..index]
            .iter()
            .map(|ch| {
                let (name, advance) = lookup(*ch);
                Placed {
                    ch: *ch,
                    name,
                    advance,
                }
            })
            .collect();
        let token_width: f64 = token.iter().map(|placed| placed.advance).sum();
        if space {
            if line.is_empty() {
                continue;
            }
            if width + token_width <= measure {
                width += token_width;
                line.extend(token);
            } else {
                lines.push(std::mem::take(&mut line));
                width = 0.0;
            }
            continue;
        }
        if !line.is_empty() && width + token_width > measure {
            lines.push(std::mem::take(&mut line));
            width = 0.0;
        }
        width += token_width;
        line.extend(token);
        if width > measure {
            lines.push(std::mem::take(&mut line));
            width = 0.0;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

impl FoundryWindow {
    pub fn preview_pane(&mut self, ui: &mut egui::Ui) {
        let height = ui.available_height();
        let total = ui.available_width();
        let copy_width = (total * 0.36)
            .clamp(240.0, 440.0)
            .min((total * 0.55).max(1.0));
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(copy_width, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.copy_column(ui),
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.sheet_column(ui),
            );
        });
    }

    fn copy_column(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Copy");
            if ui.button("Add headline").clicked() {
                self.push_block(Role::Headline, "New headline");
            }
            if ui.button("Add paragraph").clicked() {
                self.push_block(Role::Paragraph, "");
            }
        });
        let mut remove = None;
        let scroll_height = ui.available_height().max(40.0);
        egui::ScrollArea::vertical()
            .id_salt("preview-copy")
            .max_height(scroll_height)
            .auto_shrink(false)
            .show(ui, |ui| {
                for block in &mut self.copy {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(block.role == Role::Headline, "Headline")
                            .clicked()
                        {
                            block.role = Role::Headline;
                        }
                        if ui
                            .selectable_label(block.role == Role::Paragraph, "Paragraph")
                            .clicked()
                        {
                            block.role = Role::Paragraph;
                        }
                        if ui.button("Remove").clicked() {
                            remove = Some(block.id);
                        }
                    });
                    let rows = if block.role == Role::Headline { 2 } else { 5 };
                    let hint = if block.role == Role::Headline {
                        "Headline"
                    } else {
                        "Paragraph"
                    };
                    ui.add(
                        egui::TextEdit::multiline(&mut block.text)
                            .id_salt(("preview-block", block.id))
                            .desired_rows(rows)
                            .desired_width(f32::INFINITY)
                            .hint_text(hint),
                    );
                    ui.add_space(8.0);
                }
                if self.copy.is_empty() {
                    ui.weak("Add a headline or a paragraph. The sheet sets it in this font.");
                }
            });
        if let Some(id) = remove {
            self.copy.retain(|block| block.id != id);
        }
    }

    fn push_block(&mut self, role: Role, text: &str) {
        let id = self
            .copy
            .iter()
            .map(|block| block.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.copy.push(Block::new(id, role, text));
    }

    fn sheet_column(&mut self, ui: &mut egui::Ui) {
        ui.strong("Format sheet");
        if self.mode == Mode::Review {
            self.review_controls(ui);
        }
        let blocks = self.copy.clone();
        let sample = blocks
            .iter()
            .find(|block| block.role == Role::Headline && !block.text.trim().is_empty())
            .map(|block| block.text.clone())
            .unwrap_or_else(|| FALLBACK_SAMPLE.to_string());
        let view = self.settings.review.clone();
        let guides = view.guides;
        let mut clicked = None;
        let scroll_height = ui.available_height().max(40.0);
        egui::ScrollArea::vertical()
            .id_salt("preview-sheet")
            .max_height(scroll_height)
            .auto_shrink(false)
            .show(ui, |ui| {
                for block in &blocks {
                    let (label, size) = match block.role {
                        Role::Headline => ("Headline · 48", HEADLINE_PX),
                        Role::Paragraph => ("Paragraph · 15", PARAGRAPH_PX),
                    };
                    ui.weak(label);
                    let hit = self.paint_sample(ui, None, &block.text, size, &guides);
                    if clicked.is_none() {
                        clicked = hit;
                    }
                    ui.add_space(10.0);
                }
                ui.separator();
                ui.weak("Sizes");
                for size in WATERFALL {
                    ui.weak(format!("{size:.0} px"));
                    let hit = self.paint_sample(ui, None, &sample, size, &guides);
                    if clicked.is_none() {
                        clicked = hit;
                    }
                    ui.add_space(6.0);
                }
                // The chosen set at the review size, in the active font.
                ui.separator();
                ui.weak(format!("Set · {} · {:.0} px", view.set.label(), view.size));
                let hit = self.paint_sample(ui, None, view.set.text(), view.size, &guides);
                if clicked.is_none() {
                    clicked = hit;
                }
                // Every style of this family, lightest first, labeled with its weight.
                if self.family_preview || self.mode == Mode::Review {
                    let family = self
                        .session
                        .font()
                        .map(|font| font.style.family.clone())
                        .unwrap_or_default();
                    let mut styles: Vec<(u32, String, u16, bool)> = self
                        .tabs
                        .iter()
                        .filter(|tab| tab.family == family)
                        .map(|tab| (tab.id, tab.style.clone(), tab.weight, tab.italic))
                        .collect();
                    styles.sort_by_key(|(_, _, weight, italic)| (*weight, *italic));
                    ui.separator();
                    ui.weak("Styles");
                    for (id, style, weight, italic) in styles {
                        ui.weak(style_title(&style, weight, italic));
                        let hit =
                            self.paint_sample(ui, Some(id), view.set.text(), view.size, &guides);
                        if clicked.is_none() {
                            clicked = hit;
                        }
                        ui.add_space(6.0);
                    }
                }
            });
        if let Some((font, name)) = clicked {
            if let Some(id) = font {
                self.switch_to(id);
            }
            self.select_glyph(Some(name));
            if self.mode == Mode::Editor {
                self.view = None;
            }
        }
    }

    /// The set, size, and overlay controls at the top of the review pane.
    fn review_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("review-set")
                .selected_text(self.settings.review.set.label())
                .show_ui(ui, |ui| {
                    for set in CharSet::ALL {
                        ui.selectable_value(&mut self.settings.review.set, set, set.label());
                    }
                });
            ui.add(
                egui::Slider::new(&mut self.settings.review.size, 16.0..=200.0)
                    .suffix(" px")
                    .text("Size"),
            );
        });
        let guides = &mut self.settings.review.guides;
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut guides.ascender, "Ascender");
            ui.checkbox(&mut guides.cap_height, "Cap height");
            ui.checkbox(&mut guides.x_height, "x-height");
            ui.checkbox(&mut guides.baseline, "Baseline");
            ui.checkbox(&mut guides.descender, "Descender");
            ui.checkbox(&mut guides.spacing, "Spacing").on_hover_text(
                "Each glyph's advance box. The gaps between the boxes are the spacing.",
            );
            ui.checkbox(&mut guides.labels, "Labels");
        });
        ui.checkbox(&mut self.family_preview, "Every style");
        ui.add_space(4.0);
    }

    /// Draw one wrapped sample on a bone card, in the active font or the open font `font`.
    /// Returns the font and glyph under a click.
    fn paint_sample(
        &mut self,
        ui: &mut egui::Ui,
        font: Option<u32>,
        text: &str,
        pixel_size: f32,
        guides: &Guides,
    ) -> Option<(Option<u32>, String)> {
        let id = font.or(self.active)?;
        let span = (self.ascender - self.descender).abs().max(1.0);
        let scale = f64::from(pixel_size) / span;
        let width = ui.available_width().max(8.0);
        let measure = (f64::from(width) - 16.0).max(1.0) / scale;
        let lines = lay_text(text, measure, |ch| self.glyph_advance(id, ch));
        let line_gap = span * scale * 1.3;
        let height = (8.0 + line_gap * lines.len() as f64) as f32;
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(width, height.max(pixel_size + 8.0)),
            Sense::click(),
        );
        let painter = ui.painter_at(rect).with_clip_rect(rect);
        painter.rect_filled(rect, CornerRadius::same(3), color(BONE));
        let pointer = response
            .clicked()
            .then(|| response.interact_pointer_pos())
            .flatten();
        let mut clicked = None;
        let mut baseline = f64::from(rect.top()) + 4.0 + self.ascender * scale;
        let box_height = self.cap_height.max(span * 0.7);
        for line in &lines {
            // Guides go under the glyphs of their own line.
            self.paint_metric_lines(&painter, rect, baseline, scale, guides);
            let mut x = f64::from(rect.left()) + 8.0;
            for placed in line {
                let advance = placed.advance * scale;
                if guides.spacing {
                    let top = baseline - self.ascender * scale;
                    let bottom = baseline - self.descender * scale;
                    paint_advance(&painter, x, advance, top as f32, bottom as f32);
                }
                if placed.name.is_none() {
                    let top = baseline - box_height * scale;
                    let missing = Rect::from_min_size(
                        Pos2::new((x + 1.0) as f32, top as f32),
                        Vec2::new((advance - 2.0).max(1.0) as f32, (box_height * scale) as f32),
                    );
                    painter.rect_stroke(
                        missing,
                        CornerRadius::ZERO,
                        Stroke::new(1.0, color(MUTED)),
                        egui::StrokeKind::Inside,
                    );
                } else if let Some(name) = &placed.name
                    && let Some(outline) = self.outline_in(id, name)
                {
                    let view = Viewport {
                        scale,
                        origin: Pt::new(x, baseline),
                    };
                    paint_fill(&painter, &view, rect, &outline, Color32::BLACK);
                    if Some(id) == self.active && self.current.as_deref() == Some(name.as_str()) {
                        let y = (baseline - self.descender * scale * 0.35) as f32;
                        painter.hline(
                            (x as f32)..=((x + advance) as f32),
                            y,
                            Stroke::new(2.0, color(FOCUS)),
                        );
                    }
                }
                if clicked.is_none()
                    && let Some(press) = pointer
                    && let Some(name) = &placed.name
                    && f64::from(press.x) >= x
                    && f64::from(press.x) < x + advance.max(1.0)
                {
                    let top = baseline - self.ascender * scale;
                    let bottom = baseline - self.descender * scale;
                    if f64::from(press.y) >= top && f64::from(press.y) < bottom {
                        clicked = Some((font, name.clone()));
                    }
                }
                x += advance;
            }
            baseline += line_gap;
        }
        clicked
    }

    /// The metric lines chosen in the review controls, across one sample line. The metrics are
    /// the active font's, so every style of the family is measured against the same lines.
    fn paint_metric_lines(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        baseline: f64,
        scale: f64,
        guides: &Guides,
    ) {
        let lines = [
            (guides.ascender, "ascender", self.ascender, MUTED),
            (guides.cap_height, "cap height", self.cap_height, AMBER),
            (guides.x_height, "x-height", self.x_height, AMBER),
            (guides.baseline, "baseline", 0.0, GUIDE),
            (guides.descender, "descender", self.descender, MUTED),
        ];
        for (on, label, value, rgb) in lines {
            if !on {
                continue;
            }
            let y = (baseline - value * scale) as f32;
            painter.hline(
                (rect.left() + 2.0)..=(rect.right() - 2.0),
                y,
                Stroke::new(1.0, color(rgb)),
            );
            if guides.labels {
                painter.text(
                    Pos2::new(rect.right() - 6.0, y - 1.0),
                    Align2::RIGHT_BOTTOM,
                    format!("{label} {value:.0}"),
                    FontId::proportional(10.0),
                    color(rgb),
                );
            }
        }
    }

    fn glyph_advance(&mut self, font: u32, ch: char) -> (Option<String>, f64) {
        let missing = (self.ascender - self.descender).abs().max(1.0) * 0.5;
        let Some(name) = self.by_unicode.get(&u32::from(ch)).cloned() else {
            return (None, missing);
        };
        let advance = self
            .outline_in(font, &name)
            .map(|outline| outline.advance)
            .unwrap_or(missing);
        (Some(name), advance.max(0.0))
    }
}

/// The advance box of one glyph: a faint fill with an amber edge on each side.
fn paint_advance(painter: &egui::Painter, x: f64, advance: f64, top: f32, bottom: f32) {
    let left = x as f32;
    let right = (x + advance) as f32;
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom)),
        CornerRadius::ZERO,
        Color32::from_rgba_unmultiplied(0xE8, 0xB6, 0x5A, 30),
    );
    painter.vline(left, top..=bottom, Stroke::new(1.0, color(AMBER)));
    painter.vline(right, top..=bottom, Stroke::new(1.0, color(AMBER)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(text: &str, measure: f64) -> Vec<String> {
        lay_text(text, measure, |ch| (Some(ch.to_string()), 1.0))
            .into_iter()
            .map(|line| line.into_iter().map(|placed| placed.ch).collect())
            .collect()
    }

    #[test]
    fn wraps_on_whitespace_and_drops_a_leading_space() {
        assert_eq!(lines_of("The quick", 3.0), vec!["The", "quick"]);
        assert_eq!(lines_of("aa bb", 5.0), vec!["aa bb"]);
        assert_eq!(lines_of("  hi", 10.0), vec!["hi"]);
    }

    #[test]
    fn a_newline_forces_a_break_and_a_blank_line() {
        assert_eq!(lines_of("a\nb", 20.0), vec!["a", "b"]);
        assert_eq!(lines_of("a\r\n\r\nb", 20.0), vec!["a", "", "b"]);
        assert_eq!(lines_of("", 20.0), vec![""]);
    }

    #[test]
    fn an_overlong_word_stays_on_its_own_line() {
        assert_eq!(lines_of("abcdef gh", 3.0), vec!["abcdef", "gh"]);
    }

    #[test]
    fn a_run_keeps_the_glyph_name() {
        let lines = lay_text("A", 10.0, |ch| (Some(format!("glyph-{ch}")), 2.0));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0][0].name.as_deref(), Some("glyph-A"));
        assert!((lines[0][0].advance - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn every_set_has_lines_and_the_set_labels_are_distinct() {
        for set in CharSet::ALL {
            assert!(!set.text().is_empty(), "{}", set.label());
        }
        let mut labels: Vec<&str> = CharSet::ALL.iter().map(|set| set.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), CharSet::ALL.len());
    }

    #[test]
    fn a_style_label_names_its_weight_and_slant() {
        assert_eq!(style_title("Bold", 700, false), "Bold · weight 700");
        assert_eq!(
            style_title("Italic", 400, true),
            "Italic · weight 400 · italic"
        );
    }
}
