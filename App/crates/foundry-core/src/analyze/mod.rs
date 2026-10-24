//! Read-only, deterministic collection studies. No font document enters the serialized result.
mod advanced;
mod collection;
mod loader;
mod measure;
mod sheets;
#[cfg(test)]
mod tests;
mod vector;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 2;
// Version 1 is reserved for the other study implementation under development.
pub const ALGORITHM: &str = "outline-analyzer-0.2.0";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub glyphs: Vec<String>,
    pub axes: BTreeMap<String, f32>,
    pub weight_match: Option<f64>,
    pub output: Option<PathBuf>,
    pub format: OutputFormat,
    pub cache_dir: Option<PathBuf>,
    pub sheets: Vec<SheetFormat>,
    pub profile: MeasurementProfile,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeasurementProfile {
    #[default]
    Full,
    Atlas,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Json,
    Csv,
    Md,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SheetFormat {
    Png,
    Pdf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    pub value: Option<f64>,
    pub units: String,
    pub x_height_ratio: Option<f64>,
    pub cap_height_ratio: Option<f64>,
    pub method: String,
    pub glyphs: Vec<String>,
    pub reason: Option<String>,
}
impl Metric {
    fn new(
        value: Option<f64>,
        units: &str,
        method: &str,
        glyphs: &[&str],
        x: f64,
        cap: f64,
    ) -> Self {
        let value = value.filter(|n| n.is_finite());
        Self {
            value,
            units: units.into(),
            x_height_ratio: if units == "font_units" {
                ratio(value, Some(x))
            } else {
                None
            },
            cap_height_ratio: if units == "font_units" {
                ratio(value, Some(cap))
            } else {
                None
            },
            method: method.into(),
            glyphs: glyphs.iter().map(|s| s.to_string()).collect(),
            reason: value
                .is_none()
                .then(|| "missing glyph, empty outline, or degenerate measurement".into()),
        }
    }
}
pub(crate) fn ratio(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    a.zip(b)
        .filter(|(_, b)| b.is_finite() && b.abs() > 1e-12)
        .map(|(a, b)| a / b)
        .filter(|v| v.is_finite())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub path: String,
    pub sha256: String,
    pub face: u32,
    pub instance: String,
    pub axes: BTreeMap<String, f32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    pub class: String,
    pub confidence: f64,
    pub reasons: Vec<String>,
    pub serif_type: Option<String>,
    pub method: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Study {
    pub format: String,
    pub version: u32,
    pub algorithm: String,
    pub source: Source,
    pub metadata: Value,
    pub metrics: BTreeMap<String, Metric>,
    pub glyphs: BTreeMap<String, BTreeMap<String, Metric>>,
    pub classification: Classification,
    pub nearest: Vec<Value>,
    pub outliers: Vec<Value>,
    pub warnings: Vec<String>,
    pub terminals: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub format: String,
    pub version: u32,
    pub algorithm: String,
    pub fonts: Vec<Study>,
    pub collection: Value,
    pub errors: Vec<Value>,
}

pub fn analyze_fonts(paths: &[String], options: &Options) -> Result<Report, String> {
    if !options.sheets.is_empty() && options.output.is_none() {
        return Err("visual sheets require an output directory".into());
    }
    if paths.is_empty() {
        return Err("analyze needs at least one file or folder".into());
    }
    if options
        .weight_match
        .is_some_and(|v| !v.is_finite() || v <= 0.)
    {
        return Err("weight_match must be a positive finite stem/x-height ratio".into());
    }
    if options
        .axes
        .iter()
        .any(|(k, v)| k.len() != 4 || !k.is_ascii() || !v.is_finite())
    {
        return Err(
            "axis tags must contain four ASCII bytes and coordinates must be finite".into(),
        );
    }
    if options.weight_match.is_some() && !options.axes.is_empty() {
        return Err("choose either weight_match or an explicit axis location".into());
    }
    let mut files = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        loader::discover(Path::new(path), &mut files, &mut errors, true);
    }
    files.sort();
    files.dedup();
    for output in [&options.output, &options.cache_dir].into_iter().flatten() {
        loader::check_output(output, &files)?;
    }
    let results: Vec<_> = files
        .par_iter()
        .map(|path| loader::analyze_path(path, options))
        .collect();
    let mut fonts = Vec::new();
    for (path, result) in files.iter().zip(results) {
        match result {
            Ok(mut found) => fonts.append(&mut found),
            Err(error) => errors.push(json!({"path":path,"error":error})),
        }
    }
    fonts.sort_by(|a, b| {
        a.source
            .path
            .cmp(&b.source.path)
            .then(a.source.face.cmp(&b.source.face))
            .then(a.source.instance.cmp(&b.source.instance))
    });
    errors.sort_by_key(Value::to_string);
    let summary = collection::summarize(&mut fonts);
    let report = Report {
        format: "typefoundry.collection-study".into(),
        version: VERSION,
        algorithm: ALGORITHM.into(),
        fonts,
        collection: summary,
        errors,
    };
    if let Some(out) = &options.output {
        write_report(&report, out, options.format)?;
        if !options.sheets.is_empty() {
            for study in &report.fonts {
                let font = loader::font_at(study)?;
                sheets::write(&font, study, out, &options.sheets)?;
            }
        }
    }
    Ok(report)
}

pub fn classify_font(path: &str) -> Result<Value, String> {
    let report = analyze_fonts(&[path.into()], &Options::default())?;
    Ok(
        json!({"fonts":report.fonts.iter().map(|s| json!({"source":s.source,"classification":s.classification})).collect::<Vec<_>>(),"errors":report.errors}),
    )
}
pub fn compare_to_class(
    path: &str,
    class: &str,
    collection: &[String],
    options: &Options,
) -> Result<Value, String> {
    // Reference ranges never include the target merely because it was added for comparison.
    let mut read_options = options.clone();
    read_options.output = None;
    read_options.sheets.clear();
    let reference = analyze_fonts(collection, &read_options)?;
    let targets = analyze_fonts(&[path.into()], &read_options)?;
    collection::compare(&targets, &reference, class)
}
pub fn nearest_fonts(
    path: &str,
    collection: &[String],
    options: &Options,
) -> Result<Value, String> {
    let mut read_options = options.clone();
    read_options.output = None;
    read_options.sheets.clear();
    let reference = analyze_fonts(collection, &read_options)?;
    let target = analyze_fonts(&[path.into()], &read_options)?;
    Ok(
        json!({"fonts":target.fonts.iter().map(|s| json!({"source":s.source,"nearest":collection::nearest(s,&reference.fonts)})).collect::<Vec<_>>(),"errors": ([target.errors,reference.errors].concat())}),
    )
}

pub(crate) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn stable_write(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut f) => f.write_all(data).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::read(path).map_err(|e| e.to_string())? == data {
                Ok(())
            } else {
                Err(format!(
                    "refusing to replace differing output {}; choose a fresh output folder",
                    path.display()
                ))
            }
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}
pub fn render(report: &Report, format: OutputFormat) -> Result<String, String> {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(report).map_err(|e| e.to_string()),
        OutputFormat::Csv => {
            let mut out = String::from(
                "source,face,instance,family,class,metric,value,units,x_height_ratio,cap_height_ratio,method,glyphs,reason\n",
            );
            let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
            for s in &report.fonts {
                for (name, m) in
                    s.metrics
                        .iter()
                        .map(|(k, m)| (k.clone(), m))
                        .chain(s.glyphs.iter().flat_map(|(g, ms)| {
                            ms.iter().map(move |(k, m)| (format!("glyph.{g}.{k}"), m))
                        }))
                {
                    let values = [
                        s.source.path.clone(),
                        s.source.face.to_string(),
                        s.source.instance.clone(),
                        s.metadata["family"].as_str().unwrap_or("").into(),
                        s.classification.class.clone(),
                        name,
                        m.value.map(|v| v.to_string()).unwrap_or_default(),
                        m.units.clone(),
                        m.x_height_ratio.map(|v| v.to_string()).unwrap_or_default(),
                        m.cap_height_ratio
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                        m.method.clone(),
                        m.glyphs.join(" "),
                        m.reason.clone().unwrap_or_default(),
                    ];
                    out.push_str(
                        &values
                            .iter()
                            .map(|s| quote(s))
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                    out.push('\n');
                }
            }
            Ok(out)
        }
        OutputFormat::Md => {
            let escape = |s: &str| s.replace('|', "\\|").replace(['\r', '\n'], " ");
            let mut out = String::from(
                "# TypeFoundry collection study\n\nVector measurements; heuristic classification, not a trained model.\n\n",
            );
            for s in &report.fonts {
                out.push_str(&format!("## {} — {}\n\nClass: **{}** ({:.0}% confidence). {}\n\n| Metric | Value | Method |\n|---|---:|---|\n",escape(s.metadata["family"].as_str().unwrap_or("")),escape(&s.source.instance),s.classification.class,100.*s.classification.confidence,s.classification.reasons.iter().map(|v|escape(v)).collect::<Vec<_>>().join("; ")));
                for (k, m) in &s.metrics {
                    out.push_str(&format!(
                        "| {} | {} | {} |\n",
                        escape(k),
                        m.value.map(|v| format!("{v:.6}")).unwrap_or_else(|| escape(
                            m.reason.as_deref().unwrap_or("unavailable")
                        )),
                        escape(&m.method)
                    ));
                }
            }
            out.push_str("\n## Collection ranges\n\n```json\n");
            out.push_str(
                &serde_json::to_string_pretty(&report.collection).map_err(|e| e.to_string())?,
            );
            out.push_str("\n```\n");
            for error in &report.errors {
                out.push_str(&format!("\n- Error: {}\n", escape(&error.to_string())));
            }
            Ok(out)
        }
    }
}
fn write_report(report: &Report, out: &Path, format: OutputFormat) -> Result<(), String> {
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let json = |v: &Value| serde_json::to_vec_pretty(v).map_err(|e| e.to_string());
    stable_write(
        &out.join("collection.json"),
        &json(
            &json!({"format":report.format,"version":report.version,"algorithm":report.algorithm,"collection":report.collection,"errors":report.errors}),
        )?,
    )?;
    for s in &report.fonts {
        let id = hash(&serde_json::to_vec(&s.source).map_err(|e| e.to_string())?);
        stable_write(&out.join(format!("font-{id}.json")), &json(&json!(s))?)?;
    }
    let ext = match format {
        OutputFormat::Json => "json",
        OutputFormat::Csv => "csv",
        OutputFormat::Md => "md",
    };
    stable_write(
        &out.join(format!("report.{ext}")),
        render(report, format)?.as_bytes(),
    )
}
