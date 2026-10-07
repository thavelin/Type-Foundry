//! Gravity UI icons, MIT, copyright 2022 Yandex. The SVGs live in `icons/` next to this
//! crate. They are drawn locally. The window does not fetch them.

use std::collections::HashMap;

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions, Vec2};

use crate::app::FoundryWindow;

const SCALE: f32 = 2.0;

const ICONS: &[(&str, &[u8])] = &[
    (
        "arrow-rotate-left",
        include_bytes!("../icons/arrow-rotate-left.svg"),
    ),
    (
        "arrow-rotate-right",
        include_bytes!("../icons/arrow-rotate-right.svg"),
    ),
    ("circle", include_bytes!("../icons/circle.svg")),
    ("guide", include_bytes!("../icons/guide.svg")),
    ("lasso", include_bytes!("../icons/lasso.svg")),
    ("layout-cells", include_bytes!("../icons/layout-cells.svg")),
    (
        "location-arrow",
        include_bytes!("../icons/location-arrow.svg"),
    ),
    ("magic-wand", include_bytes!("../icons/magic-wand.svg")),
    ("pencil", include_bytes!("../icons/pencil.svg")),
    (
        "pencil-to-square",
        include_bytes!("../icons/pencil-to-square.svg"),
    ),
    ("square", include_bytes!("../icons/square.svg")),
];

#[derive(Default)]
pub struct IconSet {
    textures: HashMap<&'static str, TextureHandle>,
}

impl IconSet {
    pub fn texture(&mut self, ctx: &egui::Context, name: &'static str) -> TextureHandle {
        if let Some(found) = self.textures.get(name) {
            return found.clone();
        }
        let image = rasterize(bytes(name))
            .unwrap_or_else(|| ColorImage::new([1, 1], vec![Color32::TRANSPARENT]));
        let texture = ctx.load_texture(format!("icon:{name}"), image, TextureOptions::LINEAR);
        self.textures.insert(name, texture.clone());
        texture
    }
}

fn bytes(name: &str) -> &'static [u8] {
    ICONS
        .iter()
        .find(|(icon, _)| *icon == name)
        .map(|(_, data)| *data)
        .unwrap_or(&[])
}

/// Rasterize one icon to white pixels so egui can tint it with the text color.
pub fn rasterize(svg: &[u8]) -> Option<ColorImage> {
    let text = std::str::from_utf8(svg)
        .ok()?
        .replace("currentColor", "#ffffff");
    let tree = resvg::usvg::Tree::from_str(&text, &resvg::usvg::Options::default()).ok()?;
    let width = (tree.size().width() * SCALE).ceil() as u32;
    let height = (tree.size().height() * SCALE).ceil() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(SCALE, SCALE),
        &mut pixmap.as_mut(),
    );
    let pixels = pixmap
        .pixels()
        .iter()
        .map(|pixel| {
            let color = pixel.demultiply();
            Color32::from_rgba_unmultiplied(color.red(), color.green(), color.blue(), color.alpha())
        })
        .collect();
    Some(ColorImage::new([width as usize, height as usize], pixels))
}

impl FoundryWindow {
    /// A tool button: Gravity UI icon, optional name. Longer help shows on hover.
    /// Pass an empty `label` for an icon-only toolbar control (hover keeps the name).
    pub fn icon_button(
        &mut self,
        ui: &mut egui::Ui,
        name: &'static str,
        label: &str,
        selected: bool,
        enabled: bool,
        hover: &str,
    ) -> bool {
        let texture = self.icons.texture(ui.ctx(), name);
        let image = egui::Image::from_texture(&texture).fit_to_exact_size(Vec2::splat(16.0));
        let button = if label.is_empty() {
            egui::Button::image(image)
        } else {
            egui::Button::image_and_text(image, label)
        };
        ui.add_enabled(
            enabled,
            button
                .selected(selected)
                .frame(true)
                .frame_when_inactive(selected)
                .image_tint_follows_text_color(true),
        )
        .on_hover_text(hover)
        .clicked()
    }

    /// A menu row with an icon and a label.
    pub fn icon_menu(
        &mut self,
        ui: &mut egui::Ui,
        name: &'static str,
        label: &str,
        selected: bool,
    ) -> bool {
        let texture = self.icons.texture(ui.ctx(), name);
        let image = egui::Image::from_texture(&texture).fit_to_exact_size(Vec2::splat(16.0));
        let clicked = ui
            .add(
                egui::Button::image_and_text(image, label)
                    .selected(selected)
                    .frame(false)
                    .image_tint_follows_text_color(true),
            )
            .clicked();
        if clicked {
            ui.close();
        }
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_toolbar_icon_rasterizes() {
        for (name, svg) in ICONS {
            let image = rasterize(svg).unwrap_or_else(|| panic!("{name} did not parse"));
            let ink = image.pixels.iter().filter(|pixel| pixel.a() > 16).count();
            assert!(ink > 20, "{name} rendered empty");
        }
    }
}
