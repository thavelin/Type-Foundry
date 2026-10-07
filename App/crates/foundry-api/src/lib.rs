//! One command session. The CLI, plugins, and agents all go through [`Session::execute`].

use std::path::Path;

use foundry_core::{
    Anchor, Contour, Corner, ExportFormat, Font, FoundryError, Glyph, InfoUpdate, Kerning, Matrix,
    MetricsUpdate, OffsetOptions, PointKind, ProofOptions, Side, StrokeKind, StyleUpdate,
    blend_fonts, check_family, check_outlines, check_spacing, classify, compatibility, copy_family,
    diff_fonts, export_family, load_family, offset_font, save_family, stroke_font, write_proof,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use foundry_core::GlyphGroup;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    Create {
        name: String,
        #[serde(default = "default_upm")]
        upm: u16,
    },
    Open {
        path: String,
    },
    Save {
        path: String,
        /// Replace an existing file. A copy is kept beside it, with `.bak` added to the name.
        #[serde(default)]
        force: bool,
    },
    Info,
    Glyphs,
    Glyph {
        name: String,
        /// Read from this open font instead of the active one.
        #[serde(default)]
        font: Option<u32>,
    },
    PutGlyph {
        glyph: Glyph,
    },
    SetAdvance {
        name: String,
        advance: f64,
    },
    MovePoint {
        name: String,
        contour: usize,
        point: usize,
        x: f64,
        y: f64,
    },
    MovePoints {
        name: String,
        points: Vec<[usize; 2]>,
        dx: f64,
        dy: f64,
    },
    /// Absolute positions for several points of one glyph. One undo step.
    SetPoints {
        name: String,
        points: Vec<PlacedPoint>,
    },
    InsertPoint {
        name: String,
        contour: usize,
        index: usize,
        x: f64,
        y: f64,
        #[serde(default = "default_kind")]
        kind: PointKind,
        #[serde(default)]
        smooth: bool,
    },
    SplitSegment {
        name: String,
        contour: usize,
        point: usize,
        t: f64,
    },
    DeletePoints {
        name: String,
        points: Vec<[usize; 2]>,
    },
    SetPoint {
        name: String,
        contour: usize,
        point: usize,
        #[serde(default)]
        kind: Option<PointKind>,
        #[serde(default)]
        smooth: Option<bool>,
    },
    AddContour {
        name: String,
        contour: Contour,
    },
    SetClosed {
        name: String,
        contour: usize,
        closed: bool,
    },
    ReverseContour {
        name: String,
        contour: usize,
    },
    DeleteGlyph {
        name: String,
    },
    RenameGlyph {
        name: String,
        new_name: String,
    },
    SetUnicode {
        name: String,
        #[serde(default)]
        unicode: Option<u32>,
    },
    RenameFont {
        name: String,
    },
    SetMetrics {
        #[serde(default)]
        ascender: Option<f64>,
        #[serde(default)]
        descender: Option<f64>,
        #[serde(default)]
        cap_height: Option<f64>,
        #[serde(default)]
        x_height: Option<f64>,
    },
    Transform {
        #[serde(default)]
        names: Option<Vec<String>>,
        #[serde(default)]
        points: Option<Vec<[usize; 2]>>,
        matrix: Matrix,
        #[serde(default)]
        advance: bool,
        #[serde(default)]
        anchor: Anchor,
    },
    RoundCoordinates {
        #[serde(default)]
        names: Option<Vec<String>>,
    },
    Index {
        #[serde(default)]
        font: Option<u32>,
    },
    Fonts,
    SelectFont {
        id: u32,
    },
    CloseFont {
        #[serde(default)]
        id: Option<u32>,
    },
    SetStyle {
        #[serde(default)]
        family: Option<String>,
        #[serde(default)]
        style: Option<String>,
        #[serde(default)]
        weight: Option<u16>,
        #[serde(default)]
        italic: Option<bool>,
        #[serde(default)]
        italic_angle: Option<f64>,
        /// OS/2 width class, 1 (ultra-condensed) through 9 (ultra-expanded). 5 is normal.
        #[serde(default)]
        width: Option<u16>,
    },
    DeriveStyle {
        style: String,
        #[serde(default)]
        weight: Option<u16>,
        #[serde(default)]
        italic: Option<bool>,
        #[serde(default)]
        slant: f64,
    },
    OpenFamily {
        path: String,
    },
    SaveFamily {
        path: String,
        #[serde(default)]
        ids: Option<Vec<u32>>,
        #[serde(default)]
        force: bool,
    },
    ExportFamily {
        dir: String,
        format: ExportFormat,
        #[serde(default)]
        ids: Option<Vec<u32>>,
        #[serde(default)]
        force: bool,
    },
    FamilyCheck {
        #[serde(default)]
        ids: Option<Vec<u32>>,
    },
    Undo,
    Redo,
    Checkpoint,
    History,
    Check {
        a: String,
        b: String,
    },
    Blend {
        a: String,
        b: String,
        #[serde(default = "default_t")]
        t: f64,
        out: String,
        #[serde(default)]
        force: bool,
    },
    SetInfo {
        #[serde(default)]
        copyright: Option<String>,
        #[serde(default)]
        designer: Option<String>,
        #[serde(default)]
        license: Option<String>,
        #[serde(default)]
        license_url: Option<String>,
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        vendor: Option<String>,
        #[serde(default)]
        unique_id: Option<String>,
    },
    SetKerning {
        kerning: Kerning,
    },
    AddKern {
        left: String,
        right: String,
        value: f64,
    },
    SetGroup {
        name: String,
        members: Vec<String>,
    },
    AddLigature {
        glyphs: Vec<String>,
        name: String,
    },
    /// `text` omitted keeps a UFO's `features.fea`. An empty string replaces it.
    SetFeatures {
        #[serde(default)]
        text: Option<String>,
    },
    Offset {
        #[serde(default)]
        horizontal: f64,
        #[serde(default)]
        vertical: f64,
        #[serde(default)]
        gap: f64,
        #[serde(default)]
        corner: Corner,
        #[serde(default)]
        sidebearing: bool,
        #[serde(default = "default_true")]
        keep_metrics: bool,
        /// Insert arc points at round joins. Needs `corner: round`.
        #[serde(default)]
        add_points: bool,
        #[serde(default)]
        names: Option<Vec<String>>,
        /// Return the moved outlines and leave the open font alone.
        #[serde(default)]
        preview: bool,
    },
    Stroke {
        kind: StrokeKind,
        #[serde(default)]
        horizontal: f64,
        #[serde(default)]
        vertical: f64,
        #[serde(default)]
        gap: f64,
        #[serde(default)]
        corner: Corner,
        #[serde(default)]
        sidebearing: bool,
        #[serde(default = "default_true")]
        keep_metrics: bool,
        /// Insert arc points at round joins. Needs `corner: round`.
        #[serde(default)]
        add_points: bool,
        #[serde(default)]
        names: Option<Vec<String>>,
        #[serde(default)]
        preview: bool,
    },
    SetSidebearing {
        name: String,
        side: Side,
        value: f64,
        /// Apply the same sidebearing to every open style in the family.
        #[serde(default)]
        family: bool,
        #[serde(default)]
        ids: Option<Vec<u32>>,
    },
    CheckOutlines,
    CheckSpacing {
        #[serde(default)]
        min_gap: f64,
        #[serde(default)]
        pairs: Option<Vec<(String, String)>>,
    },
    Diff {
        a: String,
        b: String,
    },
    Proof {
        path: String,
        #[serde(default)]
        text: String,
        #[serde(default = "default_pixel")]
        pixel_size: f64,
        #[serde(default = "default_true")]
        guides: bool,
        #[serde(default = "default_true")]
        boxes: bool,
        #[serde(default)]
        compare: Option<String>,
        #[serde(default)]
        force: bool,
    },
    MoveGlyph {
        name: String,
        index: usize,
    },
    CopyFamily {
        path: String,
        dir: String,
        #[serde(default)]
        family: Option<String>,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        force: bool,
    },
    Slant {
        degrees: f64,
    },
    ScaleWidth {
        factor: f64,
        #[serde(default)]
        names: Option<Vec<String>>,
    },
}

fn default_upm() -> u16 {
    1000
}

fn default_t() -> f64 {
    0.5
}

fn default_kind() -> PointKind {
    PointKind::On
}

fn default_true() -> bool {
    true
}

fn default_pixel() -> f64 {
    72.0
}

/// One point set to an absolute position by `set_points`.
#[derive(Debug, Clone, Deserialize)]
pub struct PlacedPoint {
    pub contour: usize,
    pub point: usize,
    pub x: f64,
    pub y: f64,
}

/// Undo steps kept per session. Older steps are dropped.
pub const UNDO_LIMIT: usize = 200;

impl Command {
    /// True for commands that change the open font in place and can be undone.
    /// A family sidebearing and an offset preview change nothing on the active font by themselves:
    /// the family edit records one step on each style, and a preview only returns outlines.
    fn is_edit(&self) -> bool {
        match self {
            Self::SetSidebearing { family: true, .. }
            | Self::Offset { preview: true, .. }
            | Self::Stroke { preview: true, .. } => false,
            Self::PutGlyph { .. }
            | Self::SetAdvance { .. }
            | Self::MovePoint { .. }
            | Self::MovePoints { .. }
            | Self::SetPoints { .. }
            | Self::InsertPoint { .. }
            | Self::SplitSegment { .. }
            | Self::DeletePoints { .. }
            | Self::SetPoint { .. }
            | Self::AddContour { .. }
            | Self::SetClosed { .. }
            | Self::ReverseContour { .. }
            | Self::DeleteGlyph { .. }
            | Self::RenameGlyph { .. }
            | Self::SetUnicode { .. }
            | Self::RenameFont { .. }
            | Self::SetMetrics { .. }
            | Self::Transform { .. }
            | Self::RoundCoordinates { .. }
            | Self::SetStyle { .. }
            | Self::SetInfo { .. }
            | Self::SetKerning { .. }
            | Self::AddKern { .. }
            | Self::SetGroup { .. }
            | Self::AddLigature { .. }
            | Self::SetFeatures { .. }
            | Self::Offset { .. }
            | Self::Stroke { .. }
            | Self::SetSidebearing { .. }
            | Self::MoveGlyph { .. }
            | Self::Slant { .. }
            | Self::ScaleWidth { .. } => true,
            _ => false,
        }
    }

    /// Consecutive edits with the same key share one undo step, so a drag or a slider is one
    /// step. `checkpoint` ends the run.
    fn coalesce_key(&self) -> Option<String> {
        match self {
            Self::MovePoint {
                name,
                contour,
                point,
                ..
            } => Some(format!("move:{name}:{:?}", [[*contour, *point]])),
            Self::MovePoints { name, points, .. } => {
                let mut sorted = points.clone();
                sorted.sort_unstable();
                sorted.dedup();
                Some(format!("move:{name}:{sorted:?}"))
            }
            Self::SetAdvance { name, .. } => Some(format!("advance:{name}")),
            Self::SetMetrics { .. } => Some("metrics".to_string()),
            Self::SetStyle { .. } => Some("style".to_string()),
            _ => None,
        }
    }
}

fn refs(points: &[[usize; 2]]) -> Vec<(usize, usize)> {
    points.iter().map(|[c, p]| (*c, *p)).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Response {
    fn done(data: Option<Value>) -> Self {
        Self {
            ok: true,
            error: None,
            data,
        }
    }

    fn fail(message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            data,
        }
    }
}

#[derive(Debug)]
struct Fail {
    message: String,
    data: Option<Value>,
}

impl From<FoundryError> for Fail {
    fn from(err: FoundryError) -> Self {
        Self {
            message: err.to_string(),
            data: None,
        }
    }
}

/// One open font with its own undo history.
#[derive(Debug)]
struct Doc {
    id: u32,
    font: Font,
    undo: Vec<Font>,
    redo: Vec<Font>,
    last_edit: Option<String>,
    dirty: bool,
}

/// Open fonts and the active one. Editing commands act on the active font; `select_font`
/// switches it. Each font keeps its own undo history.
#[derive(Debug, Default)]
pub struct Session {
    docs: Vec<Doc>,
    active: Option<u32>,
    next_id: u32,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// The active font.
    pub fn font(&self) -> Option<&Font> {
        self.doc().map(|doc| &doc.font)
    }

    /// The active font's id.
    pub fn active(&self) -> Option<u32> {
        self.active
    }

    /// Undo and redo steps available on the active font.
    pub fn history(&self) -> (usize, usize) {
        self.doc()
            .map_or((0, 0), |doc| (doc.undo.len(), doc.redo.len()))
    }

    fn doc(&self) -> Option<&Doc> {
        let id = self.active?;
        self.docs.iter().find(|doc| doc.id == id)
    }

    fn doc_mut(&mut self) -> Option<&mut Doc> {
        let id = self.active?;
        self.docs.iter_mut().find(|doc| doc.id == id)
    }

    fn font_mut(&mut self) -> Option<&mut Font> {
        self.doc_mut().map(|doc| &mut doc.font)
    }

    fn font_by(&self, id: Option<u32>) -> Result<&Font, FoundryError> {
        match id {
            None => self.font().ok_or(FoundryError::NoFont),
            Some(id) => self
                .docs
                .iter()
                .find(|doc| doc.id == id)
                .map(|doc| &doc.font)
                .ok_or_else(|| FoundryError::Family(format!("no open font has id {id}"))),
        }
    }

    /// Open a font beside the others and make it active. Returns its id.
    fn add_font(&mut self, font: Font, dirty: bool) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        self.docs.push(Doc {
            id,
            font,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            dirty,
        });
        self.active = Some(id);
        id
    }

    /// The fonts a family command works on: the listed ids in that order, or every open font
    /// in the active font's family, in the order they were opened.
    fn family_members(&self, ids: Option<&[u32]>) -> Result<Vec<&Font>, FoundryError> {
        match ids {
            Some(ids) => ids.iter().map(|id| self.font_by(Some(*id))).collect(),
            None => {
                let family = &self.font().ok_or(FoundryError::NoFont)?.style.family;
                Ok(self
                    .docs
                    .iter()
                    .filter(|doc| &doc.font.style.family == family)
                    .map(|doc| &doc.font)
                    .collect())
            }
        }
    }

    fn family_ids(&self, ids: Option<&[u32]>) -> Result<Vec<u32>, FoundryError> {
        match ids {
            Some(ids) => {
                for id in ids {
                    self.font_by(Some(*id))?;
                }
                Ok(ids.to_vec())
            }
            None => {
                let family = &self.font().ok_or(FoundryError::NoFont)?.style.family;
                Ok(self
                    .docs
                    .iter()
                    .filter(|doc| &doc.font.style.family == family)
                    .map(|doc| doc.id)
                    .collect())
            }
        }
    }

    fn font_mut_id(&mut self, id: u32) -> Result<&mut Font, FoundryError> {
        self.docs
            .iter_mut()
            .find(|doc| doc.id == id)
            .map(|doc| &mut doc.font)
            .ok_or_else(|| FoundryError::Family(format!("no open font has id {id}")))
    }

    /// One undo step on each listed font, recorded before the edit.
    fn remember_undo(&mut self, ids: &[u32]) {
        for doc in &mut self.docs {
            if !ids.contains(&doc.id) {
                continue;
            }
            doc.undo.push(doc.font.clone());
            if doc.undo.len() > UNDO_LIMIT {
                doc.undo.remove(0);
            }
            doc.redo.clear();
            doc.last_edit = None;
            doc.dirty = true;
        }
    }

    fn offset(
        &mut self,
        options: OffsetOptions,
        kind: Option<StrokeKind>,
        preview: bool,
    ) -> Result<Option<Value>, Fail> {
        if preview {
            let mut copy = self.font().ok_or(FoundryError::NoFont)?.clone();
            let changed = apply_offset(&mut copy, &options, kind)?;
            return Ok(Some(preview_glyphs(&copy, &changed)));
        }
        let font = self.font_mut().ok_or(FoundryError::NoFont)?;
        let changed = apply_offset(font, &options, kind)?;
        Ok(Some(json!({ "glyphs": changed })))
    }

    fn set_sidebearing(
        &mut self,
        name: String,
        side: Side,
        value: f64,
        family: bool,
        ids: Option<Vec<u32>>,
    ) -> Result<Option<Value>, Fail> {
        if !value.is_finite() {
            return Err(FoundryError::NonFinite.into());
        }
        if !family {
            let font = self.font_mut().ok_or(FoundryError::NoFont)?;
            font.set_sidebearing(&name, side, value)?;
            return Ok(Some(json!({ "name": name, "side": side, "value": value })));
        }
        let targets = self.family_ids(ids.as_deref())?;
        if targets.is_empty() {
            return Err(FoundryError::Family("there are no styles to edit".into()).into());
        }
        for id in &targets {
            let font = self.font_by(Some(*id))?;
            let glyph = font
                .glyph(&name)
                .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))?;
            if side == Side::Left
                && glyph
                    .contours
                    .iter()
                    .all(|contour| contour.points.is_empty())
            {
                return Err(FoundryError::Edit(format!(
                    "{name} has no outline in {}, so it has no left sidebearing",
                    font.style.name
                ))
                .into());
            }
        }
        self.remember_undo(&targets);
        for id in &targets {
            self.font_mut_id(*id)?.set_sidebearing(&name, side, value)?;
        }
        Ok(Some(json!({
            "name": name,
            "side": side,
            "value": value,
            "styles": targets.len(),
        })))
    }

    fn font_list(&self) -> Value {
        let fonts: Vec<Value> = self
            .docs
            .iter()
            .map(|doc| {
                json!({
                    "id": doc.id,
                    "name": doc.font.name,
                    "family": doc.font.style.family,
                    "style": doc.font.style.name,
                    "weight": doc.font.style.weight,
                    "italic": doc.font.style.italic,
                    "glyphs": doc.font.glyphs.len(),
                    "active": Some(doc.id) == self.active,
                    "dirty": doc.dirty,
                })
            })
            .collect();
        json!({ "fonts": fonts, "active": self.active })
    }

    pub fn execute(&mut self, command: Command) -> Response {
        let edit = command.is_edit();
        let key = command.coalesce_key();
        let before = match self.doc() {
            Some(doc) if edit && (key.is_none() || key != doc.last_edit) => Some(doc.font.clone()),
            _ => None,
        };
        let result = self.dispatch(command);
        if result.is_ok()
            && edit
            && let Some(doc) = self.doc_mut()
        {
            if let Some(snapshot) = before {
                doc.undo.push(snapshot);
                if doc.undo.len() > UNDO_LIMIT {
                    doc.undo.remove(0);
                }
            }
            doc.redo.clear();
            doc.last_edit = key;
            doc.dirty = true;
        }
        match result {
            Ok(data) => Response::done(data),
            Err(err) => Response::fail(err.message, err.data),
        }
    }

    /// Parse one command line. Blank lines and `#` comments return `None`.
    pub fn execute_line(&mut self, line: &str) -> Option<Response> {
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }
        let command = match serde_json::from_str::<Command>(trimmed) {
            Ok(command) => command,
            Err(err) => {
                return Some(Response::fail(explain_command_error(&err), None));
            }
        };
        Some(self.execute(command))
    }

    fn dispatch(&mut self, command: Command) -> Result<Option<Value>, Fail> {
        match command {
            Command::Create { name, upm } => {
                let font = Font::new(name, upm)?;
                let id = self.add_font(font, true);
                Ok(Some(self.summary(id)))
            }
            Command::Open { path } => {
                let font = Font::load(Path::new(&path))?;
                let id = self.add_font(font, false);
                Ok(Some(self.summary(id)))
            }
            Command::Save { path, force } => {
                let font = self.font().ok_or(FoundryError::NoFont)?;
                font.save_with(Path::new(&path), force)?;
                if let Some(doc) = self.doc_mut() {
                    doc.dirty = false;
                }
                Ok(Some(json!({ "path": path })))
            }
            Command::Info => {
                let id = self.active.ok_or(FoundryError::NoFont)?;
                Ok(Some(self.summary(id)))
            }
            Command::Glyphs => {
                let font = self.font().ok_or(FoundryError::NoFont)?;
                let names: Vec<&str> = font.glyph_names();
                Ok(Some(json!({ "glyphs": names })))
            }
            Command::Glyph { name, font } => {
                let font = self.font_by(font)?;
                let glyph = font.glyph(&name).ok_or(FoundryError::MissingGlyph(name))?;
                Ok(Some(
                    serde_json::to_value(glyph)
                        .map_err(|err| FoundryError::Json(err.to_string()))?,
                ))
            }
            Command::PutGlyph { glyph } => {
                let name = glyph.name.clone();
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.insert_glyph(glyph)?;
                Ok(Some(json!({ "name": name })))
            }
            Command::SetAdvance { name, advance } => {
                if !advance.is_finite() {
                    return Err(FoundryError::NonFinite.into());
                }
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                let glyph = font
                    .glyph_mut(&name)
                    .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))?;
                glyph.advance = advance;
                Ok(Some(json!({ "name": name, "advance": advance })))
            }
            Command::MovePoint {
                name,
                contour,
                point,
                x,
                y,
            } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.move_point(&name, contour, point, x, y)?;
                Ok(Some(json!({
                    "name": name,
                    "contour": contour,
                    "point": point,
                    "x": x,
                    "y": y,
                })))
            }
            Command::MovePoints {
                name,
                points,
                dx,
                dy,
            } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.move_points(&name, &refs(&points), dx, dy)?;
                Ok(Some(
                    json!({ "name": name, "points": points, "dx": dx, "dy": dy }),
                ))
            }
            Command::SetPoints { name, points } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                let places: Vec<(usize, usize, f64, f64)> = points
                    .iter()
                    .map(|point| (point.contour, point.point, point.x, point.y))
                    .collect();
                font.set_points(&name, &places)?;
                Ok(Some(json!({ "name": name, "count": points.len() })))
            }
            Command::InsertPoint {
                name,
                contour,
                index,
                x,
                y,
                kind,
                smooth,
            } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                let point = foundry_core::Point { x, y, kind, smooth };
                font.insert_point(&name, contour, index, point)?;
                Ok(Some(
                    json!({ "name": name, "contour": contour, "point": index }),
                ))
            }
            Command::SplitSegment {
                name,
                contour,
                point,
                t,
            } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                let index = font.split_segment(&name, contour, point, t)?;
                Ok(Some(
                    json!({ "name": name, "contour": contour, "point": index }),
                ))
            }
            Command::DeletePoints { name, points } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.delete_points(&name, &refs(&points))?;
                Ok(Some(json!({ "name": name, "deleted": points.len() })))
            }
            Command::SetPoint {
                name,
                contour,
                point,
                kind,
                smooth,
            } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.set_point_type(&name, contour, point, kind, smooth)?;
                let glyph = font
                    .glyph(&name)
                    .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))?;
                let set = &glyph.contours[contour].points[point];
                Ok(Some(json!({
                    "name": name, "contour": contour, "point": point,
                    "kind": set.kind, "smooth": set.smooth,
                })))
            }
            Command::AddContour { name, contour } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                let index = font.add_contour(&name, contour)?;
                Ok(Some(json!({ "name": name, "contour": index })))
            }
            Command::SetClosed {
                name,
                contour,
                closed,
            } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.set_closed(&name, contour, closed)?;
                Ok(Some(
                    json!({ "name": name, "contour": contour, "closed": closed }),
                ))
            }
            Command::ReverseContour { name, contour } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.reverse_contour(&name, contour)?;
                Ok(Some(json!({ "name": name, "contour": contour })))
            }
            Command::DeleteGlyph { name } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.delete_glyph(&name)?;
                Ok(Some(json!({ "name": name })))
            }
            Command::RenameGlyph { name, new_name } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.rename_glyph(&name, &new_name)?;
                Ok(Some(json!({ "name": new_name, "was": name })))
            }
            Command::SetUnicode { name, unicode } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.set_unicode(&name, unicode)?;
                Ok(Some(json!({ "name": name, "unicode": unicode })))
            }
            Command::RenameFont { name } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.rename(&name)?;
                let id = self.active.ok_or(FoundryError::NoFont)?;
                Ok(Some(self.summary(id)))
            }
            Command::SetMetrics {
                ascender,
                descender,
                cap_height,
                x_height,
            } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.set_metrics(MetricsUpdate {
                    ascender,
                    descender,
                    cap_height,
                    x_height,
                })?;
                Ok(Some(json!({ "metrics": font.metrics })))
            }
            Command::Transform {
                names,
                points,
                matrix,
                advance,
                anchor,
            } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                let selection = points.as_deref().map(refs);
                let changed = font.transform_anchored(
                    names.as_deref(),
                    selection.as_deref(),
                    matrix,
                    advance,
                    anchor,
                )?;
                Ok(Some(json!({ "glyphs": changed })))
            }
            Command::RoundCoordinates { names } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                let changed = font.round_coordinates(names.as_deref())?;
                Ok(Some(json!({ "glyphs": changed })))
            }
            Command::Index { font } => {
                let font = self.font_by(font)?;
                let ligatures: Vec<&str> = font
                    .kerning
                    .as_ref()
                    .map(|kerning| {
                        kerning
                            .ligatures
                            .iter()
                            .map(|liga| liga.name.as_str())
                            .collect()
                    })
                    .unwrap_or_default();
                let glyphs: Vec<Value> = font
                    .glyphs
                    .iter()
                    .map(|glyph| {
                        let group =
                            classify(glyph.unicode, ligatures.contains(&glyph.name.as_str()));
                        json!({
                            "name": glyph.name,
                            "unicode": glyph.unicode,
                            "group": group.label(),
                            "advance": glyph.advance,
                            "contours": glyph.contours.len(),
                            "points": glyph.contours.iter().map(|c| c.points.len()).sum::<usize>(),
                        })
                    })
                    .collect();
                Ok(Some(json!({ "glyphs": glyphs })))
            }
            Command::Undo => {
                let doc = self.doc_mut().ok_or(FoundryError::NoFont)?;
                let previous = doc
                    .undo
                    .pop()
                    .ok_or_else(|| FoundryError::Edit("nothing to undo".into()))?;
                let current = std::mem::replace(&mut doc.font, previous);
                doc.redo.push(current);
                doc.last_edit = None;
                doc.dirty = true;
                self.history_data()
            }
            Command::Redo => {
                let doc = self.doc_mut().ok_or(FoundryError::NoFont)?;
                let next = doc
                    .redo
                    .pop()
                    .ok_or_else(|| FoundryError::Edit("nothing to redo".into()))?;
                let current = std::mem::replace(&mut doc.font, next);
                doc.undo.push(current);
                doc.last_edit = None;
                doc.dirty = true;
                self.history_data()
            }
            Command::Checkpoint => {
                if let Some(doc) = self.doc_mut() {
                    doc.last_edit = None;
                }
                self.history_data()
            }
            Command::Fonts => Ok(Some(self.font_list())),
            Command::SelectFont { id } => {
                self.font_by(Some(id))?;
                self.active = Some(id);
                Ok(Some(self.summary(id)))
            }
            Command::CloseFont { id } => {
                let id = id.or(self.active).ok_or(FoundryError::NoFont)?;
                let index = self
                    .docs
                    .iter()
                    .position(|doc| doc.id == id)
                    .ok_or_else(|| FoundryError::Family(format!("no open font has id {id}")))?;
                self.docs.remove(index);
                if self.active == Some(id) {
                    // The neighbour that slid into its place, or the last font.
                    self.active = self
                        .docs
                        .get(index)
                        .or_else(|| self.docs.last())
                        .map(|doc| doc.id);
                }
                Ok(Some(self.font_list()))
            }
            Command::SetStyle {
                family,
                style,
                weight,
                italic,
                italic_angle,
                width,
            } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.set_style(StyleUpdate {
                    family,
                    name: style,
                    weight,
                    italic,
                    italic_angle,
                    width,
                })?;
                Ok(Some(json!({ "name": font.name, "style": font.style })))
            }
            Command::DeriveStyle {
                style,
                weight,
                italic,
                slant,
            } => {
                let source = self.font().ok_or(FoundryError::NoFont)?;
                let derived = source.derive_style(&style, weight, italic, slant)?;
                let id = self.add_font(derived, true);
                Ok(Some(self.summary(id)))
            }
            Command::OpenFamily { path } => {
                let (file, members) = load_family(Path::new(&path))?;
                let mut opened = Vec::new();
                for (member, font) in members {
                    let id = self.add_font(font, false);
                    opened.push(json!({ "id": id, "path": member.to_string_lossy() }));
                }
                if let Some(first) = opened.first().and_then(|entry| entry["id"].as_u64()) {
                    self.active = u32::try_from(first).ok();
                }
                Ok(Some(json!({ "family": file.family, "opened": opened })))
            }
            Command::SaveFamily { path, ids, force } => {
                let members = self.family_members(ids.as_deref())?;
                let written = save_family(&members, Path::new(&path), force)?;
                let family_name = members.first().map(|font| font.style.family.clone());
                for doc in &mut self.docs {
                    if Some(&doc.font.style.family) == family_name.as_ref() {
                        doc.dirty = false;
                    }
                }
                Ok(Some(json!({ "path": path, "styles": paths(&written) })))
            }
            Command::ExportFamily {
                dir,
                format,
                ids,
                force,
            } => {
                let members = self.family_members(ids.as_deref())?;
                let issues = check_family(&members);
                let written = export_family(&members, Path::new(&dir), format, force)?;
                Ok(Some(json!({ "files": paths(&written), "issues": issues })))
            }
            Command::FamilyCheck { ids } => {
                let members = self.family_members(ids.as_deref())?;
                let issues = check_family(&members);
                let styles: Vec<&str> = members
                    .iter()
                    .map(|font| font.style.name.as_str())
                    .collect();
                Ok(Some(json!({
                    "styles": styles,
                    "ready": issues.iter().all(|issue| !issue.blocking),
                    "issues": issues,
                })))
            }
            Command::SetInfo {
                copyright,
                designer,
                license,
                license_url,
                version,
                vendor,
                unique_id,
            } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.set_info(InfoUpdate {
                    copyright,
                    designer,
                    license,
                    license_url,
                    version,
                    vendor,
                    unique_id,
                })?;
                Ok(Some(json!({ "info": font.info })))
            }
            Command::SetKerning { kerning } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.set_kerning(kerning)?;
                Ok(Some(kerning_summary(font)))
            }
            Command::AddKern { left, right, value } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.add_kern_pair(left.clone(), right.clone(), value)?;
                Ok(Some(
                    json!({ "left": left, "right": right, "value": value }),
                ))
            }
            Command::SetGroup { name, members } => {
                let count = members.len();
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.set_kern_group(name.clone(), members)?;
                Ok(Some(json!({ "name": name, "members": count })))
            }
            Command::AddLigature { glyphs, name } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.add_ligature(glyphs, name.clone())?;
                Ok(Some(json!({ "name": name })))
            }
            Command::SetFeatures { text } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.set_features(text);
                Ok(Some(json!({ "features": font.features.is_some() })))
            }
            Command::Offset {
                horizontal,
                vertical,
                gap,
                corner,
                sidebearing,
                keep_metrics,
                add_points,
                names,
                preview,
            } => self.offset(
                OffsetOptions {
                    horizontal,
                    vertical,
                    gap,
                    corner,
                    sidebearing,
                    keep_metrics,
                    add_points,
                    names,
                },
                None,
                preview,
            ),
            Command::Stroke {
                kind,
                horizontal,
                vertical,
                gap,
                corner,
                sidebearing,
                keep_metrics,
                add_points,
                names,
                preview,
            } => self.offset(
                OffsetOptions {
                    horizontal,
                    vertical,
                    gap,
                    corner,
                    sidebearing,
                    keep_metrics,
                    add_points,
                    names,
                },
                Some(kind),
                preview,
            ),
            Command::SetSidebearing {
                name,
                side,
                value,
                family,
                ids,
            } => self.set_sidebearing(name, side, value, family, ids),
            Command::CheckOutlines => {
                let font = self.font().ok_or(FoundryError::NoFont)?;
                let issues = check_outlines(font);
                Ok(Some(json!({ "issues": issues, "count": issues.len() })))
            }
            Command::CheckSpacing { min_gap, pairs } => {
                let font = self.font().ok_or(FoundryError::NoFont)?;
                let issues = check_spacing(font, min_gap, pairs.as_deref())?;
                Ok(Some(json!({ "issues": issues, "count": issues.len() })))
            }
            Command::Diff { a, b } => {
                let before = Font::load(Path::new(&a))?;
                let after = Font::load(Path::new(&b))?;
                let diffs = diff_fonts(&before, &after);
                Ok(Some(json!({ "glyphs": diffs, "count": diffs.len() })))
            }
            Command::Proof {
                path,
                text,
                pixel_size,
                guides,
                boxes,
                compare,
                force,
            } => {
                let compare = match compare {
                    Some(compare) => Some(Font::load(Path::new(&compare))?),
                    None => None,
                };
                let font = self.font().ok_or(FoundryError::NoFont)?;
                let (width, height) = write_proof(
                    font,
                    Path::new(&path),
                    &ProofOptions {
                        text,
                        pixel_size,
                        guides,
                        boxes,
                        compare,
                        force,
                    },
                )?;
                Ok(Some(
                    json!({ "path": path, "width": width, "height": height }),
                ))
            }
            Command::MoveGlyph { name, index } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.move_glyph(&name, index)?;
                Ok(Some(json!({ "name": name, "index": index })))
            }
            Command::CopyFamily {
                path,
                dir,
                family,
                label,
                force,
            } => {
                let written = copy_family(
                    Path::new(&path),
                    Path::new(&dir),
                    family.as_deref(),
                    label.as_deref(),
                    force,
                )?;
                Ok(Some(json!({ "path": written })))
            }
            Command::Slant { degrees } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                font.slant(degrees)?;
                Ok(Some(json!({
                    "italic": font.style.italic,
                    "italic_angle": font.style.italic_angle,
                })))
            }
            Command::ScaleWidth { factor, names } => {
                let font = self.font_mut().ok_or(FoundryError::NoFont)?;
                let changed = font.scale_width(factor, names.as_deref())?;
                Ok(Some(json!({ "glyphs": changed, "factor": factor })))
            }
            Command::History => self.history_data(),
            Command::Check { a, b } => compare_files(&a, &b),
            Command::Blend {
                a,
                b,
                t,
                out,
                force,
            } => {
                let left = Font::load(Path::new(&a))?;
                let right = Font::load(Path::new(&b))?;
                match blend_fonts(&left, &right, t) {
                    Ok(font) => {
                        font.save_with(Path::new(&out), force)?;
                        let data = json!({
                            "path": out,
                            "name": font.name,
                            "glyphs": font.glyph_names(),
                        });
                        let id = self.add_font(font, false);
                        let mut data = data;
                        data["id"] = json!(id);
                        Ok(Some(data))
                    }
                    Err(issues) => {
                        let message = issues
                            .iter()
                            .map(|issue| issue.detail.clone())
                            .collect::<Vec<_>>()
                            .join("; ");
                        Err(Fail {
                            message,
                            data: Some(json!({ "compatible": false, "issues": issues })),
                        })
                    }
                }
            }
        }
    }
}

impl Session {
    fn history_data(&self) -> Result<Option<Value>, Fail> {
        let (undo, redo) = self.history();
        Ok(Some(json!({ "undo": undo, "redo": redo })))
    }

    /// What `create`, `open`, and `info` return for one open font.
    fn summary(&self, id: u32) -> Value {
        let Some(doc) = self.docs.iter().find(|doc| doc.id == id) else {
            return Value::Null;
        };
        let font = &doc.font;
        let encoded = font
            .glyphs
            .iter()
            .filter(|glyph| glyph.unicode.is_some())
            .count();
        json!({
            "id": id,
            "name": font.name,
            "upm": font.upm,
            "metrics": font.metrics,
            "style": font.style,
            "info": font.info,
            "coverage": { "glyphs": font.glyphs.len(), "encoded": encoded },
            "kerning": kerning_summary(font),
            "glyphs": font.glyph_names(),
        })
    }
}

fn kerning_summary(font: &Font) -> Value {
    let kerning = font.kerning.as_ref();
    json!({
        "pairs": kerning.map(|kerning| kerning.pairs.len()).unwrap_or(0),
        "groups": kerning.map(|kerning| kerning.groups.len()).unwrap_or(0),
        "ligatures": kerning.map(|kerning| kerning.ligatures.len()).unwrap_or(0),
    })
}

fn apply_offset(
    font: &mut Font,
    options: &OffsetOptions,
    kind: Option<StrokeKind>,
) -> Result<Vec<String>, FoundryError> {
    match kind {
        Some(kind) => stroke_font(font, options, kind),
        None => offset_font(font, options),
    }
}

fn preview_glyphs(font: &Font, names: &[String]) -> Value {
    let glyphs: Vec<Value> = names
        .iter()
        .filter_map(|name| font.glyph(name))
        .filter_map(|glyph| serde_json::to_value(glyph).ok())
        .collect();
    json!({ "preview": true, "glyphs": glyphs })
}

/// serde already names the missing field. Say that in a sentence a script can show.
fn explain_command_error(err: &serde_json::Error) -> String {
    let text = err.to_string();
    if let Some(rest) = text.strip_prefix("missing field `")
        && let Some(end) = rest.find('`')
    {
        let field = &rest[..end];
        return format!("the command is missing the field `{field}`");
    }
    format!("could not read command: {text}")
}

fn paths(written: &[std::path::PathBuf]) -> Vec<String> {
    written
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

fn compare_files(a: &str, b: &str) -> Result<Option<Value>, Fail> {
    let left = Font::load(Path::new(a))?;
    let right = Font::load(Path::new(b))?;
    let issues = compatibility(&left, &right);
    let compatible = issues.is_empty();
    let message = issues
        .iter()
        .map(|issue| issue.detail.clone())
        .collect::<Vec<_>>()
        .join("; ");
    let data = json!({
        "compatible": compatible,
        "issues": issues,
    });
    if compatible {
        Ok(Some(data))
    } else {
        Err(Fail {
            message,
            data: Some(data),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_core::{Contour, Point, PointKind};
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_IDS: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = TEMP_IDS.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("typefoundry-{tick}-{id}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn square(name: &str, x: f64, advance: f64) -> Glyph {
        Glyph {
            name: name.to_string(),
            unicode: Some(72),
            advance,
            contours: vec![Contour {
                closed: true,
                points: vec![
                    point(x, 0.0),
                    point(x + 80.0, 0.0),
                    point(x + 80.0, 120.0),
                    point(x, 120.0),
                ],
            }],
        }
    }

    fn point(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            kind: PointKind::On,
            smooth: false,
        }
    }

    #[test]
    fn create_put_save_open_and_blend() {
        let dir = temp_dir();
        let narrow_path = dir.join("narrow.json");
        let wide_path = dir.join("wide.json");
        let mid_path = dir.join("mid.json");

        let mut session = Session::new();
        assert!(
            session
                .execute(Command::Create {
                    name: "Narrow".into(),
                    upm: 1000
                })
                .ok
        );
        assert!(
            session
                .execute(Command::PutGlyph {
                    glyph: square("H", 40.0, 400.0),
                })
                .ok
        );
        assert!(
            session
                .execute(Command::Save {
                    path: narrow_path.to_string_lossy().into_owned(),
                    force: false,
                })
                .ok
        );

        assert!(
            session
                .execute(Command::Create {
                    name: "Wide".into(),
                    upm: 1000
                })
                .ok
        );
        assert!(
            session
                .execute(Command::PutGlyph {
                    glyph: square("H", 140.0, 800.0),
                })
                .ok
        );
        assert!(
            session
                .execute(Command::Save {
                    path: wide_path.to_string_lossy().into_owned(),
                    force: false,
                })
                .ok
        );

        let blended = session.execute(Command::Blend {
            a: narrow_path.to_string_lossy().into_owned(),
            b: wide_path.to_string_lossy().into_owned(),
            t: 0.5,
            out: mid_path.to_string_lossy().into_owned(),
            force: false,
        });
        assert!(blended.ok, "{blended:?}");
        let opened = Font::load(&mid_path).unwrap();
        assert_eq!(opened.glyph("H").unwrap().advance, 600.0);
        assert_eq!(opened.glyph("H").unwrap().contours[0].points[0].x, 90.0);
        assert_eq!(session.font().unwrap().name, opened.name);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn moves_a_point_in_an_open_ufo_and_saves_json() {
        let dir = temp_dir();
        let ufo_path = dir.join("wide.ufo");
        let json_path = dir.join("wide.json");
        let mut session = Session::new();
        assert!(
            session
                .execute(Command::Create {
                    name: "Wide".into(),
                    upm: 1000,
                })
                .ok
        );
        assert!(
            session
                .execute(Command::PutGlyph {
                    glyph: square("H", 40.0, 400.0),
                })
                .ok
        );
        let saved = session.execute(Command::Save {
            path: ufo_path.to_string_lossy().into_owned(),
            force: false,
        });
        assert!(saved.ok, "{saved:?}");
        let moved = session.execute(Command::MovePoint {
            name: "H".into(),
            contour: 0,
            point: 0,
            x: 55.0,
            y: 5.0,
        });
        assert!(moved.ok, "{moved:?}");
        let saved_json = session.execute(Command::Save {
            path: json_path.to_string_lossy().into_owned(),
            force: false,
        });
        assert!(saved_json.ok, "{saved_json:?}");
        let opened = Font::load(&json_path).unwrap();
        assert_eq!(opened.glyph("H").unwrap().contours[0].points[0].x, 55.0);
        assert_eq!(opened.glyph("H").unwrap().contours[0].points[0].y, 5.0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_open_keeps_the_current_font() {
        let mut session = Session::new();
        session.execute(Command::Create {
            name: "Kept".into(),
            upm: 1000,
        });
        let response = session.execute(Command::Open {
            path: "does-not-exist.json".into(),
        });
        assert!(!response.ok);
        assert_eq!(session.font().unwrap().name, "Kept");
    }

    #[test]
    fn command_line_skips_comments_and_reports_bad_json() {
        let mut session = Session::new();
        assert!(session.execute_line("").is_none());
        assert!(session.execute_line("# note").is_none());
        let bad = session.execute_line("{").unwrap();
        assert!(!bad.ok);
        let created = session
            .execute_line("\u{feff}{\"op\":\"create\",\"name\":\"From Line\"}")
            .unwrap();
        assert!(created.ok);
        assert_eq!(
            created.data.unwrap()["upm"],
            json!(1000),
            "omitted upm defaults to 1000"
        );
    }

    fn line(session: &mut Session, text: &str) -> Response {
        session.execute_line(text).unwrap()
    }

    fn first_x(session: &Session) -> f64 {
        session.font().unwrap().glyph("H").unwrap().contours[0].points[0].x
    }

    #[test]
    fn undo_and_redo_with_drags_as_one_step() {
        let mut session = Session::new();
        session.execute(Command::Create {
            name: "Undo".into(),
            upm: 1000,
        });
        session.execute(Command::PutGlyph {
            glyph: square("H", 0.0, 400.0),
        });
        assert_eq!(session.history(), (1, 0));

        // A drag: three moves of the same point are one undo step.
        for x in [10.0, 20.0, 30.0] {
            assert!(
                session
                    .execute(Command::MovePoint {
                        name: "H".into(),
                        contour: 0,
                        point: 0,
                        x,
                        y: 0.0,
                    })
                    .ok
            );
        }
        assert_eq!(session.history(), (2, 0));
        // A checkpoint starts a new step even for the same point.
        assert!(line(&mut session, r#"{"op":"checkpoint"}"#).ok);
        line(
            &mut session,
            r#"{"op":"move_points","name":"H","points":[[0,0]],"dx":5,"dy":0}"#,
        );
        assert_eq!(first_x(&session), 35.0);
        assert_eq!(session.history(), (3, 0));

        assert!(line(&mut session, r#"{"op":"undo"}"#).ok);
        assert_eq!(first_x(&session), 30.0);
        // Two points land in one undo step.
        assert!(
            line(
                &mut session,
                r#"{"op":"set_points","name":"H","points":[{"contour":0,"point":0,"x":3,"y":4},{"contour":0,"point":1,"x":9,"y":8}]}"#,
            )
            .ok
        );
        assert_eq!(first_x(&session), 3.0);
        assert!(line(&mut session, r#"{"op":"undo"}"#).ok);
        assert_eq!(first_x(&session), 30.0);
        assert!(line(&mut session, r#"{"op":"undo"}"#).ok);
        assert_eq!(first_x(&session), 0.0);
        let history = line(&mut session, r#"{"op":"redo"}"#);
        assert_eq!(history.data.unwrap(), json!({ "undo": 2, "redo": 1 }));
        assert_eq!(first_x(&session), 30.0);

        // A new edit clears redo. A failed edit adds no step.
        line(
            &mut session,
            r#"{"op":"set_unicode","name":"H","unicode":104}"#,
        );
        assert_eq!(session.history(), (3, 0));
        assert!(!line(&mut session, r#"{"op":"delete_glyph","name":"Q"}"#).ok);
        assert_eq!(session.history(), (3, 0));
        assert!(!line(&mut session, r#"{"op":"redo"}"#).ok);

        // Opening or creating a font starts a fresh history.
        session.execute(Command::Create {
            name: "Fresh".into(),
            upm: 1000,
        });
        assert_eq!(session.history(), (0, 0));
        assert!(!line(&mut session, r#"{"op":"undo"}"#).ok);
    }

    #[test]
    fn edit_commands_read_from_json() {
        let mut session = Session::new();
        session.execute(Command::Create {
            name: "Lines".into(),
            upm: 1000,
        });
        session.execute(Command::PutGlyph {
            glyph: square("H", 0.0, 400.0),
        });
        for text in [
            r#"{"op":"insert_point","name":"H","contour":0,"index":1,"x":40,"y":0}"#,
            r#"{"op":"set_point","name":"H","contour":0,"point":1,"kind":"off"}"#,
            r#"{"op":"split_segment","name":"H","contour":0,"point":3,"t":0.5}"#,
            r#"{"op":"add_contour","name":"H","contour":{"closed":false,"points":[{"x":1,"y":2,"kind":"on","smooth":false}]}}"#,
            r#"{"op":"set_closed","name":"H","contour":1,"closed":true}"#,
            r#"{"op":"reverse_contour","name":"H","contour":0}"#,
            r#"{"op":"delete_points","name":"H","points":[[1,0]]}"#,
            r#"{"op":"transform","names":["H"],"matrix":[1,0,0.2,1,0,0]}"#,
            r#"{"op":"transform","matrix":[-1,0,0,1,0,0],"anchor":"advance"}"#,
            r#"{"op":"round_coordinates"}"#,
            r#"{"op":"set_metrics","x_height":510}"#,
            r#"{"op":"rename_glyph","name":"H","new_name":"Eta"}"#,
            r#"{"op":"rename_font","name":"Lines Two"}"#,
        ] {
            let response = line(&mut session, text);
            assert!(response.ok, "{text}: {response:?}");
        }
        let font = session.font().unwrap();
        assert_eq!(font.name, "Lines Two");
        assert_eq!(font.metrics.x_height, 510.0);
        let glyph = font.glyph("Eta").unwrap();
        assert_eq!(glyph.contours.len(), 1, "the one-point contour was deleted");
        assert!(glyph.contours[0].points.iter().all(|p| p.x.fract() == 0.0));

        let index = line(&mut session, r#"{"op":"index"}"#).data.unwrap();
        assert_eq!(index["glyphs"][0]["name"], json!("Eta"));
        assert_eq!(index["glyphs"][0]["unicode"], json!(72));
        assert_eq!(index["glyphs"][0]["contours"], json!(1));
    }

    #[test]
    fn a_family_of_open_fonts() {
        let dir = temp_dir();
        let mut session = Session::new();
        let created = line(&mut session, r#"{"op":"create","name":"Wide"}"#);
        let regular = created.data.unwrap()["id"].as_u64().unwrap();
        session.execute(Command::PutGlyph {
            glyph: square("H", 0.0, 400.0),
        });

        // Derive an italic: a second open font, active, slanted, with its own history.
        let derived = line(
            &mut session,
            r#"{"op":"derive_style","style":"Italic","slant":12}"#,
        );
        assert!(derived.ok, "{derived:?}");
        let data = derived.data.unwrap();
        let italic = data["id"].as_u64().unwrap();
        assert_ne!(italic, regular);
        assert_eq!(data["style"]["italic_angle"], json!(-12.0));
        assert_eq!(session.font().unwrap().name, "Wide Italic");
        assert_eq!(session.history(), (0, 0));
        // The bottom point sits on the baseline, so the shear leaves it and the recentering
        // shift is what moves it. The ink box stays centred in the advance.
        let centered = -120.0 * 12.0_f64.to_radians().tan() / 2.0;
        assert!((first_x(&session) - centered).abs() < 1e-9);

        line(
            &mut session,
            r#"{"op":"move_points","name":"H","points":[[0,0]],"dx":7,"dy":0}"#,
        );
        assert_eq!(session.history(), (1, 0));

        // The regular is untouched and keeps its own undo step from put_glyph.
        let selected = line(
            &mut session,
            &format!(r#"{{"op":"select_font","id":{regular}}}"#),
        );
        assert!(selected.ok);
        assert_eq!(first_x(&session), 0.0);
        assert_eq!(session.history(), (1, 0));
        let other = line(
            &mut session,
            &format!(r#"{{"op":"glyph","name":"H","font":{italic}}}"#),
        );
        let italic_x = other.data.unwrap()["contours"][0]["points"][0]["x"]
            .as_f64()
            .unwrap();
        assert!((italic_x - (centered + 7.0)).abs() < 1e-9);

        let listed = line(&mut session, r#"{"op":"fonts"}"#).data.unwrap();
        assert_eq!(listed["fonts"].as_array().unwrap().len(), 2);
        assert_eq!(listed["fonts"][1]["style"], json!("Italic"));
        assert_eq!(listed["fonts"][1]["dirty"], json!(true));
        assert_eq!(listed["active"], json!(regular));

        let check = line(&mut session, r#"{"op":"family_check"}"#).data.unwrap();
        assert_eq!(check["styles"], json!(["Regular", "Italic"]));
        assert_eq!(check["ready"], json!(true));

        let family = dir
            .join("Wide.family.json")
            .to_string_lossy()
            .replace('\\', "/");
        let saved = line(
            &mut session,
            &format!(r#"{{"op":"save_family","path":"{family}"}}"#),
        );
        assert!(saved.ok, "{saved:?}");
        let listed = line(&mut session, r#"{"op":"fonts"}"#).data.unwrap();
        assert_eq!(listed["fonts"][1]["dirty"], json!(false));

        let out = dir.join("ttf").to_string_lossy().replace('\\', "/");
        let exported = line(
            &mut session,
            &format!(r#"{{"op":"export_family","dir":"{out}","format":"ttf"}}"#),
        );
        assert!(exported.ok, "{exported:?}");
        let files = exported.data.unwrap()["files"].clone();
        assert!(files[1].as_str().unwrap().ends_with("Wide-Italic.ttf"));

        // Close both, then open the family file: both styles come back.
        assert!(line(&mut session, r#"{"op":"close_font"}"#).ok);
        assert_eq!(session.font().unwrap().style.name, "Italic");
        assert!(line(&mut session, r#"{"op":"close_font"}"#).ok);
        assert!(session.font().is_none());
        let opened = line(
            &mut session,
            &format!(r#"{{"op":"open_family","path":"{family}"}}"#),
        );
        assert!(opened.ok, "{opened:?}");
        assert_eq!(opened.data.unwrap()["opened"].as_array().unwrap().len(), 2);
        assert_eq!(session.font().unwrap().style.name, "Regular");

        let restyled = line(
            &mut session,
            r#"{"op":"set_style","style":"Bold","weight":700}"#,
        );
        assert!(restyled.ok);
        assert_eq!(session.font().unwrap().name, "Wide Bold");
        assert!(line(&mut session, r#"{"op":"undo"}"#).ok);
        assert_eq!(session.font().unwrap().style.name, "Regular");
        assert!(!line(&mut session, r#"{"op":"select_font","id":999}"#).ok);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_refuses_an_existing_file_unless_forced() {
        let dir = temp_dir();
        let file = dir.join("wide.json").to_string_lossy().replace('\\', "/");
        let mut session = Session::new();
        session.execute(Command::Create {
            name: "Wide".into(),
            upm: 1000,
        });
        assert!(
            session
                .execute(Command::Save {
                    path: file.clone(),
                    force: false,
                })
                .ok
        );
        let refused = session.execute(Command::Save {
            path: file.clone(),
            force: false,
        });
        assert!(!refused.ok);
        assert!(refused.error.unwrap().contains("force"));
        assert!(
            session
                .execute(Command::Save {
                    path: file,
                    force: true,
                })
                .ok
        );
        assert!(dir.join("wide.json.bak").is_file());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_point_field_names_the_field() {
        let mut session = Session::new();
        let response = session
            .execute_line(
                r#"{"op":"put_glyph","glyph":{"name":"H","advance":100,"contours":[{"closed":true,"points":[{"x":0,"y":0,"kind":"on"}]}]}}"#,
            )
            .unwrap();
        assert!(!response.ok);
        let error = response.error.unwrap();
        assert!(error.contains("`smooth`"), "{error}");
    }

    #[test]
    fn offset_is_one_undo_and_a_preview_leaves_the_font() {
        let mut session = Session::new();
        session.execute(Command::Create {
            name: "Offset".into(),
            upm: 1000,
        });
        assert!(
            session
                .execute(Command::PutGlyph {
                    glyph: square("H", 100.0, 400.0),
                })
                .ok
        );
        let before = session.font().unwrap().glyph("H").unwrap().contours[0].points[0].x;
        let preview = line(
            &mut session,
            r#"{"op":"offset","horizontal":15,"vertical":4,"names":["H"],"preview":true}"#,
        );
        assert!(preview.ok, "{preview:?}");
        assert_eq!(preview.data.unwrap()["preview"], json!(true));
        assert_eq!(
            session.font().unwrap().glyph("H").unwrap().contours[0].points[0].x,
            before
        );
        assert_eq!(session.history(), (1, 0));
        let applied = line(
            &mut session,
            r#"{"op":"offset","horizontal":15,"vertical":4,"names":["H"]}"#,
        );
        assert!(applied.ok, "{applied:?}");
        let moved = session.font().unwrap().glyph("H").unwrap().contours[0].points[0].x;
        assert!(moved < before, "{moved}");
        assert_eq!(
            session.font().unwrap().glyph("H").unwrap().contours[0]
                .points
                .len(),
            4
        );
        assert_eq!(
            session.font().unwrap().glyph("H").unwrap().contours[0].points[0].y,
            0.0
        );
        assert!(line(&mut session, r#"{"op":"undo"}"#).ok);
        assert_eq!(
            session.font().unwrap().glyph("H").unwrap().contours[0].points[0].x,
            before
        );
    }

    #[test]
    fn a_family_sidebearing_sets_every_open_style() {
        let mut session = Session::new();
        line(&mut session, r#"{"op":"create","name":"Wide"}"#);
        line(
            &mut session,
            r#"{"op":"put_glyph","glyph":{"name":"f","advance":300,"contours":[{"closed":true,"points":[{"x":40,"y":0,"kind":"on","smooth":false},{"x":140,"y":0,"kind":"on","smooth":false},{"x":140,"y":100,"kind":"on","smooth":false},{"x":40,"y":100,"kind":"on","smooth":false}]}]}}"#,
        );
        assert!(
            line(
                &mut session,
                r#"{"op":"set_style","family":"Wide","style":"Regular"}"#,
            )
            .ok
        );
        assert!(
            line(
                &mut session,
                r#"{"op":"derive_style","style":"Bold","weight":700}"#,
            )
            .ok
        );
        let set = line(
            &mut session,
            r#"{"op":"set_sidebearing","name":"f","side":"right","value":32,"family":true}"#,
        );
        assert!(set.ok, "{set:?}");
        assert_eq!(set.data.unwrap()["styles"], json!(2));
        assert!((session.font().unwrap().glyph("f").unwrap().advance - 172.0).abs() < 0.01);
        assert!(line(&mut session, r#"{"op":"undo"}"#).ok);
        assert!((session.font().unwrap().glyph("f").unwrap().advance - 300.0).abs() < 0.01);
        assert!(line(&mut session, r#"{"op":"select_font","id":1}"#).ok);
        assert!((session.font().unwrap().glyph("f").unwrap().advance - 172.0).abs() < 0.01);
        assert!(line(&mut session, r#"{"op":"undo"}"#).ok);
        assert!((session.font().unwrap().glyph("f").unwrap().advance - 300.0).abs() < 0.01);
    }
}
