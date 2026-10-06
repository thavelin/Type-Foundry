use std::fmt;

/// A document or file error. Blend mismatches are [`crate::CompatIssue`] values, not this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoundryError {
    Name,
    Upm(u16),
    Format(String),
    Version(u32),
    GlyphName,
    DuplicateGlyph(String),
    DuplicateUnicode {
        code: u32,
        glyph: String,
        other: String,
    },
    Exists(String),
    EmptyContour(String),
    NonFinite,
    MissingGlyph(String),
    MissingPoint,
    NoFont,
    Ufo(String),
    Ttf(String),
    Import(String),
    Edit(String),
    Style(String),
    Family(String),
    Io(String),
    Json(String),
}

impl fmt::Display for FoundryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name => write!(f, "font name is empty"),
            Self::Upm(upm) => write!(
                f,
                "units per em {upm} is outside {}-{}",
                crate::MIN_UPM,
                crate::MAX_UPM
            ),
            Self::Format(found) => write!(f, "unknown font format {found}"),
            Self::Version(found) => write!(f, "unsupported font version {found}"),
            Self::GlyphName => write!(f, "glyph name is empty"),
            Self::DuplicateGlyph(name) => write!(f, "duplicate glyph {name}"),
            Self::DuplicateUnicode { code, glyph, other } => {
                write!(f, "U+{code:04X} is used by both {other} and {glyph}")
            }
            Self::Exists(path) => write!(
                f,
                "{path} already exists. Pass force to overwrite it. A copy is kept beside the file, with .bak added to the name."
            ),
            Self::EmptyContour(name) => write!(f, "glyph {name} has a contour with no points"),
            Self::NonFinite => write!(f, "a coordinate or advance is not a finite number"),
            Self::MissingGlyph(name) => write!(f, "glyph {name} is not in the open font"),
            Self::MissingPoint => write!(f, "that point is not in the glyph"),
            Self::NoFont => write!(f, "no font is open"),
            Self::Ufo(message) => write!(f, "{message}"),
            Self::Ttf(message) => write!(f, "{message}"),
            Self::Import(message) => write!(f, "{message}"),
            Self::Edit(message) => write!(f, "{message}"),
            Self::Style(message) => write!(f, "{message}"),
            Self::Family(message) => write!(f, "{message}"),
            Self::Io(message) => write!(f, "{message}"),
            Self::Json(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for FoundryError {}
