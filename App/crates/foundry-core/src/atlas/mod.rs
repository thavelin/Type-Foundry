//! Interpretable font/family design space. Measurements in; no source outlines or learned font data out.
pub(crate) mod embedding;
mod gaps;
#[cfg(test)]
mod tests;
mod viewer;
use crate::analyze::{Source, Study};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "typefoundry.atlas";
pub const VERSION: u32 = 1;
pub const EMBEDDING: &str = "measured-font-space-v2";
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub output: Option<PathBuf>,
    pub min_shared: usize,
    pub neighbors: usize,
    pub gap_count: usize,
    pub labels: Option<PathBuf>,
    pub family_aliases: Option<PathBuf>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            output: None,
            min_shared: 12,
            neighbors: 5,
            gap_count: 24,
            labels: None,
            family_aliases: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dimension {
    pub key: String,
    pub label: String,
    pub group: String,
    pub method: String,
    pub center: f64,
    pub scale: f64,
    pub measured_families: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Face {
    pub id: String,
    pub family_id: String,
    pub family: String,
    #[serde(default)]
    pub revision: String,
    pub style: String,
    pub weight: u16,
    pub width: u16,
    pub italic_angle: f64,
    pub class: String,
    pub confidence: f64,
    pub reasons: Vec<String>,
    pub sources: Vec<Source>,
    pub raw: Vec<Option<f64>>,
    pub vector: Vec<f64>,
    pub mask: Vec<bool>,
    pub coverage: f64,
    pub license: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Family {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    #[serde(default)]
    pub revisions: BTreeMap<String, Vec<String>>,
    pub faces: Vec<String>,
    pub representative: String,
    pub class: String,
    pub confidence: f64,
    pub class_distribution: BTreeMap<String, usize>,
    pub raw: Vec<Option<f64>>,
    pub vector: Vec<f64>,
    pub mask: Vec<bool>,
    pub projection: [f64; 2],
    pub ranges: Vec<Value>,
    pub neighbors: Vec<Value>,
    pub cluster: Option<usize>,
    pub label_source: String,
    #[serde(default)]
    pub grouping_notes: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Atlas {
    pub format: String,
    pub version: u32,
    pub embedding: String,
    #[serde(default)]
    pub space_id: String,
    pub dimensions: Vec<Dimension>,
    pub faces: Vec<Face>,
    pub families: Vec<Family>,
    pub clusters: Vec<Value>,
    pub gaps: Vec<Value>,
    pub principles: Vec<Value>,
    pub projection: Value,
    pub inventory: Value,
    pub warnings: Vec<String>,
    pub source_errors: Vec<Value>,
}
fn family_revision(name: &str) -> (&str, Option<&str>) {
    if let Some((base, suffix)) = name.rsplit_once(' ') {
        let version = suffix
            .strip_prefix('v')
            .or_else(|| suffix.strip_prefix('V'));
        if version.is_some_and(|v| {
            !v.is_empty()
                && v.starts_with(|c: char| c.is_ascii_digit())
                && v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        }) {
            return (base, Some(suffix));
        }
    }
    (name, None)
}
fn revision(s: &Study, named: Option<&str>) -> String {
    if let Some(v) = named {
        return v.into();
    }
    for part in s.source.path.split(['/', '\\']).rev() {
        let lower = part.to_ascii_lowercase();
        if lower
            .strip_prefix("round")
            .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()))
        {
            return part.into();
        }
        for token in part.split(['-', '_']) {
            if token
                .strip_prefix('r')
                .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()))
            {
                return token
                    .strip_suffix(".ttf")
                    .or_else(|| token.strip_suffix(".otf"))
                    .or_else(|| token.strip_suffix(".json"))
                    .unwrap_or(token)
                    .into();
            }
        }
    }
    s.metadata["font_version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or("unspecified")
        .into()
}
fn revision_order(v: &str) -> Vec<u32> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect()
}
fn hash(s: &[u8]) -> String {
    format!("{:x}", Sha256::digest(s))
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn normalized(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn source_id(s: &Source) -> String {
    hash(
        serde_json::to_string(&json!({"sha256":s.sha256,"face":s.face,"axes":s.axes}))
            .unwrap_or_default()
            .as_bytes(),
    )
}
fn walk(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_file() {
        if path.extension().is_some_and(|s| s == "json") {
            files.push(path.into());
        }
        return Ok(());
    }
    for child in fs::read_dir(path).map_err(|e| e.to_string())? {
        let child = child.map_err(|e| e.to_string())?;
        if !matches!(
            child.file_name().to_str(),
            Some("bin" | "cache" | ".git" | "target" | "__pycache__" | "do-not-index")
        ) {
            walk(&child.path(), files)?;
        }
    }
    Ok(())
}
type StudyInputs = (Vec<Study>, Vec<Value>, Vec<Value>);
fn read_studies(paths: &[String]) -> Result<StudyInputs, String> {
    let mut files = Vec::new();
    for p in paths {
        walk(Path::new(p), &mut files)?;
    }
    files.sort();
    files.dedup();
    let mut studies = Vec::new();
    let mut inventories = Vec::new();
    let mut errors = Vec::new();
    for p in files {
        let v: Value = match fs::read(&p)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
        {
            Ok(v) => v,
            Err(e) => {
                errors.push(json!({"path":p,"error":e}));
                continue;
            }
        };
        let format = v.get("format").and_then(Value::as_str);
        if format == Some("typefoundry.source-inventory") {
            inventories.push(v);
            continue;
        }
        if format == Some("typefoundry.study") {
            match serde_json::from_value::<Study>(v) {
                Ok(s) => studies.push(s),
                Err(e) => errors.push(json!({"path":p,"error":e.to_string()})),
            }
        } else if format == Some("typefoundry.collection-study") {
            if let Some(fonts) = v["fonts"].as_array() {
                for font in fonts {
                    match serde_json::from_value::<Study>(font.clone()) {
                        Ok(s) => studies.push(s),
                        Err(e) => errors.push(json!({"path":p,"error":e.to_string()})),
                    }
                }
            }
            if let Some(es) = v["errors"].as_array() {
                errors.extend(es.clone());
            }
        }
    }
    Ok((studies, inventories, errors))
}
pub fn build(paths: &[String], options: &Options) -> Result<Atlas, String> {
    if paths.is_empty() {
        return Err("atlas needs measurement files or worker stores".into());
    }
    if !(4..=embedding::SPECS.len()).contains(&options.min_shared)
        || options.neighbors == 0
        || options.neighbors > 50
        || options.gap_count > 200
    {
        return Err(
            "min_shared must be 4..dimension-count, neighbors 1..50 and gap_count <=200".into(),
        );
    }
    let (studies, inventories, mut errors) = read_studies(paths)?;
    let mut merged: BTreeMap<String, Study> = BTreeMap::new();
    let mut aliases: BTreeMap<String, Vec<Source>> = BTreeMap::new();
    for s in studies {
        if s.format != "typefoundry.study"
            || s.version != 2
            || s.algorithm != crate::analyze::ALGORITHM
        {
            errors.push(json!({"source":s.source,"error":"unsupported study schema/algorithm; remeasure before mixing spaces"}));
            continue;
        }
        let id = source_id(&s.source);
        aliases
            .entry(id.clone())
            .or_default()
            .push(s.source.clone());
        match merged.entry(id) {
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(s);
            }
            std::collections::btree_map::Entry::Occupied(mut e) => {
                let measured = |s: &Study| s.metrics.values().filter(|m| m.value.is_some()).count();
                if measured(&s) > measured(e.get()) {
                    e.insert(s);
                }
            }
        }
    }
    for inventory in &inventories {
        if let Some(sources) = inventory["sources"].as_array() {
            for row in sources {
                if row["status"] != "measured" {
                    errors.push(
                        json!({"path":row["path"],"status":row["status"],"error":row["error"]}),
                    );
                    continue;
                }
                let digest = text(row, "sha256");
                for (id, study) in &merged {
                    if study.source.sha256 == digest {
                        let mut source = study.source.clone();
                        source.path = text(row, "path").into();
                        aliases.entry(id.clone()).or_default().push(source);
                    }
                }
            }
        }
    }
    if merged.is_empty() {
        return Err("no compatible studies found; scan fonts first".into());
    }
    let family_aliases: Value = match &options.family_aliases {
        Some(p) => serde_json::from_slice(&fs::read(p).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
        None => json!({}),
    };
    let mut family_names: BTreeMap<String, String> = BTreeMap::new();
    let mut grouping_notes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut faces = Vec::new();
    for (id, s) in merged {
        let family = text(&s.metadata, "family").trim();
        let family = if family.is_empty() {
            "Unnamed family"
        } else {
            family
        };
        let (base_family, named_revision) = family_revision(family);
        let revision = revision(&s, named_revision);
        let alias = family_aliases
            .get(family)
            .or_else(|| family_aliases.get(base_family));
        let canonical = if let Some(alias) = alias {
            let target = alias["family"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or("family aliases require family and reason")?;
            let reason = alias["reason"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or("family aliases require evidence in reason")?;
            grouping_notes
                .entry(normalized(target))
                .or_default()
                .push(format!("{family}: {reason}"));
            target
        } else {
            base_family
        };
        if named_revision.is_some() {
            grouping_notes
                .entry(normalized(canonical))
                .or_default()
                .push(format!(
                    "{family}: explicit version suffix retained in revision {revision}"
                ));
        }
        let family_id = hash(normalized(canonical).as_bytes());
        family_names
            .entry(family_id.clone())
            .or_insert(canonical.into());
        let mut sources = aliases.remove(&id).unwrap_or_default();
        sources.sort_by_key(|s| s.path.clone());
        sources.dedup_by(|a, b| a.path == b.path);
        let raw = embedding::extract(&s);
        let coverage = raw.iter().filter(|v| v.is_some()).count() as f64 / raw.len() as f64;
        faces.push(Face {
            id,
            family_id,
            family: family.into(),
            revision,
            style: text(&s.metadata, "style").into(),
            weight: s.metadata["weight_class"].as_u64().unwrap_or(400) as u16,
            width: s.metadata["width_class"].as_u64().unwrap_or(5) as u16,
            italic_angle: s.metadata["italic_angle"].as_f64().unwrap_or(0.),
            class: if s.classification.confidence < 0.4
                || coverage * (embedding::SPECS.len() as f64) < 12.
            {
                "unclassified".into()
            } else {
                s.classification.class
            },
            confidence: s.classification.confidence,
            reasons: s.classification.reasons,
            sources,
            raw,
            vector: vec![],
            mask: vec![],
            coverage,
            license: s.metadata["license"].clone(),
        });
    }
    faces.sort_by(|a, b| a.id.cmp(&b.id));
    let mut grouped: BTreeMap<String, Vec<&Face>> = BTreeMap::new();
    for face in &faces {
        grouped
            .entry(face.family_id.clone())
            .or_default()
            .push(face);
    }
    let labels: Value = match &options.labels {
        Some(p) => serde_json::from_slice(&fs::read(p).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
        None => json!({}),
    };
    let mut families = Vec::new();
    for (id, mut members) in grouped {
        // One regular upright representative per family fits the space. More weights do not buy more influence.
        members.sort_by(|a, b| {
            let score = |f: &Face| {
                ((f.weight as f64 - 400.).abs() / 500.)
                    + (f.width as f64 - 5.).abs() * 0.5
                    + if f.italic_angle.abs() > 1.
                        || f.style.to_lowercase().contains("italic")
                        || f.style.to_lowercase().contains("oblique")
                    {
                        1.
                    } else {
                        0.
                    }
                    + (1. - f.coverage) * 2.
            };
            score(a)
                .total_cmp(&score(b))
                .then_with(|| revision_order(&b.revision).cmp(&revision_order(&a.revision)))
                .then(a.id.cmp(&b.id))
        });
        let rep = members[0];
        let mut distribution = BTreeMap::new();
        for m in &members {
            *distribution.entry(m.class.clone()).or_insert(0) += 1;
        }
        let mut names: BTreeSet<_> = members.iter().map(|m| m.family.clone()).collect();
        let name = family_names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| names.iter().next().cloned().unwrap_or_default());
        names.remove(&name);
        let mut notes = grouping_notes
            .remove(&normalized(&name))
            .unwrap_or_default();
        notes.sort();
        notes.dedup();
        let mut family = Family {
            id: id.clone(),
            name,
            aliases: names.into_iter().collect(),
            revisions: {
                let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
                for f in &members {
                    groups
                        .entry(f.revision.clone())
                        .or_default()
                        .push(f.id.clone());
                }
                groups
            },
            faces: members.iter().map(|m| m.id.clone()).collect(),
            representative: rep.id.clone(),
            class: rep.class.clone(),
            confidence: rep.confidence,
            class_distribution: distribution,
            raw: rep.raw.clone(),
            vector: vec![],
            mask: vec![],
            projection: [0., 0.],
            ranges: vec![],
            neighbors: vec![],
            cluster: None,
            label_source: "representative outline rules".into(),
            grouping_notes: notes,
        };
        if let Some(label) = labels.get(&id).or_else(|| labels.get(&family.name)) {
            let class = label["class"]
                .as_str()
                .ok_or("manual labels require class and reason")?;
            let reason = label["reason"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or("manual labels require a nonempty reason")?;
            if ![
                "geometric sans",
                "grotesque",
                "humanist sans",
                "serif text",
                "slab",
                "mono",
                "display",
                "script",
                "unclassified",
            ]
            .contains(&class)
            {
                return Err(format!("unknown manual class {class}"));
            }
            family.class = class.into();
            family.label_source = format!("manual: {reason}");
            family.confidence = 1.;
        }
        for i in 0..embedding::SPECS.len() {
            let mut values: Vec<_> = members.iter().filter_map(|m| m.raw[i]).collect();
            values.sort_by(f64::total_cmp);
            family.ranges.push(if values.is_empty(){Value::Null}else{json!({"min":values[0],"median":embedding::quantile(&values,0.5),"max":values[values.len()-1],"locations":values.len()})});
        }
        families.push(family);
    }
    families.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    let dimensions = embedding::fit(&families);
    for f in &mut families {
        (f.vector, f.mask) = embedding::encode(&f.raw, &dimensions);
    }
    for f in &mut faces {
        (f.vector, f.mask) = embedding::encode(&f.raw, &dimensions);
    }
    let projection = embedding::project(&mut families, &dimensions);
    let neighbors: Vec<_> = families
        .iter()
        .map(|f| {
            embedding::nearest(
                f,
                &families,
                &dimensions,
                options.min_shared,
                options.neighbors,
            )
        })
        .collect();
    for (f, n) in families.iter_mut().zip(neighbors) {
        f.neighbors = n;
    }
    let clusters = embedding::cluster(&mut families, &dimensions, options.min_shared);
    let mut gaps = gaps::find(&families, &dimensions, options, &faces);
    for gap in &mut gaps {
        let v: Vec<f64> =
            serde_json::from_value(gap["vector"].clone()).map_err(|e| e.to_string())?;
        let center: Vec<f64> =
            serde_json::from_value(projection["center"].clone()).map_err(|e| e.to_string())?;
        let axes: Vec<Vec<f64>> =
            serde_json::from_value(projection["components"].clone()).map_err(|e| e.to_string())?;
        gap["projection"] = json!(
            axes.iter()
                .map(|a| a
                    .iter()
                    .enumerate()
                    .map(|(i, w)| w * (v[i] - center[i]))
                    .sum::<f64>())
                .collect::<Vec<_>>()
        );
    }

    let principles = gaps::principles(&families, &dimensions);
    errors.sort_by_key(Value::to_string);
    errors.dedup();
    let counts:Vec<_>=inventories.iter().map(|v|json!({"profile":v["profile"],"sources":v["sources"].as_array().map(Vec::len),"inventory_errors":v["inventory_errors"]})).collect();
    let space_id = hash(&serde_json::to_vec(&dimensions).map_err(|e| e.to_string())?);
    let atlas=Atlas{format:FORMAT.into(),version:VERSION,embedding:EMBEDDING.into(),space_id,dimensions,faces,families,clusters,gaps,principles,projection,inventory:json!({"workers":counts,"source_paths":inventories.iter().filter_map(|v|v["sources"].as_array()).map(Vec::len).sum::<usize>(),"deduplication":"byte hash + collection face + axis coordinates; source aliases retained; family space fits one regular upright representative per normalized family; explicit v-number suffixes grouped as revisions, source round markers and font versions retained","coverage_scope":"configured installed/local libraries and accessible NAS indexed sources; exclusions/failures retained in source_errors"}),warnings:vec!["Distances describe selected outline measurements, not visual identity or typeface originality. Missing features have masks; they are not measured zero.".into(),"PCA is a lossy browsing projection. Gap ranking uses shared measured dimensions in the full standardized space, not empty pixels in the plot.".into(),"Classification is heuristic unless manually reviewed. Family naming collisions and source-license restrictions require review before authoring from references.".into(),"A sparse region is a hypothesis within the scanned collection; feasibility and usefulness need original drawing and testing. No outlines are reconstructed or exported.".into()],source_errors:errors};
    if let Some(out) = &options.output {
        write(&atlas, out)?;
    }
    Ok(atlas)
}
struct BuildLock {
    path: PathBuf,
    file: Option<fs::File>,
}
impl Drop for BuildLock {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}
fn write(atlas: &Atlas, out: &Path) -> Result<(), String> {
    use std::io::Write;
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let lock_path = out.join(".atlas-build.lock");
    let file=fs::OpenOptions::new().write(true).create_new(true).open(&lock_path).map_err(|_|"another atlas build owns this output directory; inspect .atlas-build.lock before retrying")?;
    let mut lock = BuildLock {
        path: lock_path,
        file: Some(file),
    };
    if let Some(file) = &mut lock.file {
        writeln!(file, "{}", std::process::id()).map_err(|e| e.to_string())?;
    }

    let owned = out.join("latest.json");
    if owned.exists() {
        let v: Value = serde_json::from_slice(&fs::read(&owned).map_err(|e| e.to_string())?)
            .map_err(|_| "existing latest.json is not an atlas manifest")?;
        if v["format"] != FORMAT || v["version"] != VERSION {
            return Err("refusing to replace another document's output directory".into());
        }
    } else if out.join("explore.html").exists() {
        return Err("refusing to replace an unrelated explore.html".into());
    }
    let data = serde_json::to_vec(atlas).map_err(|e| e.to_string())?;
    let id = hash(&data);
    let name = format!("atlas-{}.json", &id[..16]);
    let p = out.join(&name);
    if p.exists() && fs::read(&p).map_err(|e| e.to_string())? != data {
        return Err("immutable atlas snapshot was changed; choose a fresh output folder".into());
    }
    if !p.exists() {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
            .map_err(|e| e.to_string())?;
        file.write_all(&data).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    let html = viewer::render(atlas)?;
    atomic_replace(&out.join("explore.html"), html.as_bytes())?;
    atomic_replace(&out.join("latest.json"),&serde_json::to_vec_pretty(&json!({"format":FORMAT,"version":VERSION,"snapshot":name,"sha256":id,"space_id":atlas.space_id,"families":atlas.families.len(),"faces":atlas.faces.len(),"gaps":atlas.gaps.len()})).map_err(|e|e.to_string())?)?;
    Ok(())
}
fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
    fs::rename(&temporary, path).map_err(|e| e.to_string())
}
fn load_atlas(path: &Path) -> Result<Atlas, String> {
    let mut bytes = fs::read(path).map_err(|e| e.to_string())?;
    let pointer: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if let Some(snapshot) = pointer.get("snapshot").and_then(Value::as_str) {
        let relative = Path::new(snapshot);
        if relative.components().count() != 1
            || relative.file_name().and_then(|s| s.to_str()) != Some(snapshot)
        {
            return Err("atlas snapshot pointer must be one local filename".into());
        }
        bytes = fs::read(path.parent().unwrap_or(Path::new(".")).join(relative))
            .map_err(|e| e.to_string())?;
        if pointer["sha256"].as_str() != Some(hash(&bytes).as_str()) {
            return Err("atlas snapshot checksum mismatch".into());
        }
    }
    let mut atlas: Atlas = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if atlas.format != FORMAT
        || atlas.version != VERSION
        || !matches!(
            atlas.embedding.as_str(),
            EMBEDDING | "measured-font-space-v1"
        )
    {
        return Err("unsupported atlas version".into());
    }
    let n = atlas.dimensions.len();
    if n != embedding::SPECS.len()
        || atlas
            .dimensions
            .iter()
            .any(|d| !d.scale.is_finite() || d.scale <= 0. || !d.center.is_finite())
        || atlas
            .families
            .iter()
            .any(|f| f.raw.len() != n || f.vector.len() != n || f.mask.len() != n)
    {
        return Err("atlas dimensions/masks are inconsistent".into());
    }
    let calibration = hash(&serde_json::to_vec(&atlas.dimensions).map_err(|e| e.to_string())?);
    if atlas.space_id.is_empty() {
        atlas.space_id = calibration;
    } else if atlas.space_id != calibration {
        return Err("atlas calibration identity mismatch".into());
    }
    if atlas
        .faces
        .iter()
        .any(|f| f.raw.len() != n || f.vector.len() != n || f.mask.len() != n)
        || atlas
            .gaps
            .iter()
            .any(|g| g["raw"].as_array().is_none_or(|a| a.len() != n))
    {
        return Err("atlas face/brief dimensions are inconsistent".into());
    }
    Ok(atlas)
}

pub fn query(path: &Path, family: &str, limit: usize, min_shared: usize) -> Result<Value, String> {
    let atlas = load_atlas(path)?;
    let matches: Vec<_> = atlas
        .families
        .iter()
        .filter(|f| f.id == family || normalized(&f.name) == normalized(family))
        .collect();
    if matches.len() != 1 {
        return Err("family must match one exact name or ID".into());
    }
    let f = matches[0];
    Ok(
        json!({"family":f,"nearest":embedding::nearest(f,&atlas.families,&atlas.dimensions,min_shared,limit.min(50)),"principles":atlas.principles.iter().filter(|p|p["class"]==f.class).collect::<Vec<_>>(),"embedding":atlas.embedding,"space_id":atlas.space_id}),
    )
}

fn class_briefs(atlas: &Atlas, class: &str, limit: usize) -> Vec<Value> {
    let stored: Vec<_> = atlas
        .gaps
        .iter()
        .filter(|g| g["class"] == class)
        .cloned()
        .collect();
    if !stored.is_empty() {
        return stored;
    }
    // The atlas's global top-N can exclude a populated class entirely. Recompute
    // within that class, keeping full-space distance checks against every face.
    let families: Vec<_> = atlas
        .families
        .iter()
        .filter(|f| f.class == class)
        .cloned()
        .collect();
    gaps::find(
        &families,
        &atlas.dimensions,
        &Options {
            gap_count: limit.max(24),
            ..Options::default()
        },
        &atlas.faces,
    )
}
pub fn briefs(path: &Path, class: Option<&str>, limit: usize) -> Result<Value, String> {
    let atlas = load_atlas(path)?;
    let candidates = if let Some(class) = class {
        class_briefs(&atlas, class, limit)
    } else {
        atlas.gaps.clone()
    };
    Ok(
        json!({"format":"typefoundry.design-briefs","version":1,"embedding":atlas.embedding,"space_id":atlas.space_id,
        "briefs":candidates.into_iter().take(limit.min(200)).collect::<Vec<_>>(),
        "method":"stored ranked gaps, or class-specific gap search when the global shortlist omitted the requested class; all distance/support constraints retained"}),
    )
}
pub fn evaluate_font(
    font: &str,
    path: &Path,
    brief: &str,
    options: &crate::analyze::Options,
) -> Result<Value, String> {
    let mut atlas = load_atlas(path)?;
    if atlas.embedding != EMBEDDING {
        return Err("remeasure and rebuild this atlas before evaluating fonts with the new analyzer; v1 snapshots remain browsable".into());
    }
    if !atlas.gaps.iter().any(|g| {
        g["id"]
            .as_str()
            .is_some_and(|id| id == brief || (brief.len() >= 8 && id.starts_with(brief)))
    }) {
        let classes: BTreeSet<_> = atlas.families.iter().map(|f| f.class.clone()).collect();
        let extra: Vec<_> = classes
            .iter()
            .flat_map(|c| class_briefs(&atlas, c, 200))
            .collect();
        atlas.gaps.extend(extra);
    }
    let candidates: Vec<_> = atlas
        .gaps
        .iter()
        .filter(|g| {
            g["id"]
                .as_str()
                .is_some_and(|id| id == brief || (brief.len() >= 8 && id.starts_with(brief)))
        })
        .collect();
    if candidates.len() != 1 {
        return Err(
            "brief must match one gap ID or an unambiguous prefix of at least 8 characters".into(),
        );
    }
    let target = candidates[0];
    let raw: Vec<Option<f64>> =
        serde_json::from_value(target["raw"].clone()).map_err(|e| e.to_string())?;
    let (vector, mask) = embedding::encode(&raw, &atlas.dimensions);
    let mut read = options.clone();
    read.output = None;
    read.sheets.clear();
    if read.glyphs.is_empty() {
        read.glyphs = "n,o,H,O,x,a,e,s,c,r,C,G,S,k,4,6,9,8,I,h,u,0,space"
            .split(',')
            .map(str::to_string)
            .collect();
    }
    let report = crate::analyze::analyze_fonts(&[font.into()], &read)?;
    let mut results = Vec::new();
    for study in report.fonts {
        let actual = embedding::extract(&study);
        let (av, am) = embedding::encode(&actual, &atlas.dimensions);
        let distance = embedding::distance(&av, &am, &vector, &mask, &atlas.dimensions, 12);
        let mut metrics:Vec<_>=atlas.dimensions.iter().enumerate().filter_map(|(i,d)|actual[i].zip(raw[i]).map(|(a,t)|json!({"metric":d.key,"label":d.label,"actual":a,"target":t,"difference":a-t,"normalized_difference":av[i]-vector[i],"direction":if (av[i]-vector[i]).abs()<0.1{"near target"}else if a<t{"increase measured feature"}else{"decrease measured feature"},"method":d.method}))).collect();
        metrics.sort_by(|a, b| {
            b["normalized_difference"]
                .as_f64()
                .unwrap_or(0.)
                .abs()
                .total_cmp(&a["normalized_difference"].as_f64().unwrap_or(0.).abs())
        });
        results.push(json!({"source":study.source,"family":study.metadata["family"],"style":study.metadata["style"],"distance":distance.map(|d|d.0),"shared_dimensions":distance.map(|d|d.1),"metrics":metrics,"reason":if distance.is_none(){Some("at least 12 shared dimensions across 4 domains required")}else{None}}));
    }
    Ok(
        json!({"format":"typefoundry.brief-evaluation","version":1,"embedding":atlas.embedding,"space_id":atlas.space_id,"brief":target["id"],"fonts":results,"errors":report.errors,"interpretation":"numeric guidance for iterating original outlines; a low score measures selected features, not visual quality or originality"}),
    )
}
