use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum Cli {
    Help,
    AtlasEvaluate {
        font: String,
        path: String,
        brief: String,
    },
    Atlas {
        paths: Vec<String>,
        options: foundry_core::atlas::Options,
    },
    AtlasQuery {
        path: String,
        family: String,
        limit: usize,
        min_shared: usize,
    },
    AtlasBriefs {
        path: String,
        class: Option<String>,
        limit: usize,
    },
    Analyze {
        paths: Vec<String>,
        options: foundry_core::analyze::Options,
    },
    New {
        name: String,
        upm: u16,
        out: PathBuf,
        force: bool,
    },
    Info {
        path: PathBuf,
    },
    Check {
        a: PathBuf,
        b: PathBuf,
    },
    Blend {
        a: PathBuf,
        b: PathBuf,
        t: f64,
        out: PathBuf,
        force: bool,
    },
    Proof {
        font: PathBuf,
        out: PathBuf,
        text: Option<String>,
        pixel_size: f64,
        guides: bool,
        boxes: bool,
        compare: Option<PathBuf>,
        force: bool,
    },
    Diff {
        a: PathBuf,
        b: PathBuf,
    },
    Run {
        file: Option<PathBuf>,
    },
    Study {
        path: PathBuf,
        out: PathBuf,
        force: bool,
    },
    Studies,
    Fit {
        font: PathBuf,
        study: PathBuf,
        family: String,
        style: Option<String>,
        weight: Option<u16>,
        out: PathBuf,
        force: bool,
    },
    Mcp,
    Version,
}

pub fn parse(args: &[String]) -> Result<Cli, String> {
    if args.is_empty() || args[0] == "help" || args[0] == "--help" || args[0] == "-h" {
        return Ok(Cli::Help);
    }
    if args[0] == "version" || args[0] == "--version" || args[0] == "-V" {
        return Ok(Cli::Version);
    }
    let (command, rest) = args.split_first().expect("args is not empty");
    let parsed = parse_flags(rest)?;
    match command.as_str() {
        "new" => parse_new(parsed),
        "analyze" => parse_analyze(parsed),
        "atlas" => parse_atlas(parsed),
        "atlas-query" => parse_atlas_query(parsed),
        "atlas-briefs" => parse_atlas_briefs(parsed),
        "atlas-evaluate" => parse_atlas_evaluate(parsed),
        "info" => parse_info(parsed),
        "check" => parse_check(parsed),
        "blend" => parse_blend(parsed),
        "proof" => parse_proof(parsed),
        "diff" => parse_diff(parsed),
        "run" => parse_run(parsed),
        "study" => parse_study(parsed),
        "studies" => parse_studies(parsed),
        "fit" => parse_fit(parsed),
        "mcp" => parse_mcp(parsed),
        other => Err(format!("unknown command {other}")),
    }
}

struct Parsed {
    positionals: Vec<String>,
    flags: BTreeMap<String, String>,
}

const SWITCHES: &[&str] = &["force", "guides", "boxes"];

fn parse_flags(args: &[String]) -> Result<Parsed, String> {
    let mut positionals = Vec::new();
    let mut flags = BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if let Some(name) = arg.strip_prefix("--") {
            if name.is_empty() {
                return Err("expected a flag name".to_string());
            }
            let (value, consumed) = if SWITCHES.contains(&name) {
                match args.get(index + 1).map(String::as_str) {
                    Some("true" | "false") => (args[index + 1].clone(), 2),
                    _ => ("true".to_string(), 1),
                }
            } else {
                let Some(value) = args.get(index + 1) else {
                    return Err(format!("--{name} needs a value"));
                };
                if value.starts_with("--") {
                    return Err(format!("--{name} needs a value"));
                }
                (value.clone(), 2)
            };
            if flags.insert(name.to_string(), value).is_some() {
                return Err(format!("--{name} was given twice"));
            }
            index += consumed;
        } else {
            positionals.push(arg.clone());
            index += 1;
        }
    }
    Ok(Parsed { positionals, flags })
}

fn parse_new(mut parsed: Parsed) -> Result<Cli, String> {
    require_positionals(&parsed, 0, "new")?;
    let name = take_required(&mut parsed, "name")?;
    let out = PathBuf::from(take_required(&mut parsed, "out")?);
    let upm = match parsed.flags.remove("upm") {
        Some(value) => parse_upm(&value)?,
        None => 1000,
    };
    let force = take_bool(&mut parsed, "force", false)?;
    reject_unknown(parsed)?;
    Ok(Cli::New {
        name,
        upm,
        out,
        force,
    })
}

fn parse_atlas_evaluate(mut parsed: Parsed) -> Result<Cli, String> {
    require_positionals(&parsed, 1, "atlas-evaluate")?;
    let font = parsed.positionals.remove(0);
    let path = take_required(&mut parsed, "atlas")?;
    let brief = take_required(&mut parsed, "brief")?;
    reject_unknown(parsed)?;
    Ok(Cli::AtlasEvaluate { font, path, brief })
}
fn parse_atlas(mut parsed: Parsed) -> Result<Cli, String> {
    if parsed.positionals.is_empty() {
        return Err("atlas needs measurement files or worker-store paths".into());
    }
    let options = foundry_core::atlas::Options {
        output: parsed.flags.remove("out").map(PathBuf::from),
        labels: parsed.flags.remove("labels").map(PathBuf::from),
        family_aliases: parsed.flags.remove("family-aliases").map(PathBuf::from),
        min_shared: atlas_number(&mut parsed, "min-shared", 12)?,
        neighbors: atlas_number(&mut parsed, "neighbors", 5)?,
        gap_count: atlas_number(&mut parsed, "gaps", 24)?,
    };
    let paths = std::mem::take(&mut parsed.positionals);
    reject_unknown(parsed)?;
    Ok(Cli::Atlas { paths, options })
}
fn atlas_number(parsed: &mut Parsed, flag: &str, default: usize) -> Result<usize, String> {
    parsed
        .flags
        .remove(flag)
        .map(|v| {
            v.parse::<usize>()
                .map_err(|_| format!("--{flag} requires a nonnegative integer"))
        })
        .unwrap_or(Ok(default))
}
fn parse_atlas_query(mut parsed: Parsed) -> Result<Cli, String> {
    require_positionals(&parsed, 1, "atlas-query")?;
    let path = parsed.positionals.remove(0);
    let family = take_required(&mut parsed, "family")?;
    let limit = atlas_number(&mut parsed, "limit", 5)?;
    let min_shared = atlas_number(&mut parsed, "min-shared", 12)?;
    reject_unknown(parsed)?;
    Ok(Cli::AtlasQuery {
        path,
        family,
        limit,
        min_shared,
    })
}
fn parse_atlas_briefs(mut parsed: Parsed) -> Result<Cli, String> {
    require_positionals(&parsed, 1, "atlas-briefs")?;
    let path = parsed.positionals.remove(0);
    let class = parsed.flags.remove("class");
    let limit = atlas_number(&mut parsed, "limit", 5)?;
    reject_unknown(parsed)?;
    Ok(Cli::AtlasBriefs { path, class, limit })
}
fn parse_analyze(mut parsed: Parsed) -> Result<Cli, String> {
    use foundry_core::analyze::{Options, OutputFormat};
    if parsed.positionals.is_empty() {
        return Err("analyze needs one or more paths".into());
    }
    let mut options = Options {
        output: parsed.flags.remove("out").map(PathBuf::from),
        cache_dir: parsed.flags.remove("cache-dir").map(PathBuf::from),
        ..Options::default()
    };
    options.format = match parsed.flags.remove("format").as_deref().unwrap_or("json") {
        "json" => OutputFormat::Json,
        "csv" => OutputFormat::Csv,
        "md" => OutputFormat::Md,
        _ => return Err("--format must be json, csv, or md".into()),
    };
    if let Some(v) = parsed.flags.remove("profile") {
        options.profile = match v.as_str() {
            "full" => foundry_core::analyze::MeasurementProfile::Full,
            "atlas" => foundry_core::analyze::MeasurementProfile::Atlas,
            _ => return Err("--profile must be full or atlas".into()),
        };
    }
    if let Some(v) = parsed.flags.remove("glyphs") {
        options.glyphs = v.split(',').map(str::to_string).collect();
    }
    if let Some(v) = parsed.flags.remove("weight-match") {
        options.weight_match = Some(v.parse().map_err(|_| "--weight-match requires a number")?);
    }
    if let Some(v) = parsed.flags.remove("axes") {
        for axis in v.split(',') {
            let (tag, coordinate) = axis
                .split_once('=')
                .ok_or("--axes requires tag=value,tag=value")?;
            if options
                .axes
                .insert(
                    tag.into(),
                    coordinate.parse().map_err(|_| "invalid axis coordinate")?,
                )
                .is_some()
            {
                return Err("duplicate axis tag".into());
            }
        }
    }
    if let Some(v) = parsed.flags.remove("sheets") {
        for format in v.split(',') {
            options.sheets.push(match format {
                "png" => foundry_core::analyze::SheetFormat::Png,
                "pdf" => foundry_core::analyze::SheetFormat::Pdf,
                _ => return Err("--sheets must be png,pdf".into()),
            });
        }
    }
    let paths = std::mem::take(&mut parsed.positionals);
    reject_unknown(parsed)?;
    Ok(Cli::Analyze { paths, options })
}
fn parse_info(parsed: Parsed) -> Result<Cli, String> {
    let path = one_path(parsed, "info")?;
    Ok(Cli::Info { path })
}

fn parse_check(parsed: Parsed) -> Result<Cli, String> {
    let (a, b) = two_paths(parsed, "check")?;
    Ok(Cli::Check { a, b })
}

fn parse_blend(mut parsed: Parsed) -> Result<Cli, String> {
    require_positionals(&parsed, 2, "blend")?;
    let a = PathBuf::from(parsed.positionals[0].clone());
    let b = PathBuf::from(parsed.positionals[1].clone());
    let out = PathBuf::from(take_required(&mut parsed, "out")?);
    let t = match parsed.flags.remove("t") {
        Some(value) => value
            .parse::<f64>()
            .map_err(|_| format!("--t must be a number, got {value}"))?,
        None => 0.5,
    };
    let force = take_bool(&mut parsed, "force", false)?;
    reject_unknown(parsed)?;
    Ok(Cli::Blend {
        a,
        b,
        t,
        out,
        force,
    })
}

fn parse_proof(mut parsed: Parsed) -> Result<Cli, String> {
    require_positionals(&parsed, 1, "proof")?;
    let font = PathBuf::from(parsed.positionals[0].clone());
    let out = PathBuf::from(take_required(&mut parsed, "out")?);
    let text = parsed.flags.remove("text");
    let pixel_size = match parsed.flags.remove("pixel-size") {
        Some(value) => value
            .parse::<f64>()
            .map_err(|_| format!("--pixel-size must be a number, got {value}"))?,
        None => 72.0,
    };
    let guides = take_bool(&mut parsed, "guides", true)?;
    let boxes = take_bool(&mut parsed, "boxes", true)?;
    let compare = parsed.flags.remove("compare").map(PathBuf::from);
    let force = take_bool(&mut parsed, "force", false)?;
    reject_unknown(parsed)?;
    Ok(Cli::Proof {
        font,
        out,
        text,
        pixel_size,
        guides,
        boxes,
        compare,
        force,
    })
}

fn parse_diff(parsed: Parsed) -> Result<Cli, String> {
    let (a, b) = two_paths(parsed, "diff")?;
    Ok(Cli::Diff { a, b })
}

fn parse_study(mut parsed: Parsed) -> Result<Cli, String> {
    require_positionals(&parsed, 1, "study")?;
    let path = PathBuf::from(parsed.positionals[0].clone());
    let out = PathBuf::from(take_required(&mut parsed, "out")?);
    let force = take_bool(&mut parsed, "force", false)?;
    reject_unknown(parsed)?;
    Ok(Cli::Study { path, out, force })
}

fn parse_studies(parsed: Parsed) -> Result<Cli, String> {
    require_positionals(&parsed, 0, "studies")?;
    reject_unknown(parsed)?;
    Ok(Cli::Studies)
}

fn parse_fit(mut parsed: Parsed) -> Result<Cli, String> {
    require_positionals(&parsed, 1, "fit")?;
    let font = PathBuf::from(parsed.positionals[0].clone());
    let study = PathBuf::from(take_required(&mut parsed, "study")?);
    let family = take_required(&mut parsed, "family")?;
    let style = parsed.flags.remove("style");
    let weight = match parsed.flags.remove("weight") {
        Some(value) => Some(
            value
                .parse::<u16>()
                .map_err(|_| format!("--weight must be a whole number, got {value}"))?,
        ),
        None => None,
    };
    let out = PathBuf::from(take_required(&mut parsed, "out")?);
    let force = take_bool(&mut parsed, "force", false)?;
    reject_unknown(parsed)?;
    Ok(Cli::Fit {
        font,
        study,
        family,
        style,
        weight,
        out,
        force,
    })
}

fn parse_run(mut parsed: Parsed) -> Result<Cli, String> {
    require_positionals(&parsed, 0, "run")?;
    let file = parsed.flags.remove("file").map(PathBuf::from);
    reject_unknown(parsed)?;
    Ok(Cli::Run { file })
}

fn parse_mcp(parsed: Parsed) -> Result<Cli, String> {
    require_positionals(&parsed, 0, "mcp")?;
    reject_unknown(parsed)?;
    Ok(Cli::Mcp)
}

fn one_path(parsed: Parsed, command: &str) -> Result<PathBuf, String> {
    require_positionals(&parsed, 1, command)?;
    reject_unknown(parsed.clone_flags())?;
    Ok(PathBuf::from(parsed.positionals[0].clone()))
}

fn two_paths(parsed: Parsed, command: &str) -> Result<(PathBuf, PathBuf), String> {
    require_positionals(&parsed, 2, command)?;
    reject_unknown(parsed.clone_flags())?;
    Ok((
        PathBuf::from(parsed.positionals[0].clone()),
        PathBuf::from(parsed.positionals[1].clone()),
    ))
}

impl Parsed {
    fn clone_flags(&self) -> Parsed {
        Parsed {
            positionals: Vec::new(),
            flags: self.flags.clone(),
        }
    }
}

fn require_positionals(parsed: &Parsed, count: usize, command: &str) -> Result<(), String> {
    if parsed.positionals.len() == count {
        Ok(())
    } else {
        Err(format!(
            "{command} expected {count} paths, got {}",
            parsed.positionals.len()
        ))
    }
}

fn take_bool(parsed: &mut Parsed, name: &str, default: bool) -> Result<bool, String> {
    match parsed.flags.remove(name).as_deref() {
        None => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(other) => Err(format!("--{name} must be true or false, got {other}")),
    }
}

fn take_required(parsed: &mut Parsed, name: &str) -> Result<String, String> {
    parsed
        .flags
        .remove(name)
        .ok_or_else(|| format!("--{name} is required"))
}

fn reject_unknown(parsed: Parsed) -> Result<(), String> {
    if let Some((name, _)) = parsed.flags.iter().next() {
        Err(format!("unknown flag --{name}"))
    } else {
        Ok(())
    }
}

fn parse_upm(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .map_err(|_| format!("--upm must be a whole number from 16 to 16384, got {value}"))
}

pub fn help_text() -> &'static str {
    "\
foundry — local type foundry commands

  foundry new --name NAME [--upm 1000] --out FILE [--force]
  foundry atlas-evaluate FONT --atlas SNAPSHOT --brief ID
  foundry atlas STUDIES... [--out FOLDER] [--labels FILE] [--family-aliases FILE] [--min-shared 12] [--neighbors 5] [--gaps 24]
  foundry atlas-query SNAPSHOT --family NAME [--limit 5]
  foundry atlas-briefs SNAPSHOT [--class CLASS] [--limit 5]
  foundry analyze PATH... [--out FOLDER] [--format json|csv|md] [--glyphs n,o,H,O] [--axes wght=600] [--weight-match 0.15] [--cache-dir FOLDER] [--sheets png,pdf] [--profile full|atlas]
  foundry info FILE
  foundry check A B
  foundry blend A B [--t 0.5] --out FILE [--force]
  foundry proof FILE --out IMAGE [--text TEXT] [--pixel-size 72] [--compare OTHER] [--force]
  foundry diff A B
  foundry studies
  foundry study PATH --out FILE [--force]
  foundry fit FONT --study STUDY --family NAME [--style NAME] [--weight N] --out FILE [--force]
  foundry run [--file FILE]
  foundry mcp

FILE, A, and B can be .json, .ufo, .ttf, .otf, .ttc, .otc, .woff, or a folder of SVG glyphs named 0041.svg. --out writes .json, .ufo, or .ttf. proof writes a PNG. --force replaces an existing file and keeps the previous one as name.bak.
`studies` lists study files kept with the Type Foundry and under %APPDATA%/Type Foundry/data/studies. The Type Foundry copy wins when the name is the same.
`study` measures letterform proportions in a font or a folder and writes them to FILE. Outlines are not stored. A repeated face is skipped. Pass --force to replace FILE. Put FILE in one of those study folders to use it later by name.
`fit` reads one family from a study and applies its x-height ratio, fitting, and vertical metrics to FONT. STUDY is a file or a library name such as klim-2026-10-17. Caps stay. Lowercase scales. Outlines are not copied. `--family The Future` matches a study family named `Test The Future`. No `--style` uses the text face. `--weight 900` picks the roman nearest that weight.
`run` reads JSON commands from stdin, or from --file. See documents/api.md.
`mcp` serves the same session as a stdio MCP server, like the foundry-mcp binary.
"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn parses_the_public_commands() {
        assert_eq!(parse(&args("")).unwrap(), Cli::Help);
        assert_eq!(
            parse(&args("new --name Wide --out wide.json")).unwrap(),
            Cli::New {
                name: "Wide".into(),
                upm: 1000,
                out: PathBuf::from("wide.json"),
                force: false,
            }
        );
        assert_eq!(
            parse(&args("new --name Wide --out wide.json --force")).unwrap(),
            Cli::New {
                name: "Wide".into(),
                upm: 1000,
                out: PathBuf::from("wide.json"),
                force: true,
            }
        );
        assert_eq!(
            parse(&args("blend a.json b.json --t 0.25 --out mid.json")).unwrap(),
            Cli::Blend {
                a: PathBuf::from("a.json"),
                b: PathBuf::from("b.json"),
                t: 0.25,
                out: PathBuf::from("mid.json"),
                force: false,
            }
        );
        assert_eq!(
            parse(&args("diff a.json b.json")).unwrap(),
            Cli::Diff {
                a: PathBuf::from("a.json"),
                b: PathBuf::from("b.json"),
            }
        );
        assert!(parse(&args("blend a.json --out mid.json")).is_err());
        assert!(parse(&args("new --name Wide")).is_err());
        assert_eq!(parse(&args("studies")).unwrap(), Cli::Studies);
        assert!(parse(&args("studies extra")).is_err());
        assert_eq!(parse(&args("mcp")).unwrap(), Cli::Mcp);
        assert!(parse(&args("mcp extra")).is_err());
        assert_eq!(
            parse(&args(
                "fit face.json --study study.json --family Future --weight 900 --out fitted.json"
            ))
            .unwrap(),
            Cli::Fit {
                font: PathBuf::from("face.json"),
                study: PathBuf::from("study.json"),
                family: "Future".into(),
                style: None,
                weight: Some(900),
                out: PathBuf::from("fitted.json"),
                force: false,
            }
        );
    }
}
#[cfg(test)]
mod analyzer_args_tests {
    use super::*;
    #[test]
    fn analyze_options_and_bad_inputs() {
        let argv = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        let Cli::Analyze{paths,options}=parse(&argv("analyze a.ttf b.ufo --axes wght=500,wdth=90 --glyphs n,o,H --format csv --out reports --cache-dir cache")).unwrap()else{panic!("wrong command");};
        assert_eq!(paths.len(), 2);
        assert_eq!(options.axes["wght"], 500.);
        assert_eq!(options.glyphs, vec!["n", "o", "H"]);
        assert_eq!(options.format, foundry_core::analyze::OutputFormat::Csv);
        for bad in [
            "analyze",
            "analyze a.ttf --format bad",
            "analyze a.ttf --axes wght=1,wght=2",
            "analyze a.ttf --axes wght",
            "analyze a.ttf --typo true",
        ] {
            assert!(parse(&argv(bad)).is_err(), "{bad}");
        }
    }
}
#[cfg(test)]
mod atlas_args_tests {
    use super::*;
    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }
    #[test]
    fn atlas_build_query_brief_and_evaluate_options() {
        assert!(matches!(
            parse(&args("atlas store --out library --min-shared 12 --gaps 20")).unwrap(),
            Cli::Atlas { .. }
        ));
        assert!(matches!(
            parse(&args("atlas-query library/latest.json --family Vostok")).unwrap(),
            Cli::AtlasQuery { .. }
        ));
        assert!(matches!(
            parse(&args("atlas-briefs library/latest.json --limit 3")).unwrap(),
            Cli::AtlasBriefs { .. }
        ));
        assert!(matches!(
            parse(&args(
                "atlas-evaluate new.ttf --atlas library/latest.json --brief 00000000"
            ))
            .unwrap(),
            Cli::AtlasEvaluate { .. }
        ));
        for bad in [
            "atlas",
            "atlas-query a.json",
            "atlas-evaluate a.ttf --atlas a.json",
            "atlas a.json --gaps bad",
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad}");
        }
    }
}
