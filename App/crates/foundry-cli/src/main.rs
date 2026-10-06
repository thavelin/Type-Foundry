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
                path: path_string(&out),
                force,
            });
            ensure_ok(saved)?;
            println!("Wrote {}", out.display());
            Ok(ExitCode::SUCCESS)
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
