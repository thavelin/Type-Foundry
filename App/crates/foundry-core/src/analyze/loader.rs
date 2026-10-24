use super::*;
use crate::Font;
use std::collections::BTreeSet;
use ttf_parser::{Face, Tag};

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}
fn err(errors: &mut Vec<Value>, path: &Path, error: impl ToString) {
    errors.push(json!({"path":path,"error":error.to_string()}));
}
pub(super) fn discover(
    path: &Path,
    files: &mut Vec<PathBuf>,
    errors: &mut Vec<Value>,
    explicit: bool,
) {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            err(errors, path, e);
            return;
        }
    };
    if meta.file_type().is_symlink() {
        err(
            errors,
            path,
            "symlinks are skipped to avoid recursive loops",
        );
        return;
    }
    let ext = extension(path);
    // Generated studies/cache are not font inputs when scanning a parent directory again.
    if !explicit
        && ext == "json"
        && let Ok(bytes) = fs::read(path)
        && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
    {
        let format = value.get("format").and_then(Value::as_str).or_else(|| {
            value
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.get("format"))
                .and_then(Value::as_str)
        });
        if matches!(
            format,
            Some("typefoundry.study" | "typefoundry.collection-study")
        ) {
            return;
        }
        if value.get("algorithm").is_some() && value.get("collection").is_some() {
            return;
        }
    }
    if meta.is_dir() {
        if ext == "ufo" || path.join("0041.svg").is_file() || path.join("U+0041.svg").is_file() {
            match fs::canonicalize(path) {
                Ok(p) => files.push(p),
                Err(e) => err(errors, path, e),
            };
            return;
        }
        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(e) => {
                err(errors, path, e);
                return;
            }
        };
        let mut children = Vec::new();
        for e in entries {
            match e {
                Ok(e) => children.push(e.path()),
                Err(e) => err(errors, path, e),
            }
        }
        children.sort();
        for p in children {
            if !matches!(
                p.file_name().and_then(|s| s.to_str()),
                Some(".git" | "target")
            ) {
                discover(&p, files, errors, false);
            }
        }
    } else if matches!(
        ext.as_str(),
        "ttf" | "otf" | "ttc" | "otc" | "woff" | "woff2" | "eot" | "json"
    ) {
        match fs::canonicalize(path) {
            Ok(p) => files.push(p),
            Err(e) => err(errors, path, e),
        }
    } else if explicit {
        err(errors, path, "unsupported analysis input");
    }
}
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return fs::canonicalize(path).map_err(|e| e.to_string());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(absolute(parent)?.join(path.file_name().ok_or("invalid output path")?))
}
pub(super) fn check_output(output: &Path, files: &[PathBuf]) -> Result<(), String> {
    let out = absolute(output)?;
    if files
        .iter()
        .any(|p| out == *p || (p.is_dir() && out.starts_with(p)))
    {
        return Err("outputs and caches must not replace or be inside source fonts".into());
    }
    Ok(())
}
fn digest_path(path: &Path) -> Result<String, String> {
    fn add(path: &Path, root: &Path, h: &mut Sha256) -> Result<(), String> {
        let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            return Err("font package contains a symlink".into());
        }
        if meta.is_dir() {
            let mut children = fs::read_dir(path)
                .map_err(|e| e.to_string())?
                .map(|v| v.map(|v| v.path()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            children.sort();
            for p in children {
                add(&p, root, h)?;
            }
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            h.update((name.len() as u64).to_be_bytes());
            h.update(name.as_bytes());
            let data = fs::read(path).map_err(|e| e.to_string())?;
            h.update((data.len() as u64).to_be_bytes());
            h.update(data);
        }
        Ok(())
    }
    if path.is_file() {
        return Ok(hash(&fs::read(path).map_err(|e| e.to_string())?));
    }
    let mut h = Sha256::new();
    add(path, path, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}
pub(super) fn analyze_path(path: &Path, options: &Options) -> Result<Vec<Study>, String> {
    let ext = extension(path);
    if matches!(ext.as_str(), "woff2" | "eot") {
        return Err(format!(
            "{ext} is deliberately unsupported by the analyzer; supply TTF/OTF/WOFF1"
        ));
    }
    let hash = digest_path(path)?;
    let key=super::hash(&serde_json::to_vec(&json!({"hash":hash,"algorithm":ALGORITHM,"version":VERSION,"axes":options.axes,"glyphs":options.glyphs,"weight_match":options.weight_match,"profile":options.profile})).map_err(|e|e.to_string())?);
    let cache = options
        .cache_dir
        .as_ref()
        .map(|p| p.join(format!("{key}.json")));
    if let Some(cache) = &cache
        && let Ok(bytes) = fs::read(cache)
        && let Ok(mut studies) = serde_json::from_slice::<Vec<Study>>(&bytes)
        && !studies.is_empty()
        && studies
            .iter()
            .all(|s| s.algorithm == ALGORITHM && s.version == VERSION && s.source.sha256 == hash)
    {
        for s in &mut studies {
            s.source.path = path.to_string_lossy().into_owned();
        }
        return Ok(studies);
    }
    let mut studies = Vec::new();
    let source = |face, instance, axes| Source {
        path: path.to_string_lossy().into_owned(),
        sha256: hash.clone(),
        face,
        instance,
        axes,
    };
    if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc" | "woff") {
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        if bytes.starts_with(b"wOF2") {
            return Err("WOFF2 is deliberately unsupported by the analyzer (including disguised extensions)".into());
        }
        let bytes = crate::sfnt::sfnt_bytes(&bytes)?;
        let count = ttf_parser::fonts_in_collection(&bytes).unwrap_or(1);
        for i in 0..count {
            let base = Face::parse(&bytes, i).map_err(|e| format!("face {i}: {e}"))?;
            let mut locations = locations(&base)?;
            if !options.axes.is_empty() {
                locations = vec![("custom".into(), options.axes.clone())];
            }
            let mut candidates = Vec::new();
            for (name, coords) in locations {
                let mut face = base.clone();
                for (tag, value) in &coords {
                    let axis = face
                        .variation_axes()
                        .into_iter()
                        .find(|a| a.tag.to_string() == *tag)
                        .ok_or_else(|| format!("unknown axis {tag}"))?;
                    if *value < axis.min_value || *value > axis.max_value {
                        return Err(format!(
                            "{tag}={value} is outside {}..{}",
                            axis.min_value, axis.max_value
                        ));
                    }
                    let tag =
                        Tag::from_bytes(tag.as_bytes().try_into().map_err(|_| "invalid axis tag")?);
                    face.set_variation(tag, *value)
                        .ok_or("could not apply variation")?;
                }
                let mut font = crate::sfnt::read_face_subset(
                    &face,
                    path.file_stem().and_then(|s| s.to_str()).unwrap_or("Font"),
                    if options.glyphs.is_empty() {
                        None
                    } else {
                        Some(&options.glyphs)
                    },
                )?;
                if name != "default" {
                    font.style.name = name.clone();
                }
                if let Some(w) = coords.get("wght") {
                    font.style.weight = w.round().clamp(1., 1000.) as u16;
                }
                let metadata = binary_metadata(&face, &font);
                candidates.push(measure::study(
                    &font,
                    source(i, name, coords),
                    metadata,
                    options,
                ));
            }
            if let Some(target) = options.weight_match {
                candidates.sort_by(|a, b| {
                    let distance = |s: &Study| {
                        s.metrics
                            .get("weight.stem_x_height")
                            .and_then(|m| m.value)
                            .map(|v| (v - target).abs())
                            .unwrap_or(f64::INFINITY)
                    };
                    distance(a)
                        .total_cmp(&distance(b))
                        .then(a.source.instance.cmp(&b.source.instance))
                });
                candidates.truncate(1);
                for s in &mut candidates {
                    s.warnings.push(format!("weight_match={target}: nearest default/named instance; outlines were not scaled. Static fonts retain their actual weight."));
                }
            }
            studies.append(&mut candidates);
        }
    } else {
        if !options.axes.is_empty() {
            return Err("axis locations require a variable binary font".into());
        }
        let font = Font::load(path).map_err(|e| e.to_string())?;
        let metadata = document_metadata(&font);
        let mut study = measure::study(
            &font,
            source(0, "default".into(), BTreeMap::new()),
            metadata,
            options,
        );
        if options.weight_match.is_some() {
            study.warnings.push("static document: weight matching cannot change stem/x-height without editing outlines".into());
        }
        studies.push(study);
    }
    if let Some(cache) = cache {
        fs::create_dir_all(cache.parent().ok_or("invalid cache path")?)
            .map_err(|e| e.to_string())?;
        // Concurrent identical requests can race safely: only create, never replace.
        if !cache.exists() {
            // Another process may populate the same key; an identical write is idempotent.
            let mut cached = studies.clone();
            for study in &mut cached {
                study.source.path.clear();
            }
            super::stable_write(
                &cache,
                &serde_json::to_vec(&cached).map_err(|e| e.to_string())?,
            )?;
        }
    }
    Ok(studies)
}
fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}
fn i16_at(data: &[u8], offset: usize) -> Option<i16> {
    u16_at(data, offset).map(|v| v as i16)
}
type Location = (String, BTreeMap<String, f32>);
fn locations(face: &Face<'_>) -> Result<Vec<Location>, String> {
    let axes: Vec<_> = face.variation_axes().into_iter().collect();
    let defaults = axes
        .iter()
        .map(|a| (a.tag.to_string(), a.def_value))
        .collect();
    let mut out = vec![("default".into(), defaults)];
    let Some(data) = face.raw_face().table(Tag::from_bytes(b"fvar")) else {
        return Ok(out);
    };
    let read = |at| {
        u16_at(data, at)
            .map(usize::from)
            .ok_or("truncated fvar table")
    };
    let start = read(4)?;
    let count = read(8)?;
    let size = read(10)?;
    let instances = read(12)?;
    let instance_size = read(14)?;
    if count != axes.len() || size < 20 || instance_size < 4 + 4 * count {
        return Err("invalid fvar record sizes".into());
    }
    for i in 0..instances {
        let at = start + count * size + i * instance_size;
        let id = u16_at(data, at).ok_or("truncated fvar instance")?;
        let name = crate::sfnt::name_text(face, id).unwrap_or_else(|| format!("instance-{i}"));
        let mut coords = BTreeMap::new();
        for (a, axis) in axes.iter().enumerate() {
            let j = at + 4 + 4 * a;
            let fixed = i32::from_be_bytes(
                data.get(j..j + 4)
                    .ok_or("truncated fvar coordinates")?
                    .try_into()
                    .map_err(|_| "invalid fvar coordinate")?,
            );
            coords.insert(axis.tag.to_string(), fixed as f32 / 65536.);
        }
        out.push((format!("{name} [{i}]"), coords));
    }
    Ok(out)
}
fn coverage(codes: impl IntoIterator<Item = u32>) -> Value {
    let codes: BTreeSet<_> = codes.into_iter().collect();
    let mut blocks = BTreeMap::new();
    let mut assigned: BTreeSet<u32> = BTreeSet::new();
    for line in include_str!("Blocks.txt")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let Some((range, name)) = line.split_once(';') else {
            continue;
        };
        let Some((lo, hi)) = range.trim().split_once("..") else {
            continue;
        };
        let (Ok(lo), Ok(hi)) = (u32::from_str_radix(lo, 16), u32::from_str_radix(hi, 16)) else {
            continue;
        };
        let present: Vec<_> = codes.range(lo..=hi).copied().collect();
        if !present.is_empty() {
            assigned.extend(present.iter().copied());
            blocks.insert(
                name.trim().to_string(),
                json!({"encoded":present.len(),"block_codepoints":hi-lo+1}),
            );
        }
    }
    if codes.len() > assigned.len() {
        blocks.insert(
            "No_Block".into(),
            json!({"encoded":codes.len()-assigned.len()}),
        );
    }
    json!({"unicode_version":"17.0.0","encoded":codes.len(),"blocks":blocks})
}
fn document_metadata(font: &Font) -> Value {
    let pairs = font.kerning.as_ref().map(|k| k.pairs.len());
    let classes = font.kerning.as_ref().map(|k| k.groups.len());
    let features: BTreeSet<_> = font
        .features
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|w| w[0] == "feature" && w[1].len() == 4)
        .map(|w| w[1])
        .collect();
    let license = &font.info.license;
    json!({"family":font.style.family,"style":font.style.name,"weight_class":font.style.weight,"width_class":font.style.width,"italic_angle":font.style.italic_angle,"upm":font.upm,"vertical_metrics":{"ascender":font.metrics.ascender,"descender":font.metrics.descender,"cap_height":font.metrics.cap_height,"x_height":font.metrics.x_height,"line_gap":null,"os2":null,"hhea":null},"glyph_count":font.glyphs.len(),"coverage":coverage(font.glyphs.iter().filter_map(|g|g.unicode)),"kerning":{"pairs":pairs,"classes":classes,"method":"document kerning entries and groups; null means unspecified"},"features":features,"license":license,"reserved_font_name":if license.is_empty(){None}else{Some(license.to_lowercase().contains("reserved font name"))},"method":"document metadata; absent source fields are null"})
}
fn binary_metadata(face: &Face<'_>, font: &Font) -> Value {
    let mut m = document_metadata(font);
    let table = |tag: &[u8; 4]| face.raw_face().table(Tag::from_bytes(tag)).unwrap_or(&[]);
    let os2 = table(b"OS/2");
    let hhea = table(b"hhea");
    m["vertical_metrics"] = json!({"ascender":face.ascender(),"descender":face.descender(),"cap_height":face.capital_height(),"x_height":face.x_height(),"line_gap":face.line_gap(),"os2":{"typo_ascender":i16_at(os2,68),"typo_descender":i16_at(os2,70),"typo_line_gap":i16_at(os2,72),"win_ascent":u16_at(os2,74),"win_descent":u16_at(os2,76),"x_height":i16_at(os2,86),"cap_height":i16_at(os2,88)},"hhea":{"ascender":i16_at(hhea,4),"descender":i16_at(hhea,6),"line_gap":i16_at(hhea,8)}});
    let mut codes = BTreeSet::new();
    if let Some(cmap) = face.tables().cmap {
        for sub in cmap.subtables {
            if sub.is_unicode() {
                sub.codepoints(|c| {
                    if sub.glyph_index(c).is_some_and(|id| id.0 != 0) {
                        codes.insert(c);
                    }
                });
            }
        }
    }
    m["coverage"] = coverage(codes);
    let mut features = BTreeSet::new();
    for layout in [face.tables().gsub, face.tables().gpos]
        .into_iter()
        .flatten()
    {
        for feature in layout.features {
            features.insert(feature.tag.to_string());
        }
    }
    m["features"] = json!(features);
    m["kerning"] = kerning_census(face, font);
    m["method"] = json!(
        "binary name, OS/2, hhea, cmap and GSUB/GPOS tables; RFN is license-string detection"
    );
    m
}

fn kerning_census(face: &Face<'_>, font: &Font) -> Value {
    fn census(
        data: &[u8],
        pairs: &mut usize,
        classes: &mut usize,
        class_pairs: &mut usize,
    ) -> Option<()> {
        let format = u16_at(data, 0)?;
        if format == 1 {
            let sets = u16_at(data, 8)?;
            for i in 0..sets {
                let at = u16_at(data, 10 + 2 * usize::from(i))? as usize;
                *pairs += usize::from(u16_at(data, at)?);
            }
        } else if format == 2 {
            let left = u16_at(data, 12)? as usize;
            let right = u16_at(data, 14)? as usize;
            *classes += left + right;
            let flags1 = u16_at(data, 4)?;
            let flags2 = u16_at(data, 6)?;
            let size = 2 * (flags1.count_ones() + flags2.count_ones()) as usize;
            if size > 0 {
                for i in 0..left * right {
                    let record = data.get(16 + i * size..16 + (i + 1) * size)?;
                    if record.iter().any(|v| *v != 0) {
                        *class_pairs += 1;
                    }
                }
            }
        } else {
            return None;
        }
        Some(())
    }
    let legacy = font.kerning.as_ref().map(|k| k.pairs.len()).unwrap_or(0);
    let mut pairs = 0;
    let mut classes = 0;
    let mut class_pairs = 0;
    let mut incomplete = false;
    if let Some(gpos) = face.raw_face().table(Tag::from_bytes(b"GPOS")) {
        let parsed = (|| {
            let start = u16_at(gpos, 8)? as usize;
            let list = gpos.get(start..)?;
            let count = u16_at(list, 0)?;
            for i in 0..count {
                let at = u16_at(list, 2 + 2 * i as usize)? as usize;
                let lookup = list.get(at..)?;
                let kind = u16_at(lookup, 0)?;
                if kind != 2 && kind != 9 {
                    continue;
                }
                for j in 0..u16_at(lookup, 4)? {
                    let at = u16_at(lookup, 6 + 2 * j as usize)? as usize;
                    let mut data = lookup.get(at..)?;
                    let mut pair = kind == 2;
                    if kind == 9 && u16_at(data, 0)? == 1 {
                        pair = u16_at(data, 2)? == 2;
                        let offset = u32::from_be_bytes(data.get(4..8)?.try_into().ok()?) as usize;
                        data = data.get(offset..)?;
                    }
                    if pair && census(data, &mut pairs, &mut classes, &mut class_pairs).is_none() {
                        incomplete = true;
                    }
                }
            }
            Some(())
        })();
        if parsed.is_none() {
            incomplete = true;
        }
    }
    json!({"pairs":if incomplete{None}else{Some(legacy+pairs)},"classes":if incomplete{None}else{Some(classes)},"legacy_pairs":legacy,"gpos_pair_records":pairs,"nonzero_class_cells":class_pairs,"method":"stored legacy kern pairs plus GPOS PairPos format1 records; classes are format2 left+right matrix dimensions including class zero, summed per subtable; extension lookups resolved; contextual activation and class expansion not counted","reason":if incomplete{Some("malformed or unsupported GPOS pair subtable")}else{None}})
}

pub(super) fn font_at(study: &Study) -> Result<Font, String> {
    let path = Path::new(&study.source.path);
    if digest_path(path)? != study.source.sha256 {
        return Err(
            "source changed during analysis; rerun to produce consistent visual sheets".into(),
        );
    }
    if matches!(
        extension(path).as_str(),
        "ttf" | "otf" | "ttc" | "otc" | "woff"
    ) {
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let bytes = crate::sfnt::sfnt_bytes(&bytes)?;
        let mut face = Face::parse(&bytes, study.source.face).map_err(|e| e.to_string())?;
        for (tag, v) in &study.source.axes {
            face.set_variation(
                Tag::from_bytes(tag.as_bytes().try_into().map_err(|_| "invalid axis tag")?),
                *v,
            )
            .ok_or("could not set axis")?;
        }
        crate::sfnt::read_face(&face, "Font")
    } else {
        Font::load(path).map_err(|e| e.to_string())
    }
}
