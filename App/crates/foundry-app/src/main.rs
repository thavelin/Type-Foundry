#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! The Type Foundry window. It reads and changes the font only through `Session::execute`.

mod align;
mod app;
mod canvas;
mod effects;
mod family_ui;
mod grid;
mod guides;
mod icons;
mod panels;
mod preview;
mod settings;
mod shapes;

use std::path::PathBuf;

use eframe::egui::{self, Color32, CornerRadius, Stroke};
use foundry_app::palette::{
    ALERT, AMBER, FOCUS, HAIRLINE, HAIRLINE_STRONG, INK, INVERSE, MUTED, PAGE, PANEL, RAISED,
};

pub const APP_TITLE: &str = "Type Foundry";
pub const LAST_DIR_KEY: &str = "last_dir";
pub const SETTINGS_KEY: &str = "settings";
pub const COPY_KEY: &str = "preview_copy";
pub const GUIDES_KEY: &str = "guides";

/// One of the named chrome colors.
pub fn color(hex: u32) -> Color32 {
    let [_, r, g, b] = hex.to_be_bytes();
    Color32::from_rgb(r, g, b)
}

fn main() -> eframe::Result {
    #[cfg(windows)]
    shortcut::ensure();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_TITLE)
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(|cc| {
            apply_chrome(&cc.egui_ctx);
            let last_dir = cc
                .storage
                .and_then(|storage| eframe::get_value::<String>(storage, LAST_DIR_KEY))
                .map(PathBuf::from);
            let settings = cc
                .storage
                .and_then(|storage| eframe::get_value::<settings::Settings>(storage, SETTINGS_KEY))
                .unwrap_or_default();
            let mut window = app::FoundryWindow::new(last_dir, settings);
            if let Some(copy) = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, COPY_KEY))
            {
                window.copy = copy;
            }
            if let Some(guides) = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, GUIDES_KEY))
            {
                window.guides = guides;
            }
            if let Some(path) = std::env::args().nth(1) {
                window.open(PathBuf::from(path));
            }
            Ok(Box::new(window))
        }),
    )
}

/// The mark from `assets/type-foundry-icon.png`, as straight RGBA.
fn app_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/type-foundry-icon.png");
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes.as_slice()));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().expect("the app icon is a png");
    let mut rgba = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgba).expect("the app icon decodes");
    rgba.truncate(info.buffer_size());
    if info.color_type == png::ColorType::Rgb {
        let rgb = rgba;
        rgba = Vec::with_capacity(rgb.len() / 3 * 4);
        for pixel in rgb.as_chunks::<3>().0 {
            rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
    }
    egui::IconData {
        rgba,
        width: info.width,
        height: info.height,
    }
}

fn apply_chrome(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.weak_text_color = Some(color(MUTED));
    visuals.hyperlink_color = color(FOCUS);
    visuals.panel_fill = color(PANEL);
    visuals.window_fill = color(PANEL);
    visuals.window_stroke = Stroke::new(1.0, color(HAIRLINE));
    visuals.extreme_bg_color = color(PAGE);
    visuals.faint_bg_color = color(RAISED);
    visuals.code_bg_color = color(RAISED);
    visuals.warn_fg_color = color(AMBER);
    visuals.error_fg_color = color(ALERT);
    visuals.selection.bg_fill = color(FOCUS);
    visuals.selection.stroke = Stroke::new(1.0, color(INVERSE));

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = color(PANEL);
    widgets.noninteractive.weak_bg_fill = color(PANEL);
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, color(HAIRLINE));
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, color(INK));
    for (state, fill, edge) in [
        (&mut widgets.inactive, RAISED, HAIRLINE),
        (&mut widgets.hovered, RAISED, HAIRLINE_STRONG),
        (&mut widgets.active, HAIRLINE, FOCUS),
        (&mut widgets.open, RAISED, HAIRLINE_STRONG),
    ] {
        state.bg_fill = color(fill);
        state.weak_bg_fill = color(fill);
        state.bg_stroke = Stroke::new(1.0, color(edge));
        state.fg_stroke = Stroke::new(1.0, color(INK));
        state.corner_radius = CornerRadius::same(3);
    }
    ctx.set_visuals(visuals);
}

/// The Start menu shortcut, created on the first launch of the window.
#[cfg(windows)]
mod shortcut {
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn ensure() {
        let Ok(target) = std::env::current_exe() else {
            return;
        };
        let Some(dir) = target.parent() else {
            return;
        };
        let Some(appdata) = std::env::var_os("APPDATA") else {
            return;
        };
        let link = PathBuf::from(appdata)
            .join(r"Microsoft\Windows\Start Menu\Programs")
            .join("Type Foundry.lnk");
        if link.exists() {
            return;
        }
        let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
        let script = format!(
            "$s = (New-Object -ComObject WScript.Shell).CreateShortcut({link}); \
             $s.TargetPath = {target}; \
             $s.WorkingDirectory = {dir}; \
             $s.Description = 'Type Foundry'; \
             $s.IconLocation = {target} + ',0'; \
             $s.Save()",
            link = quote(&link.to_string_lossy()),
            target = quote(&target.to_string_lossy()),
            dir = quote(&dir.to_string_lossy()),
        );
        let result = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        match result {
            Ok(status) if status.success() => {}
            Ok(status) => eprintln!("Start menu shortcut was not created: {status}"),
            Err(err) => eprintln!("Start menu shortcut was not created: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_app_icon_is_the_square_mark() {
        let icon = super::app_icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        let visible = icon.rgba.chunks(4).filter(|pixel| pixel[3] > 16).count();
        assert!(visible > 1000, "{visible}");
    }
}
