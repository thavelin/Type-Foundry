//! Window state, the menu bar, keyboard shortcuts, dialogs, and the bridge to the session.
//! Every change to the font is a `Command`; the window only keeps caches of what it read.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, Stroke};
use foundry_api::{Command, GlyphGroup, Response, Session};
use foundry_app::palette::{ALERT, HAIRLINE, MUTED, PANEL, SIGNAL};
use foundry_app::{Handle, Outline, Pt, Viewport};
use serde_json::{Value, json};

use crate::align::AlignTo;
use crate::effects::EffectsState;
use crate::guides::GuideBook;
use crate::icons::IconSet;
use crate::preview::{Block, starter_copy};
use crate::settings::{ReviewPlace, Settings};
use crate::{APP_TITLE, COPY_KEY, GUIDES_KEY, LAST_DIR_KEY, SETTINGS_KEY, color};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Overview,
    Editor,
    /// The review sheet, filling the main area.
    Review,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Pen,
    Rectangle,
    Oval,
    Lasso,
    Guide,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Quiet,
    Done,
    Failed,
}

/// What an edit may have changed, so the window knows which caches to drop.
pub enum Scope {
    /// One glyph's outline or advance.
    Glyph(String),
    /// Glyph names, order, or Unicode values, or anything after undo and redo.
    Structure,
    /// Font name or metrics.
    Font,
}

/// One open font, as the window lists it in the tab row.
#[derive(Clone)]
pub struct FontTab {
    pub id: u32,
    pub family: String,
    pub style: String,
    pub weight: u16,
    pub italic: bool,
    pub dirty: bool,
}

#[derive(Clone)]
pub struct GlyphEntry {
    pub name: String,
    pub unicode: Option<u32>,
    pub group: GlyphGroup,
}

pub enum Drag {
    None,
    /// Moving the selection. `last` is the font position already applied.
    Points {
        last: Pt,
    },
    Marquee {
        start: Pt,
        now: Pt,
        additive: bool,
    },
    /// A rectangle or oval drag, in font units. `start` is where the button went down.
    Shape {
        start: Pt,
        now: Pt,
    },
    /// A freeform loop, in screen points.
    Lasso {
        points: Vec<Pt>,
        additive: bool,
    },
    /// Dragging a guide that already exists.
    GuideMove {
        index: usize,
    },
    /// Drawing a new guide. `origin` and `now` are screen points.
    GuideNew {
        origin: Pt,
        now: Pt,
    },
}

pub struct NewFont {
    pub name: String,
    pub upm: u16,
}

pub struct NewGlyph {
    pub name: String,
    pub unicode: String,
    pub advance: f64,
}

#[derive(Default)]
pub struct Dialogs {
    pub new_font: Option<NewFont>,
    pub new_glyph: Option<NewGlyph>,
    pub settings: bool,
    pub shortcuts: bool,
    pub about: bool,
    pub new_style: Option<crate::family_ui::NewStyle>,
    pub export: Option<crate::family_ui::ExportForm>,
    pub report: Option<crate::family_ui::Report>,
    pub close_confirm: Option<u32>,
}

pub struct FoundryWindow {
    pub session: Session,
    /// The file each open font came from or was last saved to, by font id. Held here, not in
    /// the session.
    pub paths: HashMap<u32, PathBuf>,
    /// Open fonts, in tab order, as `fonts` last reported them.
    pub tabs: Vec<FontTab>,
    pub active: Option<u32>,
    /// A font drawn as a faint outline behind the editor, to compare styles.
    pub compare: Option<u32>,
    pub family_preview: bool,
    pub style: crate::family_ui::StyleFields,
    pub last_dir: Option<PathBuf>,
    pub font_name: String,
    pub upm: u16,
    pub ascender: f64,
    pub descender: f64,
    pub x_height: f64,
    pub cap_height: f64,
    pub glyphs: Vec<GlyphEntry>,
    /// Glyph lists of every open font, by font id, for the glyph tree. The active font's list is
    /// also in `glyphs`; both are rewritten by `refresh_index`.
    pub font_glyphs: HashMap<u32, Vec<GlyphEntry>>,
    pub by_unicode: HashMap<u32, String>,
    /// Outlines read from the session, by font id and glyph name.
    pub outlines: HashMap<(u32, String), Outline>,
    pub thumbs: HashMap<String, egui::TextureHandle>,
    pub mode: Mode,
    pub tool: Tool,
    pub current: Option<String>,
    pub selection: BTreeSet<Handle>,
    pub view: Option<Viewport>,
    pub canvas_rect: egui::Rect,
    pub drag: Drag,
    /// The open contour the pen is adding to.
    pub pen_contour: Option<usize>,
    pub settings: Settings,
    /// Lines for the person drawing. Not part of the font.
    pub guides: GuideBook,
    pub selected_guide: Option<usize>,
    pub effects: EffectsState,
    pub icons: IconSet,
    pub dialogs: Dialogs,
    /// Headline and paragraph blocks for the preview pane. View state, not part of the font.
    pub copy: Vec<Block>,
    pub filter: String,
    pub status: (String, Tone),
    shown_title: String,
    // Inspector text fields, kept between frames while they are being typed in.
    pub font_name_edit: String,
    pub glyph_name_edit: String,
    pub unicode_edit: String,
    pub edits_for: Option<String>,
    /// Last inspector focus bucket; when it changes, section open-state is reapplied for one frame.
    inspector_focus: InspectorFocus,
    /// When true, the next inspector draw forces CollapsingHeader open-state from the bools below.
    pub inspector_apply_open: bool,
    pub inspector_font_open: bool,
    pub inspector_style_open: bool,
    pub inspector_glyph_open: bool,
    pub inspector_selection_open: bool,
    pub inspector_genome_open: bool,
    /// Absolute units for `capture_genome` (default 4).
    pub genome_stem_tolerance: f64,
    /// Last `measure` / `capture_genome` payload shown in the inspector.
    pub genome_snapshot: Option<Value>,
    /// Last `audit` / `check_genome` payload shown in the inspector.
    pub audit_report: Option<Value>,
}

/// Which inspector sections should lead, driven by mode and selection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum InspectorFocus {
    OverviewEmpty,
    OverviewGlyph,
    EditorEmpty,
    EditorSelection,
}

impl FoundryWindow {
    pub fn new(last_dir: Option<PathBuf>, settings: Settings) -> Self {
        Self {
            session: Session::new(),
            paths: HashMap::new(),
            tabs: Vec::new(),
            active: None,
            compare: None,
            family_preview: false,
            style: crate::family_ui::StyleFields::default(),
            last_dir,
            font_name: String::new(),
            upm: 1000,
            ascender: 800.0,
            descender: -200.0,
            x_height: 500.0,
            cap_height: 700.0,
            glyphs: Vec::new(),
            font_glyphs: HashMap::new(),
            by_unicode: HashMap::new(),
            outlines: HashMap::new(),
            thumbs: HashMap::new(),
            mode: Mode::Overview,
            tool: Tool::Select,
            current: None,
            selection: BTreeSet::new(),
            view: None,
            canvas_rect: egui::Rect::NOTHING,
            drag: Drag::None,
            pen_contour: None,
            settings,
            guides: GuideBook::default(),
            selected_guide: None,
            effects: EffectsState::default(),
            icons: IconSet::default(),
            dialogs: Dialogs::default(),
            copy: starter_copy(),
            filter: String::new(),
            status: (
                "File > Open a font, File > Open SVG folder…, or File > New font.".to_string(),
                Tone::Quiet,
            ),
            shown_title: String::new(),
            font_name_edit: String::new(),
            glyph_name_edit: String::new(),
            unicode_edit: String::new(),
            edits_for: None,
            inspector_focus: InspectorFocus::OverviewEmpty,
            inspector_apply_open: true,
            inspector_font_open: true,
            inspector_style_open: true,
            inspector_glyph_open: false,
            inspector_selection_open: false,
            inspector_genome_open: true,
            genome_stem_tolerance: 4.0,
            genome_snapshot: None,
            audit_report: None,
        }
    }

    pub fn has_font(&self) -> bool {
        self.session.font().is_some()
    }

    /// Left glyph tree: Editor/Review/Split by default; Overview only when the override is on.
    pub fn glyph_list_visible(&self) -> bool {
        if !self.has_font() {
            return false;
        }
        self.settings.glyph_list_in_overview
            || self.settings.split_main
            || matches!(self.mode, Mode::Editor | Mode::Review)
    }

    /// Pyramid focus for the inspector. Reapplied only when the bucket changes.
    pub fn sync_inspector_focus(&mut self) {
        let focus = if self.mode == Mode::Editor
            || (self.settings.split_main && self.mode != Mode::Review)
        {
            if self.selection.is_empty() {
                InspectorFocus::EditorEmpty
            } else {
                InspectorFocus::EditorSelection
            }
        } else if self.current.is_some() {
            // Overview or Review with a glyph in focus: Glyph section joins Font/Style.
            InspectorFocus::OverviewGlyph
        } else {
            InspectorFocus::OverviewEmpty
        };
        if focus == self.inspector_focus {
            return;
        }
        self.inspector_focus = focus;
        self.inspector_apply_open = true;
        match focus {
            InspectorFocus::OverviewEmpty => {
                self.inspector_font_open = true;
                self.inspector_style_open = true;
                self.inspector_glyph_open = false;
                self.inspector_selection_open = false;
                self.inspector_genome_open = true;
            }
            InspectorFocus::OverviewGlyph => {
                self.inspector_font_open = true;
                self.inspector_style_open = true;
                self.inspector_glyph_open = true;
                self.inspector_selection_open = false;
                self.inspector_genome_open = true;
            }
            InspectorFocus::EditorEmpty => {
                self.inspector_font_open = false;
                self.inspector_style_open = false;
                self.inspector_glyph_open = true;
                self.inspector_selection_open = false;
                self.inspector_genome_open = false;
            }
            InspectorFocus::EditorSelection => {
                self.inspector_font_open = false;
                self.inspector_style_open = false;
                self.inspector_glyph_open = false;
                self.inspector_selection_open = true;
                self.inspector_genome_open = false;
            }
        }
    }

    // ---- The session bridge -------------------------------------------------------------

    /// Run a read-only command. Failures go to the status bar.
    pub fn run(&mut self, command: Command) -> Response {
        let response = self.session.execute(command);
        if !response.ok {
            let message = response
                .error
                .clone()
                .unwrap_or_else(|| "command failed".to_string());
            self.status = (message, Tone::Failed);
        }
        response
    }

    /// Run a command that changes the font, then drop what it made stale.
    pub fn edit(&mut self, command: Command, scope: Scope) -> Option<Value> {
        let response = self.run(command);
        if !response.ok {
            return None;
        }
        self.refresh_tabs();
        match scope {
            Scope::Glyph(name) => self.forget(&name),
            Scope::Structure => {
                self.refresh_index();
                self.refresh_info();
                self.forget_font();
                if self
                    .current
                    .as_ref()
                    .is_some_and(|name| !self.glyphs.iter().any(|entry| &entry.name == name))
                {
                    self.current = self.glyphs.first().map(|entry| entry.name.clone());
                    self.selection.clear();
                    self.view = None;
                }
                self.pen_contour = None;
                self.prune_selection();
                self.edits_for = None;
            }
            Scope::Font => {
                self.refresh_info();
                self.thumbs.clear();
            }
        }
        Some(response.data.unwrap_or(Value::Null))
    }

    /// Edit with a JSON command, for the many small edits the panels make.
    pub fn edit_json(&mut self, command: Value, scope: Scope) -> Option<Value> {
        match serde_json::from_value::<Command>(command) {
            Ok(command) => self.edit(command, scope),
            Err(err) => {
                self.status = (format!("internal command error: {err}"), Tone::Failed);
                None
            }
        }
    }

    pub fn checkpoint(&mut self) {
        self.session.execute(Command::Checkpoint);
    }

    fn forget(&mut self, name: &str) {
        if let Some(id) = self.active {
            self.outlines.remove(&(id, name.to_string()));
        }
        self.thumbs.remove(name);
        self.prune_selection();
    }

    fn prune_selection(&mut self) {
        let Some(outline) = self.current_outline() else {
            self.selection.clear();
            return;
        };
        self.selection
            .retain(|handle| outline.point(*handle).is_some());
        if let Some(contour) = self.pen_contour
            && outline
                .contours
                .get(contour)
                .is_none_or(|found| found.closed)
        {
            self.pen_contour = None;
        }
    }

    pub fn refresh_info(&mut self) {
        let response = self.session.execute(Command::Info);
        let Some(info) = response.data else {
            return;
        };
        self.font_name = info["name"].as_str().unwrap_or_default().to_string();
        self.upm = info["upm"]
            .as_u64()
            .and_then(|upm| u16::try_from(upm).ok())
            .unwrap_or(1000);
        let metric = |key: &str, fallback: f64| info["metrics"][key].as_f64().unwrap_or(fallback);
        self.ascender = metric("ascender", 800.0);
        self.descender = metric("descender", -200.0);
        self.x_height = metric("x_height", 500.0);
        self.cap_height = metric("cap_height", 700.0);
        self.font_name_edit = self.font_name.clone();
    }

    pub fn refresh_index(&mut self) {
        let response = self.session.execute(Command::Index { font: None });
        let entries = response
            .data
            .as_ref()
            .and_then(|data| data["glyphs"].as_array().cloned())
            .unwrap_or_default();
        self.glyphs = glyph_entries(&entries);
        self.by_unicode = self
            .glyphs
            .iter()
            .filter_map(|entry| entry.unicode.map(|code| (code, entry.name.clone())))
            .collect();
        if let Some(id) = self.active {
            self.font_glyphs.insert(id, self.glyphs.clone());
        }
    }

    /// The glyph list of an open font, read once and kept until the font is closed or changed.
    /// The glyph tree uses this for fonts other than the active one.
    pub fn font_glyphs_of(&mut self, id: u32) -> Vec<GlyphEntry> {
        if !self.font_glyphs.contains_key(&id) {
            let response = self.session.execute(Command::Index { font: Some(id) });
            let entries = response
                .data
                .as_ref()
                .and_then(|data| data["glyphs"].as_array().cloned())
                .unwrap_or_default();
            self.font_glyphs.insert(id, glyph_entries(&entries));
        }
        self.font_glyphs[&id].clone()
    }

    /// The outline of a glyph in the active font, read through the `glyph` command and cached.
    pub fn outline(&mut self, name: &str) -> Option<Outline> {
        let id = self.active?;
        self.outline_in(id, name)
    }

    /// The outline of a glyph in any open font.
    pub fn outline_in(&mut self, id: u32, name: &str) -> Option<Outline> {
        let key = (id, name.to_string());
        if let Some(found) = self.outlines.get(&key) {
            return Some(found.clone());
        }
        let response = self.session.execute(Command::Glyph {
            name: name.to_string(),
            font: Some(id),
        });
        let outline = response.data.as_ref().and_then(Outline::from_json)?;
        self.outlines.insert(key, outline.clone());
        Some(outline)
    }

    /// Drop everything cached about the active font.
    fn forget_font(&mut self) {
        if let Some(id) = self.active {
            self.outlines.retain(|(font, _), _| *font != id);
        }
        self.thumbs.clear();
        self.genome_snapshot = None;
        self.audit_report = None;
    }

    /// Live measurements into the inspector Genome section.
    pub fn measure_genome(&mut self) {
        let response = self.run(Command::Measure);
        if response.ok {
            self.genome_snapshot = response.data;
            self.inspector_genome_open = true;
            self.inspector_apply_open = true;
            let stem = self.genome_snapshot.as_ref().and_then(|data| {
                data["primary_stem"]
                    .as_f64()
                    .map(|stem| format!("{stem:.1}"))
            });
            self.status = (
                match stem {
                    Some(stem) => format!("Measured · primary stem {stem}"),
                    None => "Measured · no primary stem on sample glyphs".into(),
                },
                Tone::Done,
            );
        }
    }

    /// Capture Style Genome onto the open font (`font.lib`).
    pub fn capture_genome(&mut self) {
        let tolerance = self.genome_stem_tolerance;
        if let Some(data) = self.edit(
            Command::CaptureGenome {
                stem_tolerance: tolerance,
            },
            Scope::Font,
        ) {
            self.genome_snapshot = Some(data);
            self.inspector_genome_open = true;
            self.inspector_apply_open = true;
            self.status = ("Style Genome captured".into(), Tone::Done);
        }
    }

    /// Design-only genome check into the inspector.
    pub fn check_genome(&mut self) {
        let response = self.run(Command::CheckGenome);
        if response.ok {
            self.audit_report = response.data.map(|data| {
                json!({
                    "design": {
                        "issues": data["issues"],
                        "count": data["count"],
                    },
                    "count": data["count"],
                })
            });
            self.inspector_genome_open = true;
            self.inspector_apply_open = true;
            let count = self
                .audit_report
                .as_ref()
                .and_then(|data| data["count"].as_u64())
                .unwrap_or(0);
            self.status = (
                if count == 0 {
                    "Genome check · no design issues".into()
                } else {
                    format!("Genome check · {count} issue(s)")
                },
                if count == 0 {
                    Tone::Done
                } else {
                    Tone::Quiet
                },
            );
        }
    }

    /// Technical + design Foundry Audit into the inspector.
    pub fn run_audit(&mut self) {
        let response = self.run(Command::Audit { min_gap: 0.0 });
        if response.ok {
            self.audit_report = response.data;
            self.inspector_genome_open = true;
            self.inspector_apply_open = true;
            let count = self
                .audit_report
                .as_ref()
                .and_then(|data| data["count"].as_u64())
                .unwrap_or(0);
            self.status = (
                if count == 0 {
                    "Audit clean".into()
                } else {
                    format!("Audit · {count} issue(s)")
                },
                if count == 0 {
                    Tone::Done
                } else {
                    Tone::Quiet
                },
            );
        }
    }

    pub fn refresh_tabs(&mut self) {
        let response = self.session.execute(Command::Fonts);
        let Some(data) = response.data else {
            return;
        };
        let open: Vec<u64> = data["fonts"]
            .as_array()
            .map(|fonts| {
                fonts
                    .iter()
                    .filter_map(|font| font["id"].as_u64())
                    .collect()
            })
            .unwrap_or_default();
        self.font_glyphs
            .retain(|id, _| open.contains(&u64::from(*id)));
        self.tabs = data["fonts"]
            .as_array()
            .map(|fonts| {
                fonts
                    .iter()
                    .filter_map(|font| {
                        Some(FontTab {
                            id: u32::try_from(font["id"].as_u64()?).ok()?,
                            family: font["family"].as_str().unwrap_or_default().to_string(),
                            style: font["style"].as_str().unwrap_or_default().to_string(),
                            weight: font["weight"]
                                .as_u64()
                                .and_then(|weight| u16::try_from(weight).ok())
                                .unwrap_or(400),
                            italic: font["italic"].as_bool().unwrap_or(false),
                            dirty: font["dirty"].as_bool().unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.active = data["active"]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok());
        if self
            .compare
            .is_some_and(|id| Some(id) == self.active || !self.tabs.iter().any(|tab| tab.id == id))
        {
            self.compare = None;
        }
    }

    pub fn dirty(&self) -> bool {
        self.tabs
            .iter()
            .any(|tab| Some(tab.id) == self.active && tab.dirty)
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        self.active.and_then(|id| self.paths.get(&id).cloned())
    }

    pub fn current_outline(&mut self) -> Option<Outline> {
        let name = self.current.clone()?;
        self.outline(&name)
    }

    pub fn select_glyph(&mut self, name: Option<String>) {
        if self.current != name {
            self.selection.clear();
            self.pen_contour = None;
            self.view = None;
            self.drag = Drag::None;
        }
        self.current = name;
    }

    pub fn open_editor(&mut self, name: String) {
        self.select_glyph(Some(name));
        self.mode = Mode::Editor;
    }

    /// Switch tools. Drawing tools open the editor when a glyph is selected.
    pub fn choose_tool(&mut self, tool: Tool) {
        if self.tool != tool {
            self.pen_contour = None;
            if matches!(
                self.drag,
                Drag::Marquee { .. }
                    | Drag::Shape { .. }
                    | Drag::Lasso { .. }
                    | Drag::GuideNew { .. }
                    | Drag::GuideMove { .. }
            ) {
                self.drag = Drag::None;
            }
            self.tool = tool;
            if tool == Tool::Guide {
                self.status = (
                    "Drag across for a horizontal guide, or up and down for a vertical one. Guides show on every glyph and are not saved in the font.".into(),
                    Tone::Quiet,
                );
            }
            if tool == Tool::Lasso {
                self.status = (
                    "Drag a loop around the points you want. Shift adds them to the selection."
                        .into(),
                    Tone::Quiet,
                );
            }
        }
        if matches!(
            tool,
            Tool::Pen | Tool::Rectangle | Tool::Oval | Tool::Lasso | Tool::Guide
        ) {
            if self.current.is_some() {
                self.mode = Mode::Editor;
            } else {
                self.status = ("Select a glyph, then draw.".into(), Tone::Quiet);
            }
        }
    }

    pub fn step_glyph(&mut self, by: isize) {
        if self.glyphs.is_empty() {
            return;
        }
        let index = self
            .current
            .as_ref()
            .and_then(|name| self.glyphs.iter().position(|entry| &entry.name == name))
            .unwrap_or(0) as isize;
        let next = (index + by).rem_euclid(self.glyphs.len() as isize) as usize;
        let name = self.glyphs[next].name.clone();
        self.select_glyph(Some(name));
    }

    // ---- Files ------------------------------------------------------------------------

    /// Read a newly opened or created font and show its overview.
    pub fn load_new_font(&mut self) {
        self.font_switched();
        self.current = self.glyphs.first().map(|entry| entry.name.clone());
        self.view = None;
        self.mode = Mode::Overview;
    }

    /// Re-read everything after the active font changed. The same glyph stays selected when
    /// the new font has it, so flipping between Regular and Italic keeps your place.
    pub fn font_switched(&mut self) {
        self.refresh_tabs();
        self.refresh_info();
        self.refresh_index();
        self.thumbs.clear();
        self.selection.clear();
        self.pen_contour = None;
        self.drag = Drag::None;
        self.edits_for = None;
        self.style.loaded_for = None;
        self.audit_report = None;
        self.genome_snapshot = self.read_stored_genome();
        let keep = self
            .current
            .as_ref()
            .is_some_and(|name| self.glyphs.iter().any(|entry| &entry.name == name));
        if !keep {
            self.current = self.glyphs.first().map(|entry| entry.name.clone());
            self.view = None;
        }
    }

    /// Pull a previously captured Style Genome from the open font's `lib`, if any.
    fn read_stored_genome(&mut self) -> Option<Value> {
        let response = self.session.execute(Command::GetGenome);
        if !response.ok {
            return None;
        }
        response.data.filter(|data| !data.is_null())
    }

    pub fn switch_to(&mut self, id: u32) {
        if Some(id) == self.active {
            return;
        }
        if self.run(Command::SelectFont { id }).ok {
            self.font_switched();
        }
    }

    pub fn step_font(&mut self, by: isize) {
        if self.tabs.len() < 2 {
            return;
        }
        let index = self
            .tabs
            .iter()
            .position(|tab| Some(tab.id) == self.active)
            .unwrap_or(0) as isize;
        let next = (index + by).rem_euclid(self.tabs.len() as isize) as usize;
        let id = self.tabs[next].id;
        self.switch_to(id);
    }

    pub fn open(&mut self, path: PathBuf) {
        let response = self.run(Command::Open {
            path: path.to_string_lossy().into_owned(),
        });
        if !response.ok {
            return;
        }
        self.remember_dir(&path);
        self.settings.remember_recent(&path);
        if let Some(id) = response.data.as_ref().and_then(|data| data["id"].as_u64()) {
            self.paths.insert(id as u32, path.clone());
        }
        self.load_new_font();
        self.status = (
            format!("Opened {} · {} glyphs", path.display(), self.glyphs.len()),
            Tone::Done,
        );
    }

    pub fn create(&mut self, name: String, upm: u16) {
        let response = self.run(Command::Create { name, upm });
        if response.ok {
            self.load_new_font();
            self.status = (
                "New font. Glyph > New glyph adds the first one.".to_string(),
                Tone::Done,
            );
        }
    }

    pub fn save_to(&mut self, path: PathBuf) {
        let response = self.run(Command::Save {
            path: path.to_string_lossy().into_owned(),
            force: true,
        });
        if response.ok {
            self.remember_dir(&path);
            self.settings.remember_recent(&path);
            self.status = (format!("Saved {}", path.display()), Tone::Done);
            if let Some(id) = self.active {
                self.paths.insert(id, path);
            }
            self.refresh_tabs();
        }
    }

    pub fn save(&mut self) {
        match self.current_path() {
            Some(path) if is_writable(&path) => self.save_to(path),
            _ => self.save_as_dialog(),
        }
    }

    fn remember_dir(&mut self, path: &Path) {
        let dir = if path.is_dir() {
            Some(path.to_path_buf())
        } else {
            path.parent()
                .filter(|dir| !dir.as_os_str().is_empty())
                .map(Path::to_path_buf)
        };
        if let Some(dir) = dir {
            self.last_dir = Some(dir);
        }
    }

    fn dialog(&self) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new();
        match &self.last_dir {
            Some(dir) if dir.is_dir() => dialog.set_directory(dir),
            _ => dialog,
        }
    }

    pub fn open_file_dialog(&mut self) {
        if let Some(path) = self
            .dialog()
            .add_filter(
                "All fonts",
                &[
                    "json", "js", "ttf", "otf", "ttc", "otc", "woff", "woff2", "eot",
                ],
            )
            .add_filter("Type Foundry or typeface JSON", &["json", "js"])
            .add_filter("TrueType or OpenType", &["ttf", "otf", "ttc", "otc"])
            .add_filter("Web fonts", &["woff", "woff2", "eot"])
            .pick_file()
        {
            self.open(path);
        }
    }

    pub fn open_ufo_dialog(&mut self) {
        if let Some(path) = self.dialog().pick_folder() {
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ufo"))
            {
                self.open(path);
            } else {
                self.status = (
                    format!("{} is not a .ufo folder", path.display()),
                    Tone::Failed,
                );
            }
        }
    }

    pub fn open_svg_dialog(&mut self) {
        if let Some(path) = self.dialog().pick_folder() {
            self.open(path);
        }
    }

    pub fn save_as_dialog(&mut self) {
        let stem = self
            .current_path()
            .and_then(|path| {
                path.file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .or_else(|| self.session.font().map(|font| font.file_stem()))
            .map_or_else(
                || "Untitled.json".to_string(),
                |stem| format!("{stem}.json"),
            );
        if let Some(path) = self
            .dialog()
            .set_file_name(stem)
            .add_filter("Type Foundry font", &["json"])
            .add_filter("UFO", &["ufo"])
            .add_filter("TrueType", &["ttf"])
            .save_file()
        {
            self.save_to(path);
        }
    }

    // ---- Shared edits used by menus, shortcuts, and panels -----------------------------

    pub fn undo(&mut self) {
        if self.edit(Command::Undo, Scope::Structure).is_some() {
            self.status = ("Undone".into(), Tone::Quiet);
        }
    }

    pub fn redo(&mut self) {
        if self.edit(Command::Redo, Scope::Structure).is_some() {
            self.status = ("Redone".into(), Tone::Quiet);
        }
    }

    pub fn select_all_points(&mut self) {
        if let Some(outline) = self.current_outline() {
            self.selection = outline.handles().into_iter().collect();
        }
    }

    pub fn delete_selection(&mut self) {
        let Some(name) = self.current.clone() else {
            return;
        };
        if self.selection.is_empty() {
            return;
        }
        let points: Vec<[usize; 2]> = self
            .selection
            .iter()
            .map(|handle| [handle.contour, handle.point])
            .collect();
        let count = points.len();
        if self
            .edit_json(
                json!({ "op": "delete_points", "name": name, "points": points }),
                Scope::Glyph(name.clone()),
            )
            .is_some()
        {
            self.selection.clear();
            self.pen_contour = None;
            self.status = (format!("Deleted {count} points"), Tone::Quiet);
        }
    }

    pub fn nudge(&mut self, dx: f64, dy: f64) {
        let Some(name) = self.current.clone() else {
            return;
        };
        if self.selection.is_empty() {
            return;
        }
        let points: Vec<[usize; 2]> = self
            .selection
            .iter()
            .map(|handle| [handle.contour, handle.point])
            .collect();
        self.edit_json(
            json!({ "op": "move_points", "name": name, "points": points, "dx": dx, "dy": dy }),
            Scope::Glyph(name),
        );
    }

    /// Set the smooth flag on every selected on-curve point.
    pub fn set_smooth(&mut self, smooth: bool) {
        let Some(name) = self.current.clone() else {
            return;
        };
        let Some(outline) = self.current_outline() else {
            return;
        };
        let targets: Vec<Handle> = self
            .selection
            .iter()
            .copied()
            .filter(|handle| outline.point(*handle).is_some_and(|point| point.on))
            .collect();
        for handle in targets {
            self.edit_json(
                json!({
                    "op": "set_point", "name": name, "contour": handle.contour,
                    "point": handle.point, "smooth": smooth,
                }),
                Scope::Glyph(name.clone()),
            );
        }
    }

    pub fn set_kind(&mut self, on: bool) {
        let Some(name) = self.current.clone() else {
            return;
        };
        let targets: Vec<Handle> = self.selection.iter().copied().collect();
        for handle in targets {
            self.edit_json(
                json!({
                    "op": "set_point", "name": name, "contour": handle.contour,
                    "point": handle.point, "kind": if on { "on" } else { "off" },
                }),
                Scope::Glyph(name.clone()),
            );
        }
    }

    /// Reverse the contours that hold a selected point, or every contour with none selected.
    pub fn reverse_contours(&mut self) {
        let Some(name) = self.current.clone() else {
            return;
        };
        let Some(outline) = self.current_outline() else {
            return;
        };
        let contours: BTreeSet<usize> = if self.selection.is_empty() {
            (0..outline.contours.len()).collect()
        } else {
            self.selection.iter().map(|handle| handle.contour).collect()
        };
        for contour in contours {
            self.edit_json(
                json!({ "op": "reverse_contour", "name": name, "contour": contour }),
                Scope::Glyph(name.clone()),
            );
        }
        self.selection.clear();
    }

    pub fn round_glyphs(&mut self, all: bool) {
        let names = if all {
            Value::Null
        } else {
            match &self.current {
                Some(name) => json!([name]),
                None => return,
            }
        };
        if let Some(data) = self.edit_json(
            json!({ "op": "round_coordinates", "names": names }),
            Scope::Structure,
        ) {
            let count = data["glyphs"].as_array().map_or(0, Vec::len);
            self.status = (format!("Rounded {count} glyphs"), Tone::Done);
        }
    }

    pub fn delete_current_glyph(&mut self) {
        let Some(name) = self.current.clone() else {
            return;
        };
        if self
            .edit_json(
                json!({ "op": "delete_glyph", "name": name }),
                Scope::Structure,
            )
            .is_some()
        {
            self.status = (format!("Deleted glyph {name}"), Tone::Quiet);
        }
    }

    pub fn zoom(&mut self, factor: f64) {
        if let Some(view) = &mut self.view {
            let center = self.canvas_rect.center();
            view.zoom_at(Pt::new(f64::from(center.x), f64::from(center.y)), factor);
        }
    }

    // ---- Menus --------------------------------------------------------------------------

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let has_font = self.has_font();
        let has_glyph = self.current.is_some();
        let has_selection = !self.selection.is_empty();
        let (undo, redo) = self.session.history();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if item(ui, "New font…", "Ctrl+N", true) {
                    self.dialogs.new_font = Some(NewFont {
                        name: "Untitled".into(),
                        upm: 1000,
                    });
                }
                ui.menu_button("Open", |ui| {
                    if item(ui, "Open…", "Ctrl+O", true) {
                        self.open_file_dialog();
                    }
                    if item(ui, "Open UFO folder…", "", true) {
                        self.open_ufo_dialog();
                    }
                    if item(ui, "Open SVG folder…", "", true) {
                        self.open_svg_dialog();
                    }
                    if item(ui, "Open family…", "", true) {
                        self.open_family_dialog();
                    }
                    ui.menu_button("Open recent", |ui| {
                        let recent = self.settings.recent.clone();
                        if recent.is_empty() {
                            ui.add_enabled(false, egui::Label::new("No recent files"));
                        }
                        for path in &recent {
                            let label = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.display().to_string());
                            let exists = path.exists();
                            if item(ui, &label, "", exists) {
                                self.open(path.clone());
                            }
                        }
                        if !recent.is_empty() {
                            ui.separator();
                            if item(ui, "Clear list", "", true) {
                                self.settings.recent.clear();
                            }
                        }
                    });
                });
                ui.separator();
                if item(ui, "Save", "Ctrl+S", has_font) {
                    self.save();
                }
                if item(ui, "Save As…", "Ctrl+Shift+S", has_font) {
                    self.save_as_dialog();
                }
                if item(ui, "Close font", "Ctrl+W", has_font)
                    && let Some(id) = self.active
                {
                    self.request_close(id);
                }
                ui.separator();
                ui.menu_button("Family", |ui| {
                    if item(ui, "Make italic…", "Ctrl+Shift+I", has_font) {
                        self.open_italic(None);
                    }
                    if item(ui, "New style from this font…", "Ctrl+Shift+D", has_font) {
                        self.open_new_style();
                    }
                    if item(ui, "Save family…", "", has_font) {
                        self.save_family_dialog();
                    }
                    if item(ui, "Export family…", "", has_font) {
                        self.dialogs.export = Some(crate::family_ui::ExportForm { format: "ttf" });
                    }
                    if item(ui, "Check family", "", has_font) {
                        self.family_check();
                    }
                });
                ui.menu_button("Style Genome", |ui| {
                    if item(ui, "Measure…", "", has_font) {
                        self.measure_genome();
                    }
                    if item(ui, "Capture genome", "", has_font) {
                        self.capture_genome();
                    }
                    if item(ui, "Check genome", "", has_font) {
                        self.check_genome();
                    }
                    if item(ui, "Foundry Audit…", "", has_font) {
                        self.run_audit();
                    }
                });
                ui.separator();
                if item(ui, "Quit", "Ctrl+Q", true) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if item(ui, &format!("Undo ({undo})"), "Ctrl+Z", undo > 0) {
                    self.undo();
                }
                if item(ui, &format!("Redo ({redo})"), "Ctrl+Shift+Z", redo > 0) {
                    self.redo();
                }
                ui.separator();
                if item(ui, "Select all points", "Ctrl+A", has_glyph) {
                    self.select_all_points();
                }
                if item(ui, "Deselect", "Esc", has_selection) {
                    self.selection.clear();
                }
                if item(ui, "Delete points", "Del", has_selection) {
                    self.delete_selection();
                }
                ui.separator();
                ui.menu_button("Point", |ui| {
                    if item(ui, "Make smooth", "", has_selection) {
                        self.set_smooth(true);
                    }
                    if item(ui, "Make corner", "", has_selection) {
                        self.set_smooth(false);
                    }
                    if item(ui, "Make on-curve", "", has_selection) {
                        self.set_kind(true);
                    }
                    if item(ui, "Make off-curve", "", has_selection) {
                        self.set_kind(false);
                    }
                });
                if item(ui, "Reverse contour direction", "", has_glyph) {
                    self.reverse_contours();
                }
                ui.separator();
                ui.menu_button("Align points", |ui| {
                    let ready = self.selection.len() >= 2;
                    let spread = self.selection.len() >= 3;
                    if item(ui, "Left", "", ready) {
                        self.align_selection(AlignTo::Left);
                    }
                    if item(ui, "Center", "", ready) {
                        self.align_selection(AlignTo::CenterX);
                    }
                    if item(ui, "Right", "", ready) {
                        self.align_selection(AlignTo::Right);
                    }
                    if item(ui, "Top", "", ready) {
                        self.align_selection(AlignTo::Top);
                    }
                    if item(ui, "Middle", "", ready) {
                        self.align_selection(AlignTo::Middle);
                    }
                    if item(ui, "Bottom", "", ready) {
                        self.align_selection(AlignTo::Bottom);
                    }
                    ui.separator();
                    if item(ui, "Distribute horizontally", "", spread) {
                        self.align_selection(AlignTo::DistributeX);
                    }
                    if item(ui, "Distribute vertically", "", spread) {
                        self.align_selection(AlignTo::DistributeY);
                    }
                });
            });
            ui.menu_button("View", |ui| {
                ui.menu_button("Mode", |ui| {
                    if item(ui, "Font overview", "Ctrl+1", has_font) {
                        self.mode = Mode::Overview;
                        self.settings.split_main = false;
                    }
                    if item(ui, "Glyph editor", "Ctrl+2", has_glyph) {
                        self.mode = Mode::Editor;
                    }
                    if item(ui, "Overview and editor", "", has_glyph) {
                        self.settings.split_main = true;
                        self.mode = Mode::Editor;
                    }
                    if item(ui, "Review sheet pane", "Ctrl+3", has_font) {
                        self.mode = Mode::Review;
                    }
                });
                ui.menu_button("Canvas", |ui| {
                    if item(ui, "Zoom in", "Ctrl+=", has_glyph) {
                        self.zoom(1.25);
                    }
                    if item(ui, "Zoom out", "Ctrl+-", has_glyph) {
                        self.zoom(0.8);
                    }
                    if item(ui, "Fit glyph", "Ctrl+0", has_glyph) {
                        self.view = None;
                    }
                });
                ui.menu_button("Panels", |ui| {
                    ui.checkbox(
                        &mut self.settings.glyph_list_in_overview,
                        "Glyph tree in Overview",
                    )
                    .on_hover_text(
                        "Force the glyph tree on in Overview too. Editor, Review, and Split show it either way.",
                    );
                    ui.checkbox(&mut self.settings.show_inspector, "Inspector");
                    ui.checkbox(&mut self.settings.show_preview, "Docked review sheet");
                    ui.horizontal(|ui| {
                        ui.label("Dock");
                        ui.selectable_value(
                            &mut self.settings.review_place,
                            ReviewPlace::Bottom,
                            "Bottom",
                        );
                        ui.selectable_value(
                            &mut self.settings.review_place,
                            ReviewPlace::Right,
                            "Right",
                        );
                        ui.selectable_value(
                            &mut self.settings.review_place,
                            ReviewPlace::Float,
                            "Window",
                        );
                    });
                    ui.checkbox(&mut self.family_preview, "Preview every style");
                });
                ui.menu_button("Display", |ui| {
                    let onion = self.settings.onion_skin;
                    ui.checkbox(&mut self.settings.onion_skin, "Onion skin")
                        .on_hover_text(
                            "Previous and next glyphs beside this one, outlines only, no handles.",
                        );
                    if self.settings.onion_skin && !onion {
                        self.view = None;
                    }
                    ui.checkbox(&mut self.settings.show_guides, "Guides");
                    ui.horizontal(|ui| {
                        ui.label("Background");
                        ui.selectable_value(&mut self.settings.dark_canvas, false, "White");
                        ui.selectable_value(&mut self.settings.dark_canvas, true, "Black");
                    });
                    ui.separator();
                    ui.checkbox(&mut self.settings.fill, "Fill");
                    ui.checkbox(&mut self.settings.outline, "Outline stroke");
                    ui.checkbox(&mut self.settings.metrics, "Metrics");
                    ui.checkbox(&mut self.settings.point_numbers, "Point numbers");
                });
                ui.separator();
                if item(ui, "Settings…", "Ctrl+,", true) {
                    self.dialogs.settings = true;
                }
            });
            ui.menu_button("Glyph", |ui| {
                if item(ui, "New glyph…", "Ctrl+Shift+N", has_font) {
                    self.dialogs.new_glyph = Some(NewGlyph {
                        name: String::new(),
                        unicode: String::new(),
                        advance: f64::from(self.upm) * 0.6,
                    });
                }
                if item(ui, "Delete glyph", "", has_glyph) {
                    self.delete_current_glyph();
                }
                ui.separator();
                if item(ui, "Previous glyph", "[", has_font) {
                    self.step_glyph(-1);
                }
                if item(ui, "Next glyph", "]", has_font) {
                    self.step_glyph(1);
                }
            });
            ui.menu_button("Tools", |ui| {
                ui.menu_button("Select", |ui| {
                    for (tool, icon, label) in [
                        (Tool::Select, "location-arrow", "Select   V"),
                        (Tool::Lasso, "lasso", "Lasso   L"),
                    ] {
                        if self.icon_menu(ui, icon, label, self.tool == tool) {
                            self.choose_tool(tool);
                        }
                    }
                });
                ui.menu_button("Draw", |ui| {
                    for (tool, icon, label) in [
                        (Tool::Pen, "pencil", "Pen   P"),
                        (Tool::Rectangle, "square", "Rectangle   R"),
                        (Tool::Oval, "circle", "Oval   O"),
                    ] {
                        if self.icon_menu(ui, icon, label, self.tool == tool) {
                            self.choose_tool(tool);
                        }
                    }
                });
                if self.icon_menu(ui, "guide", "Guide   G", self.tool == Tool::Guide) {
                    self.choose_tool(Tool::Guide);
                }
            });
            ui.menu_button("Effects", |ui| {
                if item(ui, "Transform…", "Ctrl+E", has_font) {
                    self.effects.open = true;
                }
                ui.menu_button("Quick", |ui| {
                    for (label, effect) in crate::effects::QUICK {
                        if item(ui, label, "", has_glyph) {
                            self.effects.quick(effect);
                            self.effects.open = true;
                        }
                    }
                });
                ui.menu_button("Round coordinates", |ui| {
                    if item(ui, "This glyph", "", has_glyph) {
                        self.round_glyphs(false);
                    }
                    if item(ui, "All glyphs", "", has_font) {
                        self.round_glyphs(true);
                    }
                });
            });
            ui.menu_button("Help", |ui| {
                if item(ui, "Keyboard shortcuts", "F1", true) {
                    self.dialogs.shortcuts = true;
                }
                if item(ui, "About Type Foundry", "", true) {
                    self.dialogs.about = true;
                }
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.icon_button(
                ui,
                "layout-cells",
                "",
                self.mode == Mode::Overview && !self.settings.split_main,
                true,
                "Overview · Ctrl+1",
            ) {
                self.mode = Mode::Overview;
                self.settings.split_main = false;
            }
            if self.icon_button(
                ui,
                "pencil-to-square",
                "",
                self.mode == Mode::Editor,
                self.current.is_some(),
                "Editor · Ctrl+2 · in Split, the editor takes the right side",
            ) {
                // Editor and Review pick what the main area or the Split right side shows. They
                // leave Split on, so Split and either pane work together.
                self.mode = Mode::Editor;
            }
            if self.icon_button(
                ui,
                "text",
                "",
                self.mode == Mode::Review,
                self.has_font(),
                "Review · Ctrl+3 · in Split, the review sheet takes the right side",
            ) {
                self.mode = Mode::Review;
            }
            if ui
                .selectable_label(self.settings.split_main, "Split")
                .on_hover_text("Show the glyph overview and the editor at the same time")
                .clicked()
            {
                self.settings.split_main = !self.settings.split_main;
                if self.settings.split_main && self.current.is_none() && self.mode != Mode::Review {
                    self.status = (
                        "Pick a glyph to edit beside the overview.".into(),
                        Tone::Quiet,
                    );
                }
            }
            ui.separator();
            for (tool, icon, hover) in [
                (
                    Tool::Select,
                    "location-arrow",
                    "Select · V · drag points, drag empty space to box-select, Alt-click an outline to add a point",
                ),
                (
                    Tool::Pen,
                    "pencil",
                    "Pen · P · click to add corner points, Shift-click for off-curve, click the first point to close",
                ),
                (
                    Tool::Rectangle,
                    "square",
                    "Rectangle · R · drag to add a closed rectangle",
                ),
                (
                    Tool::Oval,
                    "circle",
                    "Oval · O · drag to add a closed oval",
                ),
                (
                    Tool::Lasso,
                    "lasso",
                    "Lasso · L · drag a loop around points. Shift adds to the selection",
                ),
                (
                    Tool::Guide,
                    "guide",
                    "Guide · G · drag across for a horizontal guide, or up and down for a vertical one",
                ),
            ] {
                if self.icon_button(ui, icon, "", self.tool == tool, true, hover) {
                    self.choose_tool(tool);
                }
            }
            ui.separator();
            let (undo, redo) = self.session.history();
            if self.icon_button(ui, "arrow-rotate-left", "", false, undo > 0, "Undo · Ctrl+Z") {
                self.undo();
            }
            if self.icon_button(ui, "arrow-rotate-right", "", false, redo > 0, "Redo · Ctrl+Y") {
                self.redo();
            }
            ui.separator();
            if self.icon_button(
                ui,
                "magic-wand",
                "",
                false,
                self.has_font(),
                "Effects · Ctrl+E",
            ) {
                self.effects.open = true;
            }
            ui.separator();
            self.compare_picker(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(name) = &self.current {
                    ui.weak(name);
                }
                if !self.font_name.is_empty() {
                    let marker = if self.dirty() { " •" } else { "" };
                    ui.strong(format!("{}{marker}", self.font_name));
                }
            });
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (text, tone) = &self.status;
            let tint = match tone {
                Tone::Quiet => color(MUTED),
                Tone::Done => color(SIGNAL),
                Tone::Failed => color(ALERT),
            };
            ui.colored_label(tint, text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(view) = &self.view
                    && self.mode == Mode::Editor
                {
                    ui.weak(format!("{:.0}%", view.scale * 100.0));
                }
                if !self.selection.is_empty() {
                    ui.weak(format!("{} selected", self.selection.len()));
                }
                ui.weak(match self.tool {
                    Tool::Select => "Select",
                    Tool::Pen => "Pen",
                    Tool::Rectangle => "Rectangle",
                    Tool::Oval => "Oval",
                    Tool::Lasso => "Lasso",
                    Tool::Guide => "Guide",
                });
            });
        });
    }

    // ---- Keyboard ---------------------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let command = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let command_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let pressed = |shortcut: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&shortcut));

        // Check the shifted forms first, so Ctrl+Shift+S is not taken as Ctrl+S.
        if pressed(command_shift(Key::S)) && self.has_font() {
            self.save_as_dialog();
        }
        if pressed(command_shift(Key::N)) && self.has_font() {
            self.dialogs.new_glyph = Some(NewGlyph {
                name: String::new(),
                unicode: String::new(),
                advance: f64::from(self.upm) * 0.6,
            });
        }
        if pressed(command_shift(Key::D)) && self.has_font() {
            self.open_new_style();
        }
        if pressed(command_shift(Key::I)) && self.has_font() {
            self.open_italic(None);
        }
        if pressed(command_shift(Key::Tab)) {
            self.step_font(-1);
        }
        if pressed(command(Key::Tab)) {
            self.step_font(1);
        }
        if pressed(command(Key::W))
            && let Some(id) = self.active
        {
            self.request_close(id);
        }
        if pressed(command(Key::N)) {
            self.dialogs.new_font = Some(NewFont {
                name: "Untitled".into(),
                upm: 1000,
            });
        }
        if pressed(command(Key::O)) {
            self.open_file_dialog();
        }
        if pressed(command(Key::S)) && self.has_font() {
            self.save();
        }
        if pressed(command(Key::Q)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if pressed(command(Key::E)) && self.has_font() {
            self.effects.open = true;
        }
        if pressed(command(Key::Comma)) {
            self.dialogs.settings = true;
        }
        if pressed(command(Key::Num1)) {
            self.mode = Mode::Overview;
            self.settings.split_main = false;
        }
        if pressed(command(Key::Num2)) && self.current.is_some() {
            self.mode = Mode::Editor;
        }
        if pressed(command(Key::Num3)) && self.has_font() {
            self.mode = Mode::Review;
        }
        if pressed(command(Key::Equals)) || pressed(command(Key::Plus)) {
            self.zoom(1.25);
        }
        if pressed(command(Key::Minus)) {
            self.zoom(0.8);
        }
        if pressed(command(Key::Num0)) {
            self.view = None;
        }

        // Text fields keep undo and the letter keys. Ctrl+S still saves while typing.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if pressed(command_shift(Key::Z)) {
            self.redo();
        }
        if pressed(command(Key::Z)) {
            self.undo();
        }
        if pressed(command(Key::Y)) {
            self.redo();
        }
        if pressed(command(Key::A)) {
            self.select_all_points();
        }
        let plain = |key| KeyboardShortcut::new(Modifiers::NONE, key);
        let shift = |key| KeyboardShortcut::new(Modifiers::SHIFT, key);
        if pressed(plain(Key::F1)) {
            self.dialogs.shortcuts = true;
        }
        if pressed(plain(Key::V)) {
            self.choose_tool(Tool::Select);
        }
        if pressed(plain(Key::P)) {
            self.choose_tool(Tool::Pen);
        }
        if pressed(plain(Key::R)) {
            self.choose_tool(Tool::Rectangle);
        }
        if pressed(plain(Key::O)) {
            self.choose_tool(Tool::Oval);
        }
        if pressed(plain(Key::L)) {
            self.choose_tool(Tool::Lasso);
        }
        if pressed(plain(Key::G)) {
            self.choose_tool(Tool::Guide);
        }
        if pressed(plain(Key::OpenBracket)) {
            self.step_glyph(-1);
        }
        if pressed(plain(Key::CloseBracket)) {
            self.step_glyph(1);
        }
        if pressed(plain(Key::Escape)) {
            if matches!(
                self.drag,
                Drag::Shape { .. } | Drag::Lasso { .. } | Drag::GuideNew { .. }
            ) {
                self.drag = Drag::None;
                self.status = ("Cancelled.".into(), Tone::Quiet);
            }
            self.selection.clear();
            self.selected_guide = None;
            self.pen_contour = None;
        }
        if pressed(plain(Key::Tab)) {
            self.settings.split_main = false;
            self.mode = match self.mode {
                Mode::Overview if self.current.is_some() => Mode::Editor,
                _ => Mode::Overview,
            };
        }
        if self.editing() {
            if pressed(plain(Key::Delete)) || pressed(plain(Key::Backspace)) {
                if self.selection.is_empty() && self.selected_guide.is_some() {
                    self.delete_selected_guide();
                } else {
                    self.delete_selection();
                }
            }
            for (key, dx, dy) in [
                (Key::ArrowLeft, -1.0, 0.0),
                (Key::ArrowRight, 1.0, 0.0),
                (Key::ArrowUp, 0.0, 1.0),
                (Key::ArrowDown, 0.0, -1.0),
            ] {
                if pressed(shift(key)) {
                    self.nudge(dx * 10.0, dy * 10.0);
                } else if pressed(plain(key)) {
                    self.nudge(dx, dy);
                }
            }
        } else if self.mode == Mode::Overview
            && pressed(plain(Key::Enter))
            && let Some(name) = self.current.clone()
        {
            self.open_editor(name);
        }
    }

    // ---- Dialogs ----------------------------------------------------------------------

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(mut form) = self.dialogs.new_font.take() {
            let mut keep = true;
            let mut create = false;
            egui::Window::new("New font")
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .collapsible(false)
                .resizable(false)
                .open(&mut keep)
                .show(ctx, |ui| {
                    egui::Grid::new("new_font").num_columns(2).show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut form.name);
                        ui.end_row();
                        ui.label("Units per em");
                        ui.add(egui::DragValue::new(&mut form.upm).range(16..=16384));
                        ui.end_row();
                    });
                    create = ui.button("Create").clicked();
                });
            if create {
                self.create(form.name.clone(), form.upm);
            } else if keep {
                self.dialogs.new_font = Some(form);
            }
        }

        if let Some(mut form) = self.dialogs.new_glyph.take() {
            let mut keep = true;
            let mut add = false;
            egui::Window::new("New glyph")
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .collapsible(false)
                .resizable(false)
                .open(&mut keep)
                .show(ctx, |ui| {
                    egui::Grid::new("new_glyph").num_columns(2).show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut form.name)
                            .on_hover_text("For example A, a, zero, or uni00E9");
                        ui.end_row();
                        ui.label("Character or hex");
                        ui.text_edit_singleline(&mut form.unicode).on_hover_text(
                            "Type the character, or a hex code like 00E9. Leave empty for none.",
                        );
                        ui.end_row();
                        ui.label("Advance");
                        ui.add(egui::DragValue::new(&mut form.advance).speed(1.0));
                        ui.end_row();
                    });
                    add = ui.button("Add glyph").clicked();
                });
            if add {
                self.add_glyph(&form);
            } else if keep {
                self.dialogs.new_glyph = Some(form);
            }
        }

        let mut settings_open = self.dialogs.settings;
        egui::Window::new("Settings")
            .open(&mut settings_open)
            .resizable(false)
            .show(ctx, |ui| {
                if self.settings.ui(ui) {
                    self.thumbs.clear();
                }
            });
        self.dialogs.settings = settings_open;

        let mut shortcuts_open = self.dialogs.shortcuts;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut shortcuts_open)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("keys")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (keys, action) in SHORTCUTS {
                            ui.strong(*keys);
                            ui.label(*action);
                            ui.end_row();
                        }
                    });
            });
        self.dialogs.shortcuts = shortcuts_open;

        let mut about_open = self.dialogs.about;
        egui::Window::new("About Type Foundry")
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut about_open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(format!("Type Foundry {}", env!("CARGO_PKG_VERSION")));
                ui.weak("A local type kit. Every edit is a command on one session, the same one the CLI, plugins, and the MCP server use. Nothing is uploaded.");
            });
        self.dialogs.about = about_open;
    }

    fn add_glyph(&mut self, form: &NewGlyph) {
        let name = form.name.trim().to_string();
        if name.is_empty() {
            self.status = ("A glyph needs a name".into(), Tone::Failed);
            self.dialogs.new_glyph = Some(NewGlyph {
                name: String::new(),
                unicode: form.unicode.clone(),
                advance: form.advance,
            });
            return;
        }
        let unicode = parse_unicode(&form.unicode);
        if self
            .edit_json(
                json!({
                    "op": "put_glyph",
                    "glyph": { "name": name, "unicode": unicode, "advance": form.advance, "contours": [] },
                }),
                Scope::Structure,
            )
            .is_some()
        {
            self.open_editor(name.clone());
            self.choose_tool(Tool::Pen);
            self.status = (
                format!("Added {name}. The pen is ready: click to place points."),
                Tone::Done,
            );
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = if self.font_name.is_empty() {
            APP_TITLE.to_string()
        } else {
            let marker = if self.dirty() { "• " } else { "" };
            format!("{marker}{} — {APP_TITLE}", self.font_name)
        };
        if title != self.shown_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.shown_title = title;
        }
    }
}

impl eframe::App for FoundryWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);
        self.update_title(&ctx);

        let bar = egui::Frame::new()
            .fill(color(PANEL))
            .stroke(Stroke::new(1.0, color(HAIRLINE)))
            .inner_margin(egui::Margin::symmetric(8, 4));
        egui::Panel::top("menu")
            .frame(bar)
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("toolbar")
            .frame(bar)
            .show(ui, |ui| self.toolbar(ui));
        if self.has_font() {
            egui::Panel::top("fonts")
                .frame(bar)
                .show(ui, |ui| self.tab_row(ui));
        }
        egui::Panel::bottom("status")
            .frame(bar)
            .show(ui, |ui| self.status_bar(ui));
        // The review pane is the main area in Review mode, so the docked sheet steps aside.
        let review = self.settings.show_preview && self.has_font() && self.mode != Mode::Review;
        if review && self.settings.review_place == ReviewPlace::Bottom {
            egui::Panel::bottom("preview")
                .frame(bar)
                .resizable(true)
                .default_size(300.0)
                .size_range(160.0..=520.0)
                .show(ui, |ui| self.preview_pane(ui));
        }
        if self.glyph_list_visible() {
            egui::Panel::left("glyphs")
                .resizable(true)
                .default_size(170.0)
                .size_range(120.0..=320.0)
                .frame(bar)
                .show(ui, |ui| self.glyph_list(ui));
        }
        if self.settings.show_inspector && self.has_font() {
            self.sync_inspector_focus();
            egui::Panel::right("inspector")
                .resizable(true)
                .default_size(270.0)
                .size_range(240.0..=420.0)
                .frame(bar)
                .show(ui, |ui| self.inspector(ui));
        }
        if review && self.settings.review_place == ReviewPlace::Right {
            egui::Panel::right("review")
                .frame(bar)
                .resizable(true)
                .default_size(420.0)
                .size_range(280.0..=760.0)
                .show(ui, |ui| self.preview_pane(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(color(foundry_app::palette::PAGE)))
            .show(ui, |ui| self.main_view(ui));
        if review && self.settings.review_place == ReviewPlace::Float {
            let mut open = true;
            egui::Window::new("Review")
                .id(egui::Id::new("review-sheet"))
                .open(&mut open)
                .resizable(true)
                .default_size([680.0, 300.0])
                .min_size([360.0, 180.0])
                .show(&ctx, |ui| self.preview_pane(ui));
            self.settings.show_preview = open;
        }

        self.dialogs(&ctx);
        self.family_dialogs(&ctx);
        self.effects_window(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(dir) = &self.last_dir {
            eframe::set_value(storage, LAST_DIR_KEY, &dir.to_string_lossy().into_owned());
        }
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
        eframe::set_value(storage, COPY_KEY, &self.copy);
        eframe::set_value(storage, GUIDES_KEY, &self.guides);
    }
}

impl FoundryWindow {
    /// The editor is on screen, either alone or beside the overview.
    fn editing(&self) -> bool {
        self.current.is_some()
            && match self.mode {
                Mode::Editor => true,
                // Split shows the editor on the right unless the review sheet has that side.
                Mode::Overview => self.settings.split_main,
                Mode::Review => false,
            }
    }

    fn main_view(&mut self, ui: &mut egui::Ui) {
        if self.settings.split_main && self.has_font() {
            self.split_view(ui);
            return;
        }
        match self.mode {
            Mode::Overview => self.overview(ui),
            Mode::Editor => self.canvas(ui),
            Mode::Review => self.preview_pane(ui),
        }
    }

    /// Overview on the left, the glyph being edited on the right, with a draggable split.
    fn split_view(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let ratio = self.settings.split_ratio.clamp(0.22, 0.78);
        let room = full.width().max(96.0);
        let left_w = (room * ratio).clamp(48.0, room - 48.0);
        let gap = 6.0;
        let left = egui::Rect::from_min_size(full.min, egui::vec2(left_w, full.height()));
        let bar = egui::Rect::from_min_size(
            egui::pos2(left.right(), full.top()),
            egui::vec2(gap, full.height()),
        );
        let right = egui::Rect::from_min_max(egui::pos2(bar.right(), full.top()), full.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
            self.overview(ui);
        });
        let separator = ui.allocate_rect(bar, egui::Sense::click_and_drag());
        if separator.hovered() || separator.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if separator.dragged()
            && let Some(pointer) = separator.interact_pointer_pos()
        {
            self.settings.split_ratio =
                ((pointer.x - full.left()) / full.width()).clamp(0.22, 0.78);
        }
        ui.painter().vline(
            bar.center().x,
            bar.y_range(),
            Stroke::new(1.0, color(foundry_app::palette::HAIRLINE)),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
            // The right side is the review sheet in Review mode, otherwise the glyph editor.
            if self.mode == Mode::Review {
                self.preview_pane(ui);
            } else if self.current.is_some() {
                self.canvas(ui);
            } else {
                ui.weak("Pick a glyph in the overview to edit it here.");
            }
        });
    }
}

/// A menu entry with a shortcut hint. Returns true when clicked, and closes the menu.
fn item(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    let clicked = ui
        .add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut))
        .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn is_writable(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "json" | "ufo" | "ttf"))
}

/// One typed character, or a hex code of two or more digits with or without `U+`. Empty means
/// none.
/// Reads the glyph entries of an `index` response. A glyph with no name is skipped.
fn glyph_entries(entries: &[Value]) -> Vec<GlyphEntry> {
    entries
        .iter()
        .filter_map(|entry| {
            Some(GlyphEntry {
                name: entry["name"].as_str()?.to_string(),
                unicode: entry["unicode"]
                    .as_u64()
                    .and_then(|code| u32::try_from(code).ok()),
                group: GlyphGroup::ALL
                    .into_iter()
                    .find(|group| Some(group.label()) == entry["group"].as_str())
                    .unwrap_or(GlyphGroup::Unencoded),
            })
        })
        .collect()
}

pub fn parse_unicode(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut chars = text.chars();
    if let (Some(only), None) = (chars.next(), chars.next()) {
        return Some(u32::from(only));
    }
    let hex = text
        .trim_start_matches("U+")
        .trim_start_matches("u+")
        .trim_start_matches("0x");
    u32::from_str_radix(hex, 16)
        .ok()
        .filter(|code| char::from_u32(*code).is_some())
}

const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+N", "New font"),
    ("Ctrl+O", "Open"),
    ("Ctrl+S / Ctrl+Shift+S", "Save / Save As"),
    ("Ctrl+Z / Ctrl+Shift+Z, Ctrl+Y", "Undo / Redo"),
    ("Ctrl+1 / Ctrl+2, Tab", "Overview / Editor"),
    ("Enter (overview)", "Edit the selected glyph"),
    ("[ and ]", "Previous / next glyph"),
    ("V / L", "Select tool / Lasso"),
    ("G", "Guide. Drag across or up and down"),
    ("V / P", "Select tool / Pen tool"),
    (
        "R / O",
        "Rectangle / Oval. Drag to add one. Esc cancels the drag",
    ),
    ("Click, Shift+click", "Select a point, add to the selection"),
    ("Drag empty space", "Box select (Shift adds)"),
    ("Alt+click an outline", "Add a point on the segment"),
    ("Double-click a point", "Toggle smooth"),
    ("Arrows, Shift+arrows", "Nudge 1 or 10 units"),
    ("Delete / Backspace", "Delete selected points"),
    ("Ctrl+A / Esc", "Select all points / Deselect"),
    (
        "Pen: click, Shift+click",
        "Add an on-curve / off-curve point",
    ),
    ("Pen: click the first point", "Close the contour"),
    ("Scroll, Ctrl+= / Ctrl+-", "Zoom"),
    ("Right or middle drag", "Pan"),
    ("Ctrl+0, double-click empty", "Fit the glyph"),
    ("Ctrl+E", "Effects"),
    ("Ctrl+Shift+I", "Make an italic style from this font"),
    ("Ctrl+Shift+D", "New style from this font"),
    ("Ctrl+,", "Settings"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_characters_and_hex_codes() {
        assert_eq!(parse_unicode("é"), Some(0xE9));
        assert_eq!(parse_unicode("00E9"), Some(0xE9));
        assert_eq!(parse_unicode("U+0041"), Some(0x41));
        assert_eq!(
            parse_unicode("A"),
            Some(0x41),
            "one character is that character"
        );
        assert_eq!(parse_unicode("Z"), Some(u32::from('Z')));
        assert_eq!(parse_unicode(""), None);
        assert_eq!(parse_unicode("D800"), None);
    }

    fn window_with_font() -> FoundryWindow {
        let mut window = FoundryWindow::new(None, Settings::default());
        let response = window.session.execute(Command::Create {
            name: "Test".into(),
            upm: 1000,
        });
        assert!(response.ok, "{}", response.error.unwrap_or_default());
        window
    }

    #[test]
    fn measure_and_capture_update_inspector_state() {
        let mut window = window_with_font();
        let put = json!({
            "op": "put_glyph",
            "glyph": {
                "name": "H",
                "unicode": 72,
                "advance": 120.0,
                "contours": [
                    {"closed": true, "points": [
                        {"x": 0.0, "y": 0.0, "kind": "on", "smooth": false},
                        {"x": 20.0, "y": 0.0, "kind": "on", "smooth": false},
                        {"x": 20.0, "y": 100.0, "kind": "on", "smooth": false},
                        {"x": 0.0, "y": 100.0, "kind": "on", "smooth": false}
                    ]},
                    {"closed": true, "points": [
                        {"x": 80.0, "y": 0.0, "kind": "on", "smooth": false},
                        {"x": 100.0, "y": 0.0, "kind": "on", "smooth": false},
                        {"x": 100.0, "y": 100.0, "kind": "on", "smooth": false},
                        {"x": 80.0, "y": 100.0, "kind": "on", "smooth": false}
                    ]}
                ]
            }
        });
        assert!(window.edit_json(put, Scope::Structure).is_some());
        window.measure_genome();
        let snap = window
            .genome_snapshot
            .as_ref()
            .expect("measure fills snapshot");
        assert_eq!(snap["primary_stem"], json!(20.0));
        window.capture_genome();
        assert!(
            window.genome_snapshot.as_ref().unwrap()["primary_stem"]
                .as_f64()
                .is_some()
        );
        window.run_audit();
        assert!(
            window.audit_report.as_ref().unwrap()["count"]
                .as_u64()
                .is_some()
        );
    }

    #[test]
    fn glyph_list_stays_off_in_overview_until_override() {
        let mut window = window_with_font();
        window.mode = Mode::Overview;
        window.settings.split_main = false;
        window.settings.glyph_list_in_overview = false;
        assert!(!window.glyph_list_visible());

        window.settings.glyph_list_in_overview = true;
        assert!(window.glyph_list_visible());
    }

    #[test]
    fn glyph_list_shows_in_editor_review_and_split_without_override() {
        let mut window = window_with_font();
        window.settings.glyph_list_in_overview = false;

        window.mode = Mode::Editor;
        window.settings.split_main = false;
        assert!(window.glyph_list_visible());

        window.mode = Mode::Review;
        assert!(window.glyph_list_visible());

        window.mode = Mode::Overview;
        window.settings.split_main = true;
        assert!(window.glyph_list_visible());
    }

    #[test]
    fn inspector_focus_follows_mode_and_selection() {
        let mut window = window_with_font();
        window.mode = Mode::Overview;
        window.current = None;
        window.sync_inspector_focus();
        assert!(window.inspector_font_open && window.inspector_style_open);
        assert!(!window.inspector_glyph_open && !window.inspector_selection_open);

        window.current = Some("A".into());
        window.sync_inspector_focus();
        assert!(window.inspector_glyph_open);

        window.mode = Mode::Editor;
        window.selection.clear();
        window.sync_inspector_focus();
        assert!(!window.inspector_font_open && !window.inspector_style_open);
        assert!(window.inspector_glyph_open && !window.inspector_selection_open);

        window.selection.insert(Handle {
            contour: 0,
            point: 0,
        });
        window.sync_inspector_focus();
        assert!(window.inspector_selection_open);
        assert!(!window.inspector_glyph_open);
    }
}
