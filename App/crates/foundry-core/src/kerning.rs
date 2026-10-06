//! Kerning groups expand to glyph pairs. A glyph-to-glyph pair wins over a group pair.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::FoundryError;
use crate::font::{Font, Kerning, Ligature};

/// Glyph-to-glyph kerning, most specific pair winning. Zero values are dropped.
pub(crate) fn resolved_pairs(font: &Font) -> Result<Vec<(String, String, f64)>, FoundryError> {
    let Some(kerning) = &font.kerning else {
        return Ok(Vec::new());
    };
    let known: BTreeSet<&str> = font
        .glyphs
        .iter()
        .map(|glyph| glyph.name.as_str())
        .collect();
    let mut best: BTreeMap<(String, String), (u8, f64)> = BTreeMap::new();
    for pair in &kerning.pairs {
        if !pair.value.is_finite() {
            return Err(FoundryError::NonFinite);
        }
        let left_group = kerning.groups.contains_key(&pair.left);
        let right_group = kerning.groups.contains_key(&pair.right);
        let lefts = expand(&pair.left, left_group, kerning, &known)?;
        let rights = expand(&pair.right, right_group, kerning, &known)?;
        let rank = match (left_group, right_group) {
            (false, false) => 3,
            (false, true) => 2,
            (true, false) => 1,
            (true, true) => 0,
        };
        for left in &lefts {
            for right in &rights {
                let key = (left.clone(), right.clone());
                if best.get(&key).is_none_or(|(old, _)| *old <= rank) {
                    best.insert(key, (rank, pair.value));
                }
            }
        }
    }
    Ok(best
        .into_iter()
        .filter(|(_, (_, value))| *value != 0.0)
        .map(|((left, right), (_, value))| (left, right, value))
        .collect())
}

fn expand(
    name: &str,
    is_group: bool,
    kerning: &Kerning,
    known: &BTreeSet<&str>,
) -> Result<Vec<String>, FoundryError> {
    if is_group {
        let members = kerning
            .groups
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or_default();
        return Ok(members
            .iter()
            .filter(|member| known.contains(member.as_str()))
            .cloned()
            .collect());
    }
    if known.contains(name) {
        Ok(vec![name.to_string()])
    } else {
        Err(FoundryError::Edit(format!(
            "kerning mentions {name}, which is not a glyph or a group"
        )))
    }
}

/// `feature liga` text for the ligatures stored on the font. Empty when there are none.
pub(crate) fn liga_feature(ligatures: &[Ligature]) -> String {
    if ligatures.is_empty() {
        return String::new();
    }
    let mut lines = vec!["feature liga {".to_string()];
    for liga in ligatures {
        lines.push(format!(
            "    sub {} by {};",
            liga.glyphs.join(" "),
            liga.name
        ));
    }
    lines.push("} liga;".to_string());
    lines.join("\n")
}

/// Feature text to write. A font that never stored features, and has no ligatures, returns
/// `None`, so a UFO merge keeps `features.fea` already on disk. Ligature lines are inserted
/// into an existing `liga` feature, or appended when that feature is absent.
pub(crate) fn features_to_write(font: &Font) -> Option<String> {
    let ligatures = font
        .kerning
        .as_ref()
        .map(|kerning| kerning.ligatures.as_slice())
        .unwrap_or(&[]);
    match &font.features {
        None if ligatures.is_empty() => None,
        None => Some(format!("{}\n", liga_feature(ligatures))),
        Some(text) if ligatures.is_empty() => Some(text.clone()),
        Some(text) => Some(merge_liga(text, ligatures)),
    }
}

fn merge_liga(text: &str, ligatures: &[Ligature]) -> String {
    let missing: Vec<String> = ligatures
        .iter()
        .map(|liga| format!("sub {} by {};", liga.glyphs.join(" "), liga.name))
        .filter(|line| !text.contains(line))
        .collect();
    if missing.is_empty() {
        return text.to_string();
    }
    if let Some(at) = text.rfind("} liga;") {
        let mut out = String::new();
        out.push_str(&text[..at]);
        for line in &missing {
            out.push_str("    ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&text[at..]);
        return out;
    }
    let mut out = text.trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(&liga_feature(ligatures));
    out.push('\n');
    out
}
