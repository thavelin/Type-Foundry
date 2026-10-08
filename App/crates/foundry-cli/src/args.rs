use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum Cli {
    Help,
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
        "info" => parse_info(parsed),
        "check" => parse_check(parsed),
        "blend" => parse_blend(parsed),
        "proof" => parse_proof(parsed),
        "diff" => parse_diff(parsed),
        "run" => parse_run(parsed),
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
  foundry info FILE
  foundry check A B
  foundry blend A B [--t 0.5] --out FILE [--force]
  foundry proof FILE --out IMAGE [--text TEXT] [--pixel-size 72] [--compare OTHER] [--force]
  foundry diff A B
  foundry run [--file FILE]
  foundry mcp

FILE, A, and B can be .json, .ufo, .ttf, .otf, .ttc, .otc, .woff, or a folder of SVG glyphs named 0041.svg. --out writes .json, .ufo, or .ttf. proof writes a PNG. --force replaces an existing file and keeps the previous one as name.bak.
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
        assert_eq!(parse(&args("mcp")).unwrap(), Cli::Mcp);
        assert!(parse(&args("mcp extra")).is_err());
    }
}
