mod args;

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use foundry_api::{Command, Session};

fn main() -> ExitCode {
    enable_utf8_console();
    let argv: Vec<String> = env::args().skip(1).collect();
    let cli = match args::parse(&argv) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("{err}");
            eprintln!();
            eprint!("{}", args::help_text());
            return ExitCode::from(2);
        }
    };
    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: args::Cli) -> Result<ExitCode, String> {
    match cli {
        args::Cli::Help => {
            print!("{}", args::help_text());
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::Version => {
            let hash = option_env!("FOUNDRY_GIT_HASH").unwrap_or("unknown");
            println!("foundry 0.1.0 ({hash})");
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::New {
            name,
            upm,
            out,
            force,
        } => {
            let mut session = Session::new();
            let created = session.execute(Command::Create { name, upm });
            ensure_ok(created)?;
            let saved = session.execute(Command::Save {
                ttf_tolerance: None,
                path: path_string(&out),
                force,
            });
            let data = ensure_ok(saved)?;
            print_ttf_report(&data);
            println!("Wrote {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::AtlasEvaluate { font, path, brief } => {
            let data = ensure_ok(Session::new().execute(Command::AtlasEvaluate {
                font,
                path,
                brief,
                options: foundry_core::analyze::Options {
                    profile: foundry_core::analyze::MeasurementProfile::Atlas,
                    ..Default::default()
                },
            }))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?
            );
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::Atlas { paths, options } => {
            let data = ensure_ok(Session::new().execute(Command::AtlasBuild { paths, options }))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?
            );
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::AtlasQuery {
            path,
            family,
            limit,
            min_shared,
        } => {
            let data = ensure_ok(Session::new().execute(Command::AtlasQuery {
                path,
                family,
                limit,
                min_shared,
            }))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?
            );
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::AtlasBriefs { path, class, limit } => {
            let data =
                ensure_ok(Session::new().execute(Command::AtlasBriefs { path, class, limit }))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?
            );
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::Analyze { paths, options } => {
            let mut session = Session::new();
            let data = ensure_ok(session.execute(Command::Analyze {
                paths,
                options: options.clone(),
            }))?;
            let report: foundry_core::analyze::Report =
                serde_json::from_value(data).map_err(|e| e.to_string())?;
            print!(
                "{}",
                foundry_core::analyze::render(&report, options.format)?
            );
            Ok(if report.errors.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        args::Cli::Info { path } => {
            let mut session = Session::new();
            let opened = session.execute(Command::Open {
                path: path_string(&path),
            });
            let data = ensure_ok(opened)?;
            print_info(&data);
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::Check { a, b } => {
            let mut session = Session::new();
            let response = session.execute(Command::Check {
                a: path_string(&a),
                b: path_string(&b),
            });
            if response.ok {
                println!("Compatible.");
                Ok(ExitCode::SUCCESS)
            } else {
                print_failure(&response);
                Ok(ExitCode::from(1))
            }
        }
        args::Cli::Blend {
            a,
            b,
            t,
            out,
            force,
        } => {
            let mut session = Session::new();
            let response = session.execute(Command::Blend {
                a: path_string(&a),
                b: path_string(&b),
                t,
                out: path_string(&out),
                force,
            });
            if response.ok {
                let data = response.data.unwrap_or(serde_json::Value::Null);
                println!(
                    "Wrote {}",
                    data["path"]
                        .as_str()
                        .unwrap_or(out.to_str().unwrap_or("the output"))
                );
                println!("Name: {}", data["name"].as_str().unwrap_or(""));
                Ok(ExitCode::SUCCESS)
            } else {
                print_failure(&response);
                Ok(ExitCode::from(1))
            }
        }
        args::Cli::Proof {
            font,
            out,
            text,
            pixel_size,
            guides,
            boxes,
            compare,
            force,
        } => {
            let mut session = Session::new();
            let opened = session.execute(Command::Open {
                path: path_string(&font),
            });
            ensure_ok(opened)?;
            let response = session.execute(Command::Proof {
                path: path_string(&out),
                text: text.unwrap_or_default(),
                pixel_size,
                guides,
                boxes,
                compare: compare.map(|path| path_string(&path)),
                force,
            });
            let data = ensure_ok(response)?;
            println!(
                "Wrote {} ({}x{})",
                data["path"].as_str().unwrap_or("the proof"),
                data["width"].as_u64().unwrap_or(0),
                data["height"].as_u64().unwrap_or(0)
            );
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::Diff { a, b } => {
            let mut session = Session::new();
            let response = session.execute(Command::Diff {
                a: path_string(&a),
                b: path_string(&b),
            });
            let data = ensure_ok(response)?;
            let count = data["count"].as_u64().unwrap_or(0);
            println!("{count} glyphs differ");
            if let Some(glyphs) = data["glyphs"].as_array() {
                for glyph in glyphs {
                    println!(
                        "{}: {}",
                        glyph["name"].as_str().unwrap_or(""),
                        glyph["detail"].as_str().unwrap_or("")
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::Fit {
            font,
            study,
            family,
            style,
            weight,
            out,
            force,
        } => {
            let mut session = Session::new();
            let opened = session.execute(Command::Open {
                path: path_string(&font),
            });
            ensure_ok(opened)?;
            let response = session.execute(Command::FitStudy {
                study: path_string(&study),
                family,
                style,
                weight,
            });
            let data = ensure_ok(response)?;
            print_fit(&data);
            let saved = session.execute(Command::Save {
                ttf_tolerance: None,
                path: path_string(&out),
                force,
            });
            let data = ensure_ok(saved)?;
            print_ttf_report(&data);
            println!("Wrote {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::Study { path, out, force } => {
            eprintln!("Measuring letterforms. Outlines are not kept.");
            let mut session = Session::new();
            let response = session.execute(Command::Study {
                path: path_string(&path),
                out: path_string(&out),
                force,
                progress: true,
            });
            let data = ensure_ok(response)?;
            print_study(&data);
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::Studies => {
            let mut session = Session::new();
            let response = session.execute(Command::ListStudies);
            let data = ensure_ok(response)?;
            print_studies(&data);
            Ok(ExitCode::SUCCESS)
        }
        args::Cli::Run { file } => run_stream(file),
        args::Cli::Mcp => {
            let stdin = io::stdin();
            foundry_mcp::serve(stdin.lock(), io::stdout().lock(), io::stderr().lock())
                .map_err(|err| err.to_string())?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn run_stream(file: Option<std::path::PathBuf>) -> Result<ExitCode, String> {
    let reader: Box<dyn BufRead> = if let Some(path) = file {
        let handle = fs::File::open(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        Box::new(io::BufReader::new(handle))
    } else {
        Box::new(io::BufReader::new(io::stdin()))
    };
    let mut session = Session::new();
    let mut failed = false;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in reader.lines() {
        let line = line.map_err(|err| err.to_string())?;
        let Some(response) = session.execute_line(&line) else {
            continue;
        };
        failed |= !response.ok;
        write_response(&mut out, &response)?;
    }
    if failed {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn ensure_ok(response: foundry_api::Response) -> Result<serde_json::Value, String> {
    if response.ok {
        Ok(response.data.unwrap_or(serde_json::Value::Null))
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "command failed".to_string()))
    }
}

fn print_ttf_report(data: &serde_json::Value) {
    if let Some(report) = data.get("ttf").filter(|r| r.is_object()) {
        println!(
            "TTF: {} cubics split; max error {:.3} units; {} segments exceed tolerance after rounding",
            report["split_segments"].as_u64().unwrap_or(0),
            report["max_error"].as_f64().unwrap_or(0.0),
            report["tolerance_exceeded"].as_u64().unwrap_or(0)
        );
    }
}

fn print_fit(data: &serde_json::Value) {
    println!(
        "Fitted to {} {}",
        data["family"].as_str().unwrap_or(""),
        data["style"].as_str().unwrap_or("")
    );
    println!(
        "x/cap {:.3} -> {:.3}, lowercase scale {:.3}",
        data["x_over_cap_before"].as_f64().unwrap_or(0.0),
        data["x_over_cap_after"].as_f64().unwrap_or(0.0),
        data["scale"].as_f64().unwrap_or(0.0)
    );
    if let Some(bearing) = data["sidebearing"].as_f64() {
        println!("sidebearing {bearing:.1}");
    }
    println!(
        "ascender {:.1}, cap {:.1}, x-height {:.1}, descender {:.1}",
        data["ascender"].as_f64().unwrap_or(0.0),
        data["cap_height"].as_f64().unwrap_or(0.0),
        data["x_height"].as_f64().unwrap_or(0.0),
        data["descender"].as_f64().unwrap_or(0.0)
    );
    match (
        data["overshoot_drawn"].as_f64(),
        data["overshoot_study"].as_f64(),
    ) {
        (Some(drawn), Some(target)) => println!(
            "o overshoot is {drawn:.3} of the em; the study face is {target:.3}. Outlines were not reshaped to match it."
        ),
        (None, Some(target)) => println!(
            "The study face overshoots by {target:.3} of the em. This font has no o to compare."
        ),
        _ => {}
    }
}

fn print_studies(data: &serde_json::Value) {
    let Some(studies) = data["studies"].as_array() else {
        println!("No studies.");
        return;
    };
    if studies.is_empty() {
        println!("No studies.");
        println!(
            "Add a typefoundry.study file under the Type Foundry studies folder, or pass --out to foundry study."
        );
        return;
    }
    for study in studies {
        println!(
            "{}  {} faces, {} families  {}",
            study["name"].as_str().unwrap_or(""),
            study["faces"].as_u64().unwrap_or(0),
            study["families"].as_u64().unwrap_or(0),
            study["path"].as_str().unwrap_or("")
        );
    }
}

fn print_study(data: &serde_json::Value) {
    println!("Wrote {}", data["path"].as_str().unwrap_or("the study"));
    println!(
        "{} faces, {} families, {} duplicates",
        data["faces"].as_u64().unwrap_or(0),
        data["families"].as_u64().unwrap_or(0),
        data["duplicates"].as_u64().unwrap_or(0)
    );
    if let Some(notes) = data["notes"].as_array() {
        for note in notes {
            if let Some(line) = note.as_str() {
                println!("{line}");
            }
        }
    }
    if let Some(families) = data["family_notes"].as_array() {
        println!();
        for family in families {
            if let Some(line) = family.as_str() {
                println!("{line}");
            }
        }
    }
    if let Some(failed) = data["failed"].as_array()
        && !failed.is_empty()
    {
        println!();
        println!("{} files could not be read", failed.len());
        for failure in failed.iter().take(12) {
            println!(
                "{}: {}",
                failure["path"].as_str().unwrap_or(""),
                failure["reason"].as_str().unwrap_or("")
            );
        }
    }
}

fn print_info(data: &serde_json::Value) {
    println!("Name: {}", data["name"].as_str().unwrap_or(""));
    println!("Family: {}", data["style"]["family"].as_str().unwrap_or(""));
    println!("Style: {}", data["style"]["name"].as_str().unwrap_or(""));
    println!("Weight: {}", data["style"]["weight"].as_u64().unwrap_or(0));
    println!(
        "Width class: {}",
        data["style"]["width"].as_u64().unwrap_or(5)
    );
    println!(
        "Italic: {}",
        data["style"]["italic"].as_bool().unwrap_or(false)
    );
    println!("UPM: {}", data["upm"].as_u64().unwrap_or(0));
    println!(
        "Ascender: {}",
        data["metrics"]["ascender"].as_f64().unwrap_or(0.0)
    );
    println!(
        "Cap height: {}",
        data["metrics"]["cap_height"].as_f64().unwrap_or(0.0)
    );
    println!(
        "X-height: {}",
        data["metrics"]["x_height"].as_f64().unwrap_or(0.0)
    );
    println!(
        "Descender: {}",
        data["metrics"]["descender"].as_f64().unwrap_or(0.0)
    );
    println!(
        "Coverage: {} glyphs, {} encoded",
        data["coverage"]["glyphs"].as_u64().unwrap_or(0),
        data["coverage"]["encoded"].as_u64().unwrap_or(0)
    );
    println!(
        "Kerning: {} pairs, {} groups, {} ligatures",
        data["kerning"]["pairs"].as_u64().unwrap_or(0),
        data["kerning"]["groups"].as_u64().unwrap_or(0),
        data["kerning"]["ligatures"].as_u64().unwrap_or(0)
    );
    let glyphs = data["glyphs"].as_array().map(|items| {
        items
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    });
    println!("Glyphs: {}", glyphs.unwrap_or_default());
}

/// JSON for `foundry run`. Non-ASCII characters inside strings are `\u` escapes, so PowerShell
/// 5.1 can redirect the stream with its default code page. The console itself is set to UTF-8
/// for the lines a person reads.
fn write_response(out: &mut impl Write, response: &foundry_api::Response) -> Result<(), String> {
    let text = serde_json::to_string(response).map_err(|err| err.to_string())?;
    writeln!(out, "{}", escape_non_ascii(&text)).map_err(|err| err.to_string())
}

fn escape_non_ascii(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(ch) = chars.next() {
        if !in_string {
            if ch == '"' {
                in_string = true;
            }
            out.push(ch);
            continue;
        }
        if ch == '\\' {
            out.push(ch);
            if let Some(next) = chars.next() {
                out.push(next);
            }
            continue;
        }
        if ch == '"' {
            in_string = false;
            out.push(ch);
            continue;
        }
        let code = u32::from(ch);
        if code < 0x80 {
            out.push(ch);
        } else if code <= 0xFFFF {
            out.push_str(&format!("\\u{code:04x}"));
        } else {
            let shifted = code - 0x10000;
            let high = 0xD800 + (shifted >> 10);
            let low = 0xDC00 + (shifted & 0x3FF);
            out.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
        }
    }
    out
}

fn enable_utf8_console() {
    #[cfg(windows)]
    unsafe {
        SetConsoleOutputCP(65001);
        SetConsoleCP(65001);
    }
}

#[cfg(windows)]
unsafe extern "system" {
    fn SetConsoleOutputCP(code_page: u32) -> i32;
    fn SetConsoleCP(code_page: u32) -> i32;
}

fn print_failure(response: &foundry_api::Response) {
    if let Some(error) = &response.error {
        eprintln!("{error}");
    }
}

fn path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::escape_non_ascii;

    #[test]
    fn json_strings_escape_non_ascii() {
        let text = r#"{"ok":true,"data":{"name":"Vostok café"}}"#;
        let escaped = escape_non_ascii(text);
        assert!(escaped.contains(r"caf\u00e9"), "{escaped}");
        assert!(!escaped.contains('é'));
        assert!(escaped.contains(r#""ok":true"#));
    }
}
