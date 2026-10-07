//! Sorts glyphs into the groups the glyph tree shows. The group is read from the Unicode value
//! and from whether a ligature names the glyph. Nothing here is written to the font.

/// The groups in the order the glyph tree lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphGroup {
    Uppercase,
    Lowercase,
    Figures,
    Punctuation,
    Symbols,
    Spaces,
    Marks,
    Ligatures,
    /// Glyphs with no Unicode value, such as `.notdef`, alternates, and small caps.
    Unencoded,
}

impl GlyphGroup {
    pub const ALL: [GlyphGroup; 9] = [
        GlyphGroup::Uppercase,
        GlyphGroup::Lowercase,
        GlyphGroup::Figures,
        GlyphGroup::Punctuation,
        GlyphGroup::Symbols,
        GlyphGroup::Spaces,
        GlyphGroup::Marks,
        GlyphGroup::Ligatures,
        GlyphGroup::Unencoded,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GlyphGroup::Uppercase => "Uppercase",
            GlyphGroup::Lowercase => "Lowercase",
            GlyphGroup::Figures => "Figures",
            GlyphGroup::Punctuation => "Punctuation",
            GlyphGroup::Symbols => "Symbols",
            GlyphGroup::Spaces => "Spaces",
            GlyphGroup::Marks => "Marks",
            GlyphGroup::Ligatures => "Ligatures",
            GlyphGroup::Unencoded => "Unencoded",
        }
    }
}

/// Picks the group for one glyph. A ligature wins over its character. A glyph without a
/// Unicode value is unencoded. The rest follow the character's class: this is a heuristic for
/// the common blocks, not a full Unicode category table.
pub fn classify(unicode: Option<u32>, ligature: bool) -> GlyphGroup {
    if ligature {
        return GlyphGroup::Ligatures;
    }
    let Some(c) = unicode.and_then(char::from_u32) else {
        return GlyphGroup::Unencoded;
    };
    if c.is_control() {
        GlyphGroup::Unencoded
    } else if c.is_whitespace() {
        GlyphGroup::Spaces
    } else if is_mark(c as u32) {
        GlyphGroup::Marks
    } else if c.is_uppercase() {
        GlyphGroup::Uppercase
    } else if c.is_lowercase() {
        GlyphGroup::Lowercase
    } else if c.is_numeric() {
        GlyphGroup::Figures
    } else if is_punctuation(c) {
        GlyphGroup::Punctuation
    } else {
        GlyphGroup::Symbols
    }
}

/// Combining marks in the common blocks.
fn is_mark(code: u32) -> bool {
    matches!(
        code,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F
    )
}

/// ASCII and Latin-1 punctuation, plus the General Punctuation block. Symbols such as `$`, `+`,
/// `|`, `©`, and `°` are left out and fall through to Symbols.
fn is_punctuation(c: char) -> bool {
    matches!(
        c,
        '!' | '"' | '#' | '%' | '&' | '\'' | '(' | ')' | '*' | ',' | '-' | '.' | '/' | ':'
            | ';' | '?' | '@' | '[' | '\\' | ']' | '_' | '{' | '}'
            | '\u{00A1}' | '\u{00A7}' | '\u{00AB}' | '\u{00B6}' | '\u{00B7}' | '\u{00BB}'
            | '\u{00BF}' | '\u{2010}'..='\u{2027}' | '\u{2030}'..='\u{205E}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(c: char) -> GlyphGroup {
        classify(Some(u32::from(c)), false)
    }

    #[test]
    fn letters_figures_and_punctuation_sort_apart() {
        assert_eq!(group('A'), GlyphGroup::Uppercase);
        assert_eq!(group('z'), GlyphGroup::Lowercase);
        assert_eq!(group('7'), GlyphGroup::Figures);
        assert_eq!(group('?'), GlyphGroup::Punctuation);
        assert_eq!(group('\u{2014}'), GlyphGroup::Punctuation);
        assert_eq!(group('$'), GlyphGroup::Symbols);
        assert_eq!(group('\u{00B0}'), GlyphGroup::Symbols);
        assert_eq!(group(' '), GlyphGroup::Spaces);
    }

    #[test]
    fn combining_marks_and_cyrillic_are_grouped() {
        assert_eq!(group('\u{0301}'), GlyphGroup::Marks);
        assert_eq!(group('\u{0416}'), GlyphGroup::Uppercase);
        assert_eq!(group('\u{0436}'), GlyphGroup::Lowercase);
    }

    #[test]
    fn unencoded_and_ligature_glyphs_have_their_own_groups() {
        assert_eq!(classify(None, false), GlyphGroup::Unencoded);
        assert_eq!(classify(Some(0x41), true), GlyphGroup::Ligatures);
        assert_eq!(classify(Some(0x0A), false), GlyphGroup::Unencoded);
    }

    #[test]
    fn every_group_is_listed_once() {
        let mut labels: Vec<&str> = GlyphGroup::ALL.iter().map(|g| g.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), GlyphGroup::ALL.len());
    }
}
