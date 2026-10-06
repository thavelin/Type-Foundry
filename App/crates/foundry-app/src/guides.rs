//! Drawing guides. They belong to the person using the window, not to the font: they are stored
//! with the view, shared by every glyph of a family, and never written into a font file.

use std::collections::BTreeMap;

use eframe::egui::{self, Pos2, Rect, Stroke};
use foundry_app::palette::{FOCUS, GUIDE};
use foundry_app::{Pt, Viewport};
use serde::{Deserialize, Serialize};

use crate::app::{Drag, FoundryWindow, Tone};
use crate::canvas::pt;
use crate::color;

/// A horizontal guide sits at `at` on Y. A vertical guide sits at `at` on X.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub vertical: bool,
    pub at: f64,
}

/// Guides kept between launches, keyed by family name.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GuideBook {
    pub by_family: BTreeMap<String, Vec<Guide>>,
}

const HIT: f64 = 6.0;

/// The guide whose line is closest to `screen`, within `radius` screen points.
pub fn nearest_guide(guides: &[Guide], view: &Viewport, screen: Pt, radius: f64) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, guide) in guides.iter().enumerate() {
        let distance = if guide.vertical {
            (view.to_screen(Pt::new(guide.at, 0.0)).x - screen.x).abs()
        } else {
            (view.to_screen(Pt::new(0.0, guide.at)).y - screen.y).abs()
        };
        if distance <= radius && best.is_none_or(|(_, so_far)| distance < so_far) {
            best = Some((index, distance));
        }
    }
    best.map(|(index, _)| index)
}

impl FoundryWindow {
    pub fn guide_family(&self) -> String {
        self.session
            .font()
            .map(|font| font.style.family.clone())
            .filter(|family| !family.is_empty())
            .unwrap_or_else(|| self.font_name.clone())
    }

    fn guides_for(&self, family: &str) -> &[Guide] {
        self.guides
            .by_family
            .get(family)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn hit_guide(&self, family: &str, view: &Viewport, screen: Pt) -> Option<usize> {
        nearest_guide(self.guides_for(family), view, screen, HIT)
    }

    fn add_guide(&mut self, family: &str, vertical: bool, at: f64) {
        let list = self.guides.by_family.entry(family.to_string()).or_default();
        list.push(Guide { vertical, at });
        self.selected_guide = Some(list.len() - 1);
        let axis = if vertical { "Vertical" } else { "Horizontal" };
        self.status = (
            format!(
                "{axis} guide at {at:.0}. It shows on every glyph in this family, and it is not saved in the font."
            ),
            Tone::Done,
        );
    }

    pub fn delete_selected_guide(&mut self) {
        let Some(index) = self.selected_guide else {
            return;
        };
        let family = self.guide_family();
        let Some(list) = self.guides.by_family.get_mut(&family) else {
            self.selected_guide = None;
            return;
        };
        if index < list.len() {
            list.remove(index);
        }
        self.selected_guide = None;
        self.status = ("Removed the guide.".into(), Tone::Quiet);
    }

    fn move_guide(&mut self, family: &str, index: usize, at: f64) {
        if let Some(guide) = self
            .guides
            .by_family
            .get_mut(family)
            .and_then(|list| list.get_mut(index))
        {
            guide.at = at;
            self.selected_guide = Some(index);
        }
    }

    /// Drag an existing guide. Returns true when this frame belonged to a guide.
    pub fn interact_guides(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        view: &Viewport,
    ) -> bool {
        if !self.settings.show_guides {
            return false;
        }
        let primary = egui::PointerButton::Primary;
        let family = self.guide_family();
        if response.drag_started_by(primary)
            && let Some(press) = ui.input(|input| input.pointer.press_origin())
            && let Some(index) = self.hit_guide(&family, view, pt(press))
        {
            self.selected_guide = Some(index);
            self.drag = Drag::GuideMove { index };
            return true;
        }
        if let Drag::GuideMove { index } = self.drag {
            if response.dragged_by(primary)
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let at = view.to_font(pt(pointer));
                let at = if self
                    .guides_for(&family)
                    .get(index)
                    .is_some_and(|guide| guide.vertical)
                {
                    at.x
                } else {
                    at.y
                };
                let at = if self.settings.snap { at.round() } else { at };
                self.move_guide(&family, index, at);
            }
            if response.drag_stopped_by(primary) {
                self.drag = Drag::None;
            }
            return true;
        }
        false
    }

    pub fn guide_tool(&mut self, ui: &egui::Ui, response: &egui::Response, view: &Viewport) {
        if self.interact_guides(ui, response, view) {
            return;
        }
        let primary = egui::PointerButton::Primary;
        if response.drag_started_by(primary)
            && let Some(press) = ui.input(|input| input.pointer.press_origin())
        {
            let at = pt(press);
            self.drag = Drag::GuideNew {
                origin: at,
                now: at,
            };
            self.selected_guide = None;
        }
        if response.dragged_by(primary)
            && let Some(pointer) = response.interact_pointer_pos()
            && let Drag::GuideNew { now, .. } = &mut self.drag
        {
            *now = pt(pointer);
        }
        if response.drag_stopped_by(primary)
            && let Drag::GuideNew { origin, now } = std::mem::replace(&mut self.drag, Drag::None)
        {
            let dx = now.x - origin.x;
            let dy = now.y - origin.y;
            if dx.hypot(dy) < 8.0 {
                self.status = (
                    "Drag across for a horizontal guide, or up and down for a vertical one.".into(),
                    Tone::Quiet,
                );
                return;
            }
            let vertical = dy.abs() > dx.abs();
            let font = view.to_font(now);
            let mut at = if vertical { font.x } else { font.y };
            if self.settings.snap {
                at = at.round();
            }
            let family = self.guide_family();
            self.add_guide(&family, vertical, at);
        }
    }

    pub fn paint_guides(&self, painter: &egui::Painter, view: &Viewport, rect: Rect) {
        if !self.settings.show_guides {
            return;
        }
        let family = self.guide_family();
        let font = egui::FontId::monospace(10.0);
        let (_paper, ink) = self.canvas_colors();
        let halo = Stroke::new(3.5, ink);
        for (index, guide) in self.guides_for(&family).iter().enumerate() {
            let selected = self.selected_guide == Some(index);
            let stroke = if selected {
                Stroke::new(2.0, color(FOCUS))
            } else {
                Stroke::new(1.75, color(GUIDE))
            };
            let label = if selected { color(FOCUS) } else { color(GUIDE) };
            if guide.vertical {
                let x = view.to_screen(Pt::new(guide.at, 0.0)).x as f32;
                painter.vline(x, rect.y_range(), halo);
                painter.vline(x, rect.y_range(), stroke);
                painter.text(
                    Pos2::new(x + 4.0, rect.top() + 4.0),
                    egui::Align2::LEFT_TOP,
                    format!("{:.0}", guide.at),
                    font.clone(),
                    label,
                );
            } else {
                let y = view.to_screen(Pt::new(0.0, guide.at)).y as f32;
                painter.hline(rect.x_range(), y, halo);
                painter.hline(rect.x_range(), y, stroke);
                painter.text(
                    Pos2::new(rect.left() + 6.0, y - 2.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{:.0}", guide.at),
                    font.clone(),
                    label,
                );
            }
        }
        if let Drag::GuideNew { origin, now } = self.drag {
            if (now.x - origin.x).hypot(now.y - origin.y) < 8.0 {
                return;
            }
            let vertical = (now.y - origin.y).abs() > (now.x - origin.x).abs();
            let stroke = Stroke::new(1.0, color(FOCUS));
            if vertical {
                painter.vline(now.x as f32, rect.y_range(), stroke);
            } else {
                painter.hline(rect.x_range(), now.y as f32, stroke);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guide_is_hit_along_its_line() {
        let guides = vec![
            Guide {
                vertical: false,
                at: 100.0,
            },
            Guide {
                vertical: true,
                at: 40.0,
            },
        ];
        let view = Viewport {
            scale: 1.0,
            origin: Pt::new(0.0, 200.0),
        };
        assert_eq!(
            nearest_guide(&guides, &view, Pt::new(50.0, 100.0), 6.0),
            Some(0)
        );
        assert_eq!(
            nearest_guide(&guides, &view, Pt::new(40.0, 10.0), 6.0),
            Some(1)
        );
        assert_eq!(
            nearest_guide(&guides, &view, Pt::new(50.0, 120.0), 6.0),
            None
        );
    }
}
