//! Font documents and the blend operation that builds a new face from two compatible ones.

pub mod analyze;
pub mod atlas;
mod blend;
mod critique;
mod diff;
mod edit;
mod error;
mod family;
mod font;
mod genome;
mod geometry;
mod group;
mod import;
mod kerning;
mod offset;
mod outline;
mod proof;
mod save;
mod sfnt;
mod smooth;
mod stroke;
mod study;
pub use stroke::{
    Preset as StrokePresetOptions, Stroke, StrokeOptions, StrokePreset, WidthPoint, expand_strokes,
    stroke_counts, stroke_edit, validate_stroke,
};
mod svgfont;
mod symmetry;
mod ttf;
mod typeface;
mod ufo;
mod vostok_tests;
mod webfont;

pub use blend::{CompatIssue, blend_fonts, compatibility};
pub use critique::{
    CritiqueLayer, CritiqueSuggestion, critique_font, filter_rejected, resolve_critique,
    resolve_critique_with,
};
pub use diff::{GlyphDiff, diff_fonts};
pub use edit::{Anchor, Matrix, MetricsUpdate, Side, glyph_stem_widths};
pub use error::FoundryError;
pub use family::{
    ExportFormat, FAMILY_FORMAT, FAMILY_VERSION, FamilyFile, FamilyIssue, StyleUpdate,
    check_family, copy_family, export_family, load_family, save_family,
};
pub use font::{
    Contour, FONT_FORMAT, FONT_VERSION, Font, FontInfo, Glyph, InfoUpdate, KernPair, Kerning,
    Ligature, MAX_UPM, MIN_UPM, Metrics, Point, PointKind, Style,
};
pub use genome::{
    DECISIONS_KEY, DesignDecision, GENOME_KEY, GenomeIssue, STEM_SAMPLES, StyleGenome, audit_font,
    capture_genome, check_genome, list_decisions, measure_font, record_decision, stored_genome,
};
pub use geometry::{ItalicMode, ValidityIssue, measure_stems_ray, outline_valid};
pub use group::{GlyphGroup, classify};
pub use offset::{
    Corner, OffsetMode, OffsetOptions, OffsetReport, StrokeKind, ZoneOptions, offset_font,
    offset_font_detailed, stroke_font,
};
pub use outline::{OutlineIssue, SpacingIssue, check_outlines, check_spacing};
pub use proof::{ProofOptions, write_proof};
pub use smooth::{
    SmoothOptions, SmoothTarget, SmoothnessReport, check_smoothness, smooth_outlines,
};
pub use study::{
    FamilyStudy, LetterformStudy, ListedStudy, ProportionFit, STUDY_FORMAT, STUDY_VERSION,
    StudyFailure, StudyLibrary, apply_study, list_studies, read_study, resolve_study, select_face,
    shipped_studies_dir, study_font, study_path, user_studies_dir, write_study,
};
pub use symmetry::{
    AxisSpec, MirrorOptions, SymmetrizeOptions, check_symmetry, glyph_from_mirror, mirror_glyphs,
    symmetrize_glyphs,
};

pub use ttf::{TtfOptions, TtfReport};
