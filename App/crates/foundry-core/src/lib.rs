//! Font documents and the blend operation that builds a new face from two compatible ones.

mod blend;
mod diff;
mod edit;
mod error;
mod family;
mod font;
mod import;
mod kerning;
mod offset;
mod outline;
mod proof;
mod save;
mod sfnt;
mod svgfont;
mod ttf;
mod typeface;
mod ufo;
mod webfont;

pub use blend::{CompatIssue, blend_fonts, compatibility};
pub use diff::{GlyphDiff, diff_fonts};
pub use edit::{Anchor, Matrix, MetricsUpdate, Side};
pub use error::FoundryError;
pub use family::{
    ExportFormat, FAMILY_FORMAT, FAMILY_VERSION, FamilyFile, FamilyIssue, StyleUpdate,
    check_family, copy_family, export_family, load_family, save_family,
};
pub use font::{
    Contour, FONT_FORMAT, FONT_VERSION, Font, FontInfo, Glyph, InfoUpdate, KernPair, Kerning,
    Ligature, MAX_UPM, MIN_UPM, Metrics, Point, PointKind, Style,
};
pub use offset::{Corner, OffsetOptions, StrokeKind, offset_font, stroke_font};
pub use outline::{OutlineIssue, SpacingIssue, check_outlines, check_spacing};
pub use proof::{ProofOptions, write_proof};
