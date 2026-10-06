//! The glyph editor canvas: tools, selection, and drawing. Edits go out as commands.

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};
use foundry_app::palette::{AMBER, BONE, FOCUS, MUTED, SIGNAL};
use foundry_app::{
    Bounds, Handle, Outline, Pt, Viewport, flatten, handles_in_polygon, handles_in_rect, hit_test,
    nearest_segment, scanline_spans,
};
use serde_json::json;

use crate::app::{Drag, FoundryWindow, Scope, Tone, Tool};
use crate::color;

const HIT_RADIUS: f64 = 9.0;
const CURVE_STEPS: usize = 24;
const FIT_MARGIN: f64 = 56.0;

impl FoundryWindow {
    pub fn canvas(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        let (paper, ink) = self.canvas_colors();
        painter.rect_filled(rect, CornerRadius::ZERO, paper);

        let Some(name) = self.current.clone() else {
            hint(
                &painter,
                rect,
                "Pick a glyph in the overview, or Glyph > New glyph.",
            );
            return;
        };
        let Some(outline) = self.current_outline() else {
            hint(&painter, rect, "This glyph could not be read.");
            return;
        };

        // Fit once per glyph. When the canvas moves or resizes (a panel opening, say), keep the
        // zoom and move the view with the canvas's top-left corner, so a panel growing on the
        // right moves nothing under the pointer.
        if self.view.is_none() {
            self.view = Some(self.fit(&outline, rect));
        } else if self.canvas_rect != rect
            && self.canvas_rect.is_positive()
            && let Some(view) = &mut self.view
        {
            let shift = rect.min - self.canvas_rect.min;
            view.pan(f64::from(shift.x), f64::from(shift.y));
        }
        self.canvas_rect = rect;
        let Some(mut view) = self.view else {
            return;
        };

        self.navigate(ui, &response, &mut view, &outline, rect);
        let pointer = response.hover_pos().map(pt);
        let alt = ui.input(|input| input.modifiers.alt);
        let shift = ui.input(|input| input.modifiers.shift);

        match self.tool {
            Tool::Select => self.select_tool(ui, &response, &view, &outline, &name, shift, alt),
            Tool::Lasso => self.lasso_tool(ui, &response, &view, &outline, shift),
            Tool::Pen => self.pen_tool(&response, &view, &outline, &name, shift),
            Tool::Rectangle | Tool::Oval => self.shape_tool(ui, &response, &view, &name),
            Tool::Guide => self.guide_tool(ui, &response, &view),
        }
        self.view = Some(view);

        // Paint from the session's current state, after this frame's edits.
        let outline = self.current_outline().unwrap_or(outline);
        let painter = painter.with_clip_rect(rect);
        if self.settings.onion_skin {
            self.paint_onion(&painter, &view, &outline);
        }
        if self.settings.metrics {
            self.paint_metrics(&painter, &view, rect, outline.advance);
        }
        if self.settings.fill {
            paint_fill(&painter, &view, rect, &outline, ink);
        }
        // Another style of the family, drawn over the fill so it shows on ink and paper alike.
        if let (Some(id), Some(name)) = (self.compare, self.current.clone())
            && let Some(other) = self.outline_in(id, &name)
        {
            paint_stroke(&painter, &view, &other, Stroke::new(1.5, color(MUTED)));
        }
        if self.settings.outline || !self.settings.fill {
            paint_stroke(&painter, &view, &outline, Stroke::new(1.25, ink));
        }
        if self.effects.open
            && let Some(ghost) = self.effects.preview(&outline, &self.selection)
        {
            paint_stroke(&painter, &view, &ghost, Stroke::new(1.5, color(AMBER)));
        }
        self.paint_shape_ghost(&painter, &view, rect);
        self.paint_guides(&painter, &view, rect);
        self.paint_handles(&painter, &view, &outline);
        self.paint_overlays(ui, &painter, &view, &outline, pointer, alt);
    }

    fn fit(&mut self, outline: &Outline, rect: Rect) -> Viewport {
        let mut bounds = outline.bounds(self.descender, self.ascender);
        if self.settings.onion_skin {
            let (left, right) = self.onion_edges(outline);
            bounds.min.x = bounds.min.x.min(left);
            bounds.max.x = bounds.max.x.max(right);
        }
        Viewport::fit(bounds_of(rect), bounds, FIT_MARGIN)
    }

    /// Editor paper and the fill drawn on it. White is the bone paper.
    pub(crate) fn canvas_colors(&self) -> (Color32, Color32) {
        if self.settings.dark_canvas {
            (Color32::BLACK, Color32::WHITE)
        } else {
            (color(BONE), Color32::BLACK)
        }
    }

    /// Font x of the previous glyph's origin, and of the far side of the next glyph.
    fn onion_edges(&mut self, outline: &Outline) -> (f64, f64) {
        let (previous, next) = self.neighbor_names();
        let left = previous
            .and_then(|name| self.outline(&name))
            .map(|other| -other.advance)
            .unwrap_or(0.0);
        let right = next
            .and_then(|name| self.outline(&name))
            .map(|other| outline.advance + other.advance)
            .unwrap_or(outline.advance);
        (left, right)
    }

    fn neighbor_names(&self) -> (Option<String>, Option<String>) {
        let Some(current) = &self.current else {
            return (None, None);
        };
        let Some(index) = self.glyphs.iter().position(|entry| &entry.name == current) else {
            return (None, None);
        };
        let previous = index.checked_sub(1).map(|at| self.glyphs[at].name.clone());
        let next = self.glyphs.get(index + 1).map(|entry| entry.name.clone());
        (previous, next)
    }

    fn paint_onion(&mut self, painter: &egui::Painter, view: &Viewport, outline: &Outline) {
        let (previous, next) = self.neighbor_names();
        let stroke = Stroke::new(1.5, color(MUTED));
        if let Some(name) = previous
            && let Some(other) = self.outline(&name)
        {
            paint_stroke(painter, &shifted(view, -other.advance), &other, stroke);
        }
        if let Some(name) = next
            && let Some(other) = self.outline(&name)
        {
            paint_stroke(painter, &shifted(view, outline.advance), &other, stroke);
        }
    }

    /// Zoom, pan, and refit.
    fn navigate(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        view: &mut Viewport,
        outline: &Outline,
        rect: Rect,
    ) {
        if response.hovered() {
            let (scroll, zoom, pointer) = ui.input(|input| {
                (
                    input.smooth_scroll_delta.y,
                    input.zoom_delta(),
                    input.pointer.hover_pos(),
                )
            });
            let factor = f64::from(zoom) * f64::from(scroll * 0.0025).exp();
            if let Some(pointer) = pointer
                && (factor - 1.0).abs() > f64::EPSILON
            {
                view.zoom_at(pt(pointer), factor);
            }
        }
        if response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
        {
            let delta = response.drag_delta();
            view.pan(f64::from(delta.x), f64::from(delta.y));
        }
        if response.double_clicked()
            && response
                .interact_pointer_pos()
                .is_some_and(|press| hit_test(outline, view, pt(press), HIT_RADIUS).is_none())
            && nearest_segment(
                outline,
                view,
                response
                    .interact_pointer_pos()
                    .map_or(Pt::new(0.0, 0.0), pt),
                HIT_RADIUS,
            )
            .is_none()
        {
            *view = self.fit(outline, rect);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn select_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        view: &Viewport,
        outline: &Outline,
        name: &str,
        shift: bool,
        alt: bool,
    ) {
        if self.interact_guides(ui, response, view) {
            return;
        }
        let primary = egui::PointerButton::Primary;
        // A drag begins past egui's threshold, so look at where the button went down.
        if response.drag_started_by(primary)
            && let Some(press) = ui.input(|input| input.pointer.press_origin())
        {
            match hit_test(outline, view, pt(press), HIT_RADIUS) {
                Some(handle) => {
                    if shift {
                        self.selection.insert(handle);
                    } else if !self.selection.contains(&handle) {
                        self.selection = [handle].into_iter().collect();
                    }
                    self.checkpoint();
                    self.drag = Drag::Points {
                        last: view.to_font(pt(press)),
                    };
                }
                None => {
                    self.drag = Drag::Marquee {
                        start: pt(press),
                        now: pt(press),
                        additive: shift,
                    };
                }
            }
        }

        if response.dragged_by(primary)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            match &mut self.drag {
                Drag::Points { last } => {
                    let target = view.to_font(pt(pointer));
                    let (mut dx, mut dy) = (target.x - last.x, target.y - last.y);
                    if self.settings.snap {
                        dx = dx.trunc();
                        dy = dy.trunc();
                    }
                    if dx != 0.0 || dy != 0.0 {
                        *last = Pt::new(last.x + dx, last.y + dy);
                        self.nudge(dx, dy);
                    }
                }
                Drag::Marquee { now, .. } => *now = pt(pointer),
                Drag::Shape { .. }
                | Drag::None
                | Drag::Lasso { .. }
                | Drag::GuideMove { .. }
                | Drag::GuideNew { .. } => {}
            }
        }

        if response.drag_stopped_by(primary) {
            match std::mem::replace(&mut self.drag, Drag::None) {
                Drag::Marquee {
                    start,
                    now,
                    additive,
                } => {
                    let picked = handles_in_rect(outline, view, start, now);
                    if !additive {
                        self.selection.clear();
                    }
                    self.selection.extend(picked);
                }
                Drag::Points { .. } => self.checkpoint(),
                Drag::Shape { .. }
                | Drag::None
                | Drag::Lasso { .. }
                | Drag::GuideMove { .. }
                | Drag::GuideNew { .. } => {}
            }
        }

        if response.clicked_by(primary)
            && let Some(press) = response.interact_pointer_pos()
        {
            let press = pt(press);
            if self.settings.show_guides {
                let family = self.guide_family();
                if let Some(index) = self.hit_guide(&family, view, press) {
                    self.selected_guide = Some(index);
                    return;
                }
            }
            match hit_test(outline, view, press, HIT_RADIUS) {
                Some(handle) if shift => {
                    if !self.selection.remove(&handle) {
                        self.selection.insert(handle);
                    }
                }
                Some(handle) => self.selection = [handle].into_iter().collect(),
                None if alt => {
                    if let Some(hit) = nearest_segment(outline, view, press, HIT_RADIUS) {
                        self.split(name, hit.contour, hit.end, hit.t);
                    }
                }
                None if !shift => {
                    self.selection.clear();
                    self.selected_guide = None;
                }
                None => {}
            }
        }

        if response.double_clicked()
            && let Some(press) = response.interact_pointer_pos()
            && let Some(handle) = hit_test(outline, view, pt(press), HIT_RADIUS)
            && let Some(point) = outline.point(handle)
            && point.on
        {
            self.edit_json(
                json!({
                    "op": "set_point", "name": name, "contour": handle.contour,
                    "point": handle.point, "smooth": !point.smooth,
                }),
                Scope::Glyph(name.to_string()),
            );
            let label = if point.smooth { "corner" } else { "smooth" };
            self.status = (format!("Point is now {label}"), Tone::Quiet);
        }
    }

    fn split(&mut self, name: &str, contour: usize, end: usize, t: f64) {
        if let Some(data) = self.edit_json(
            json!({ "op": "split_segment", "name": name, "contour": contour, "point": end, "t": t }),
            Scope::Glyph(name.to_string()),
        ) {
            let point = data["point"].as_u64().unwrap_or(0) as usize;
            self.selection = [Handle { contour, point }].into_iter().collect();
            self.status = ("Added a point".into(), Tone::Quiet);
        }
    }

    fn lasso_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        view: &Viewport,
        outline: &Outline,
        shift: bool,
    ) {
        let primary = egui::PointerButton::Primary;
        if response.drag_started_by(primary)
            && let Some(press) = ui.input(|input| input.pointer.press_origin())
        {
            self.drag = Drag::Lasso {
                points: vec![pt(press)],
                additive: shift,
            };
        }
        if response.dragged_by(primary)
            && let Some(pointer) = response.interact_pointer_pos()
            && let Drag::Lasso { points, .. } = &mut self.drag
        {
            let at = pt(pointer);
            let far = points
                .last()
                .is_none_or(|last| (last.x - at.x).hypot(last.y - at.y) >= 2.0);
            if far {
                points.push(at);
            }
        }
        if response.drag_stopped_by(primary)
            && let Drag::Lasso { points, additive } = std::mem::replace(&mut self.drag, Drag::None)
        {
            if points.len() < 3 {
                return;
            }
            let picked = handles_in_polygon(outline, view, &points);
            let count = picked.len();
            if !additive {
                self.selection.clear();
            }
            self.selection.extend(picked);
            self.status = (format!("{count} points in the lasso."), Tone::Quiet);
        }
    }

    fn pen_tool(
        &mut self,
        response: &egui::Response,
        view: &Viewport,
        outline: &Outline,
        name: &str,
        shift: bool,
    ) {
        if !response.clicked_by(egui::PointerButton::Primary) {
            return;
        }
        let Some(press) = response.interact_pointer_pos() else {
            return;
        };
        let mut at = view.to_font(pt(press));
        if self.settings.snap {
            at = Pt::new(at.x.round(), at.y.round());
        }
        let kind = if shift { "off" } else { "on" };

        if let Some(contour) = self.pen_contour {
            let length = outline.contours.get(contour).map_or(0, |c| c.points.len());
            let on_first = outline
                .point(Handle { contour, point: 0 })
                .is_some_and(|first| {
                    let at = view.to_screen(first.at);
                    (at.x - f64::from(press.x)).hypot(at.y - f64::from(press.y)) <= HIT_RADIUS
                });
            if length >= 2 && on_first {
                if self
                    .edit_json(
                        json!({ "op": "set_closed", "name": name, "contour": contour, "closed": true }),
                        Scope::Glyph(name.to_string()),
                    )
                    .is_some()
                {
                    self.pen_contour = None;
                    self.status = ("Closed the contour".into(), Tone::Done);
                }
                return;
            }
            if self
                .edit_json(
                    json!({
                        "op": "insert_point", "name": name, "contour": contour,
                        "index": length, "x": at.x, "y": at.y, "kind": kind,
                    }),
                    Scope::Glyph(name.to_string()),
                )
                .is_some()
            {
                self.selection = [Handle {
                    contour,
                    point: length,
                }]
                .into_iter()
                .collect();
            }
            return;
        }

        if let Some(data) = self.edit_json(
            json!({
                "op": "add_contour", "name": name,
                "contour": { "closed": false, "points": [
                    { "x": at.x, "y": at.y, "kind": "on", "smooth": false }
                ]},
            }),
            Scope::Glyph(name.to_string()),
        ) {
            let contour = data["contour"].as_u64().unwrap_or(0) as usize;
            self.pen_contour = Some(contour);
            self.selection = [Handle { contour, point: 0 }].into_iter().collect();
            self.status = (
                "New contour. Click to add points, click the first point to close, Esc to stop."
                    .into(),
                Tone::Quiet,
            );
        }
    }

    fn shape_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        view: &Viewport,
        name: &str,
    ) {
        let primary = egui::PointerButton::Primary;
        // A drag begins past egui's threshold, so the button-down position is the corner.
        if response.drag_started_by(primary)
            && let Some(press) = ui.input(|input| input.pointer.press_origin())
        {
            let at = self.snapped(view.to_font(pt(press)));
            self.drag = Drag::Shape { start: at, now: at };
            self.pen_contour = None;
        }
        if response.dragged_by(primary)
            && let Some(pointer) = response.interact_pointer_pos()
            && matches!(self.drag, Drag::Shape { .. })
        {
            let at = self.snapped(view.to_font(pt(pointer)));
            if let Drag::Shape { now, .. } = &mut self.drag {
                *now = at;
            }
        }
        if response.drag_stopped_by(primary)
            && let Drag::Shape { start, now } = std::mem::replace(&mut self.drag, Drag::None)
        {
            self.commit_shape(name, start, now);
        }
    }

    fn commit_shape(&mut self, name: &str, start: Pt, now: Pt) {
        let points = match self.tool {
            Tool::Rectangle => crate::shapes::rectangle(start, now),
            Tool::Oval => crate::shapes::oval(start, now),
            Tool::Select | Tool::Pen | Tool::Lasso | Tool::Guide => None,
        };
        let Some(points) = points else {
            self.status = ("The shape needs a little room.".into(), Tone::Quiet);
            return;
        };
        let label = match self.tool {
            Tool::Rectangle => "rectangle",
            Tool::Oval => "oval",
            Tool::Select | Tool::Pen | Tool::Lasso | Tool::Guide => "shape",
        };
        if let Some(data) = self.edit_json(
            json!({
                "op": "add_contour",
                "name": name,
                "contour": crate::shapes::contour_value(&points),
            }),
            Scope::Glyph(name.to_string()),
        ) {
            let contour = data["contour"].as_u64().unwrap_or(0) as usize;
            self.selection = (0..points.len())
                .map(|point| Handle { contour, point })
                .collect();
            self.pen_contour = None;
            self.status = (format!("Added a {label}."), Tone::Done);
        }
    }

    fn snapped(&self, point: Pt) -> Pt {
        if self.settings.snap {
            Pt::new(point.x.round(), point.y.round())
        } else {
            point
        }
    }

    fn paint_shape_ghost(&self, painter: &egui::Painter, view: &Viewport, rect: Rect) {
        let Drag::Shape { start, now } = self.drag else {
            return;
        };
        let points = match self.tool {
            Tool::Rectangle => crate::shapes::rectangle(start, now),
            Tool::Oval => crate::shapes::oval(start, now),
            Tool::Select | Tool::Pen | Tool::Lasso | Tool::Guide => None,
        };
        if let Some(points) = points {
            let ghost = crate::shapes::outline_of(&points);
            paint_fill(
                painter,
                view,
                rect,
                &ghost,
                color(AMBER).gamma_multiply(0.35),
            );
            paint_stroke(painter, view, &ghost, Stroke::new(1.5, color(AMBER)));
        } else if start.x != now.x || start.y != now.y {
            let area = Rect::from_two_pos(pos(view.to_screen(start)), pos(view.to_screen(now)));
            painter.rect_stroke(
                area,
                CornerRadius::ZERO,
                Stroke::new(1.0, color(AMBER)),
                egui::StrokeKind::Inside,
            );
        }
    }

    fn paint_metrics(&self, painter: &egui::Painter, view: &Viewport, rect: Rect, advance: f64) {
        let line = Stroke::new(1.0, color(MUTED));
        let faint = Stroke::new(1.0, color(MUTED).gamma_multiply(0.45));
        let font = egui::FontId::proportional(10.0);
        for (label, value, stroke) in [
            ("baseline", 0.0, line),
            ("ascender", self.ascender, faint),
            ("descender", self.descender, faint),
            ("x-height", self.x_height, faint),
            ("cap height", self.cap_height, faint),
        ] {
            let y = view.to_screen(Pt::new(0.0, value)).y as f32;
            painter.hline(rect.x_range(), y, stroke);
            painter.text(
                Pos2::new(rect.left() + 6.0, y - 2.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                font.clone(),
                color(MUTED),
            );
        }
        for x in [0.0, advance] {
            let x = view.to_screen(Pt::new(x, 0.0)).x as f32;
            painter.vline(x, rect.y_range(), faint);
        }
    }

    fn paint_handles(&self, painter: &egui::Painter, view: &Viewport, outline: &Outline) {
        let tether = Stroke::new(1.0, color(AMBER));
        for contour in &outline.contours {
            let count = contour.points.len();
            for (index, point) in contour.points.iter().enumerate() {
                if point.on {
                    continue;
                }
                for other in [(index + count - 1) % count, (index + 1) % count] {
                    let wraps =
                        (other == count - 1 && index == 0) || (other == 0 && index == count - 1);
                    if !contour.closed && wraps {
                        continue;
                    }
                    let neighbour = &contour.points[other];
                    if neighbour.on {
                        painter.line_segment(
                            [
                                pos(view.to_screen(point.at)),
                                pos(view.to_screen(neighbour.at)),
                            ],
                            tether,
                        );
                    }
                }
            }
        }
        let size = self.settings.handle_size;
        let rim = Stroke::new(1.0, Color32::BLACK);
        let number_font = egui::FontId::monospace(9.0);
        for (contour_index, contour) in outline.contours.iter().enumerate() {
            for (point_index, point) in contour.points.iter().enumerate() {
                let at = pos(view.to_screen(point.at));
                let handle = Handle {
                    contour: contour_index,
                    point: point_index,
                };
                let fill = if self.selection.contains(&handle) {
                    color(SIGNAL)
                } else if point.on {
                    color(FOCUS)
                } else {
                    color(AMBER)
                };
                if !point.on {
                    let half = Vec2::splat(size * 0.85);
                    painter.rect(
                        Rect::from_center_size(at, half * 2.0),
                        CornerRadius::ZERO,
                        fill,
                        rim,
                        egui::StrokeKind::Middle,
                    );
                } else if point.smooth {
                    painter.circle(at, size, fill, rim);
                } else {
                    // Corner points are diamonds, smooth points are circles.
                    let r = size * 1.15;
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            at + Vec2::new(0.0, -r),
                            at + Vec2::new(r, 0.0),
                            at + Vec2::new(0.0, r),
                            at + Vec2::new(-r, 0.0),
                        ],
                        fill,
                        rim,
                    ));
                }
                if point_index == 0 && contour.points.len() > 1 {
                    // Mark each contour's start point.
                    painter.circle_stroke(at, size + 3.0, Stroke::new(1.0, color(MUTED)));
                }
                if self.settings.point_numbers {
                    painter.text(
                        at + Vec2::new(size + 2.0, -size - 2.0),
                        egui::Align2::LEFT_BOTTOM,
                        point_index.to_string(),
                        number_font.clone(),
                        color(MUTED),
                    );
                }
            }
        }
    }

    fn paint_overlays(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        view: &Viewport,
        outline: &Outline,
        pointer: Option<Pt>,
        alt: bool,
    ) {
        if let Drag::Marquee { start, now, .. } = &self.drag {
            let area = Rect::from_two_pos(pos(*start), pos(*now));
            painter.rect(
                area,
                CornerRadius::ZERO,
                color(FOCUS).gamma_multiply(0.12),
                Stroke::new(1.0, color(FOCUS)),
                egui::StrokeKind::Inside,
            );
        }
        if let Drag::Lasso { points, .. } = &self.drag
            && points.len() >= 2
        {
            let mut line: Vec<Pos2> = points.iter().copied().map(pos).collect();
            if let Some(first) = line.first().copied() {
                line.push(first);
            }
            painter.add(egui::Shape::line(line, Stroke::new(1.0, color(FOCUS))));
        }
        let Some(pointer) = pointer else {
            return;
        };
        match self.tool {
            Tool::Select if alt => {
                if let Some(hit) = nearest_segment(outline, view, pointer, HIT_RADIUS) {
                    painter.circle(
                        pos(view.to_screen(hit.at)),
                        self.settings.handle_size,
                        color(SIGNAL).gamma_multiply(0.6),
                        Stroke::new(1.0, Color32::BLACK),
                    );
                }
            }
            Tool::Pen => {
                if let Some(contour) = self.pen_contour
                    && let Some(last) = outline
                        .contours
                        .get(contour)
                        .and_then(|found| found.points.last())
                {
                    painter.line_segment(
                        [pos(view.to_screen(last.at)), pos(pointer)],
                        Stroke::new(1.0, color(FOCUS)),
                    );
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            Tool::Rectangle | Tool::Oval | Tool::Guide => {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            Tool::Lasso => {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            Tool::Select => {
                if hit_test(outline, view, pointer, HIT_RADIUS).is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }
        }
        if self.settings.coordinates {
            let at = view.to_font(pointer);
            painter.text(
                pos(pointer) + Vec2::new(14.0, 14.0),
                egui::Align2::LEFT_TOP,
                format!("{:.0}, {:.0}", at.x, at.y),
                egui::FontId::monospace(10.0),
                color(MUTED),
            );
        }
    }
}

/// Fill closed contours with `ink`, one pixel row at a time, by the nonzero rule.
pub fn paint_fill(
    painter: &egui::Painter,
    view: &Viewport,
    clip: Rect,
    outline: &Outline,
    ink: Color32,
) {
    let polygons: Vec<Vec<Pt>> = outline
        .contours
        .iter()
        .filter(|contour| contour.closed)
        .map(|contour| {
            flatten(contour, CURVE_STEPS)
                .into_iter()
                .map(|point| view.to_screen(point))
                .collect()
        })
        .collect();
    let (mut top, mut bottom) = (f64::MAX, f64::MIN);
    for point in polygons.iter().flatten() {
        top = top.min(point.y);
        bottom = bottom.max(point.y);
    }
    let mut y = (top.max(f64::from(clip.top()))).floor() as f32;
    let end = bottom.min(f64::from(clip.bottom())) as f32;
    while y < end {
        for (left, right) in scanline_spans(&polygons, f64::from(y) + 0.5) {
            let span =
                Rect::from_min_max(Pos2::new(left as f32, y), Pos2::new(right as f32, y + 1.0));
            painter.rect_filled(span, CornerRadius::ZERO, ink);
        }
        y += 1.0;
    }
    for contour in outline.contours.iter().filter(|contour| !contour.closed) {
        let line: Vec<Pos2> = flatten(contour, CURVE_STEPS)
            .into_iter()
            .map(|point| pos(view.to_screen(point)))
            .collect();
        painter.add(egui::Shape::line(line, Stroke::new(1.5, ink)));
    }
}

fn paint_stroke(painter: &egui::Painter, view: &Viewport, outline: &Outline, stroke: Stroke) {
    for contour in &outline.contours {
        let mut line: Vec<Pos2> = flatten(contour, CURVE_STEPS)
            .into_iter()
            .map(|point| pos(view.to_screen(point)))
            .collect();
        if contour.closed
            && let Some(first) = line.first().copied()
        {
            line.push(first);
        }
        painter.add(egui::Shape::line(line, stroke));
    }
}

fn hint(painter: &egui::Painter, rect: Rect, text: &str) {
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(15.0),
        color(MUTED),
    );
}

fn bounds_of(rect: Rect) -> Bounds {
    Bounds {
        min: pt(rect.min),
        max: pt(rect.max),
    }
}

fn shifted(view: &Viewport, dx: f64) -> Viewport {
    let mut view = *view;
    view.origin.x += dx * view.scale;
    view
}

pub fn pt(pos: Pos2) -> Pt {
    Pt::new(f64::from(pos.x), f64::from(pos.y))
}

pub fn pos(point: Pt) -> Pos2 {
    Pos2::new(point.x as f32, point.y as f32)
}
