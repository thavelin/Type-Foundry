//! A stdio MCP server. Every tool is one [`foundry_api::Command`] run on one [`Session`].
//!
//! Stdout carries only protocol messages, one JSON object per line. Logs go to the separate
//! log writer, which the binaries point at stderr. No tool uploads a font or reaches the network.

use std::io::{self, BufRead, Write};

use foundry_api::{Command, Response, Session};
use serde_json::{Map, Value, json};

pub const SERVER_NAME: &str = "typefoundry";
pub const SERVER_VERSION: &str = "0.1.0";
/// Used when a client's `initialize` does not name a protocol version.
pub const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// The command session plus, per open font, the path `font_save` falls back to.
#[derive(Debug, Default)]
pub struct Server {
    session: Session,
    paths: std::collections::HashMap<u32, String>,
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Answer one JSON-RPC message. Notifications, which have no `id`, return `None`.
    pub fn handle_message(&mut self, message: &Value) -> Option<Value> {
        let Some(object) = message.as_object() else {
            return Some(error(
                Value::Null,
                INVALID_REQUEST,
                "expected a JSON object",
            ));
        };
        let id = match object.get("id") {
            None => None,
            Some(id @ (Value::Number(_) | Value::String(_))) => Some(id.clone()),
            Some(_) => {
                return Some(error(
                    Value::Null,
                    INVALID_REQUEST,
                    "id must be a number or a string",
                ));
            }
        };
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return id.map(|id| error(id, INVALID_REQUEST, "method is missing"));
        };
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        let id = id?;

        let outcome = match method {
            "initialize" => Ok(initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_list() })),
            "tools/call" => self.call_tool(&params),
            other => Err((METHOD_NOT_FOUND, format!("unknown method {other}"))),
        };
        Some(match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => error(id, code, &message),
        })
    }

    /// Parse and answer one line from the stream. Blank lines return `None`.
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if trimmed.is_empty() {
            return None;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(message) => self.handle_message(&message),
            Err(err) => Some(error(
                Value::Null,
                PARSE_ERROR,
                &format!("could not read message: {err}"),
            )),
        }
    }

    fn call_tool(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| (INVALID_PARAMS, "tools/call needs a tool name".to_string()))?;
        let mut args = match params.get("arguments") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(args)) => args.clone(),
            Some(_) => return Err((INVALID_PARAMS, "arguments must be an object".to_string())),
        };
        let op = match name {
            "font_create" => "create",
            "font_open" => "open",
            "font_save" => "save",
            "font_info" => "info",
            "font_glyphs" => "glyphs",
            "glyph_get" => "glyph",
            "point_move" => "move_point",
            "font_check" => "check",
            "font_blend" => "blend",
            "font_list" => "fonts",
            "font_select" => "select_font",
            "font_close" => "close_font",
            "style_set" => "set_style",
            "style_derive" => "derive_style",
            "points_set" => "set_points",
            "family_check" => "family_check",
            "family_open" => "open_family",
            "family_save" => "save_family",
            "family_export" => "export_family",
            "info_set" => "set_info",
            "kern_set" => "set_kerning",
            "kern_add" => "add_kern",
            "group_set" => "set_group",
            "ligature_add" => "add_ligature",
            "features_set" => "set_features",
            "path_offset" => "offset",
            "path_stroke" => "stroke",
            "sidebearing_set" => "set_sidebearing",
            "outline_check" => "check_outlines",
            "spacing_check" => "check_spacing",
            "font_diff" => "diff",
            "font_proof" => "proof",
            "glyph_move" => "move_glyph",
            "family_copy" => "copy_family",
            "undo" => "undo",
            "redo" => "redo",
            "width_scale" => "scale_width",
            "slant" => "slant",
            "font_measure" => "measure",
            "genome_capture" => "capture_genome",
            "genome_check" => "check_genome",
            "font_audit" => "audit",
            "decision_record" => "record_decision",
            "decision_list" => "list_decisions",
            other => return Err((INVALID_PARAMS, format!("unknown tool {other}"))),
        };

        if op == "save" && !args.contains_key("path") {
            let remembered = self.session.active().and_then(|id| self.paths.get(&id));
            match remembered {
                Some(path) => {
                    args.insert("path".to_string(), Value::String(path.clone()));
                }
                None => {
                    return Ok(tool_result(
                        "font_save needs a path: this font has not been opened from or saved to a file yet",
                        true,
                    ));
                }
            }
        }
        args.insert("op".to_string(), Value::String(op.to_string()));
        let command = match serde_json::from_value::<Command>(Value::Object(args.clone())) {
            Ok(command) => command,
            Err(err) => {
                return Ok(tool_result(
                    &format!("bad arguments for {name}: {err}"),
                    true,
                ));
            }
        };

        let response = self.session.execute(command);
        if response.ok {
            self.remember_paths(op, &args, response.data.as_ref());
        }
        Ok(response_result(&response))
    }

    /// The file that holds each open font. A new or derived style has none until it is saved.
    fn remember_paths(&mut self, op: &str, args: &Map<String, Value>, data: Option<&Value>) {
        let key = match op {
            "open" | "save" => "path",
            "blend" => "out",
            "open_family" => {
                for entry in data
                    .and_then(|data| data["opened"].as_array())
                    .into_iter()
                    .flatten()
                {
                    if let (Some(id), Some(path)) = (entry["id"].as_u64(), entry["path"].as_str()) {
                        self.paths.insert(id as u32, path.replace('\\', "/"));
                    }
                }
                return;
            }
            "close_font" => {
                let open: Vec<u32> = data
                    .and_then(|data| data["fonts"].as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|font| font["id"].as_u64().map(|id| id as u32))
                    .collect();
                self.paths.retain(|id, _| open.contains(id));
                return;
            }
            _ => return,
        };
        if let (Some(id), Some(path)) =
            (self.session.active(), args.get(key).and_then(Value::as_str))
        {
            self.paths.insert(id, path.to_string());
        }
    }
}

/// Serve newline-delimited JSON-RPC until `input` ends. Only responses are written to `output`.
pub fn serve(input: impl BufRead, mut output: impl Write, mut log: impl Write) -> io::Result<()> {
    writeln!(log, "{SERVER_NAME} MCP server {SERVER_VERSION} on stdio")?;
    let mut server = Server::new();
    for line in input.lines() {
        let line = line?;
        let Some(reply) = server.handle_line(&line) else {
            continue;
        };
        if let Some(message) = reply.get("error").and_then(|err| err.get("message")) {
            writeln!(log, "request failed: {message}")?;
        }
        serde_json::to_writer(&mut output, &reply)?;
        writeln!(output)?;
        output.flush()?;
    }
    Ok(())
}

fn initialize(params: &Value) -> Value {
    let version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
    })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

fn tool_result(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

fn response_result(response: &Response) -> Value {
    let text = serde_json::to_string(response)
        .unwrap_or_else(|err| format!("{{\"ok\":false,\"error\":\"{err}\"}}"));
    tool_result(&text, !response.ok)
}

fn tool_list() -> Value {
    let path = json!({
        "type": "string",
        "description": "A typefoundry.font .json file, a .ufo directory, a folder of SVG glyphs named 0041.svg, or a .ttf, .otf, .ttc, .otc, or .woff font. Saving writes .json, .ufo, or .ttf. Use forward slashes."
    });
    let name = json!({ "type": "string", "description": "Glyph name." });
    let none = json!({ "type": "object", "properties": {} });
    let mut tools = match json!([
        {
            "name": "font_create",
            "description": "Start an empty font in the session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "upm": { "type": "integer", "minimum": 16, "maximum": 16384, "default": 1000 }
                },
                "required": ["name"]
            }
        },
        {
            "name": "font_open",
            "description": "Open a .json font, a .ufo directory, a folder of SVG glyphs named 0041.svg, or a .ttf, .otf, .ttc, .otc, or .woff font from local disk.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": path },
                "required": ["path"]
            }
        },
        {
            "name": "font_save",
            "description": "Save the open font to local disk. Without a path, saves to the last path opened or saved by this server. Refuses to replace an existing file unless force is true, and then keeps the previous file as name.bak.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": path,
                    "force": { "type": "boolean", "default": false }
                }
            }
        },
        {
            "name": "font_info",
            "description": "Name, units per em, vertical metrics, style, name-table fields, coverage, kerning counts, and glyph names of the open font.",
            "inputSchema": none
        },
        {
            "name": "font_glyphs",
            "description": "Glyph names of the open font.",
            "inputSchema": none
        },
        {
            "name": "glyph_get",
            "description": "One glyph: unicode, advance, and contours of on and off points.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": name },
                "required": ["name"]
            }
        },
        {
            "name": "point_move",
            "description": "Move one point of the open font to absolute font coordinates. contour and point are zero-based.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": name,
                    "contour": { "type": "integer", "minimum": 0 },
                    "point": { "type": "integer", "minimum": 0 },
                    "x": { "type": "number" },
                    "y": { "type": "number" }
                },
                "required": ["name", "contour", "point", "x", "y"]
            }
        },
        {
            "name": "font_check",
            "description": "Report whether two font files can be blended, and why not.",
            "inputSchema": {
                "type": "object",
                "properties": { "a": path, "b": path },
                "required": ["a", "b"]
            }
        },
        {
            "name": "font_blend",
            "description": "Blend two compatible font files, write the result to out, and open it. t is 0 at a and 1 at b.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "a": path,
                    "b": path,
                    "t": { "type": "number", "default": 0.5 },
                    "out": path,
                    "force": { "type": "boolean", "default": false }
                },
                "required": ["a", "b", "out"]
            }
        },
        {
            "name": "font_list",
            "description": "List every open font with its id, family, style, weight, italic flag, and whether it has unsaved changes. Other tools act on the active font.",
            "inputSchema": none
        },
        {
            "name": "font_select",
            "description": "Make an open font the active one, by id from font_list.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer" } },
                "required": ["id"]
            }
        },
        {
            "name": "font_close",
            "description": "Close an open font by id, or the active font. Unsaved changes are lost.",
            "inputSchema": { "type": "object", "properties": { "id": { "type": "integer" } } }
        },
        {
            "name": "style_set",
            "description": "Set the active font's family name, style name, weight (1-1000), italic flag, or italic angle (degrees counter-clockwise, -12 leans right).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "family": { "type": "string" },
                    "style": { "type": "string" },
                    "weight": { "type": "integer", "minimum": 1, "maximum": 1000 },
                    "italic": { "type": "boolean" },
                    "italic_angle": { "type": "number" },
                    "width": { "type": "integer", "minimum": 1, "maximum": 9, "description": "OS/2 width class. 1 is ultra-condensed, 5 is normal, 9 is ultra-expanded." }
                }
            }
        },
        {
            "name": "style_derive",
            "description": "Copy the active font as a new style of its family and make the copy active. slant leans every glyph that many degrees, to start an italic.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "style": { "type": "string", "description": "For example Italic or Bold." },
                    "weight": { "type": "integer", "minimum": 1, "maximum": 1000 },
                    "italic": { "type": "boolean" },
                    "slant": { "type": "number", "default": 0 }
                },
                "required": ["style"]
            }
        },
        {
            "name": "points_set",
            "description": "Move several points of one glyph to absolute positions, as one undo step. Used for alignment.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "points": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "contour": { "type": "integer" },
                                "point": { "type": "integer" },
                                "x": { "type": "number" },
                                "y": { "type": "number" }
                            },
                            "required": ["contour", "point", "x", "y"]
                        }
                    }
                },
                "required": ["name", "points"]
            }
        },
        {
            "name": "family_check",
            "description": "Check the open styles of the active font's family: matching family names, distinct styles, units per em, line metrics, and glyph coverage.",
            "inputSchema": none
        },
        {
            "name": "family_open",
            "description": "Open a .family.json file and every style it lists.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": path },
                "required": ["path"]
            }
        },
        {
            "name": "family_save",
            "description": "Save every open style of the active font's family as Family-Style.json beside a .family.json file at path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": path,
                    "force": { "type": "boolean", "default": false }
                },
                "required": ["path"]
            }
        },
        {
            "name": "family_export",
            "description": "Write every open style of the active font's family into dir as Family-Style.ttf, .ufo, or .json, with names set so apps group them as one family.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "dir": path,
                    "format": { "type": "string", "enum": ["ttf", "ufo", "json"] },
                    "force": { "type": "boolean", "default": false }
                },
                "required": ["dir", "format"]
            }
        }
    ]) {
        Value::Array(items) => items,
        other => vec![other],
    };
    if let Value::Array(more) = json!([
        {
            "name": "info_set",
            "description": "Set name-table fields on the open font: copyright, designer, license, license URL, version, a four-character vendor id, and the unique id. Empty strings clear a field. Omitted fields stay as they are.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "copyright": { "type": "string" },
                    "designer": { "type": "string" },
                    "license": { "type": "string" },
                    "license_url": { "type": "string" },
                    "version": { "type": "string" },
                    "vendor": { "type": "string" },
                    "unique_id": { "type": "string" }
                }
            }
        },
        {
            "name": "kern_set",
            "description": "Replace the open font's kerning groups, pairs, and ligatures. An empty object clears them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kerning": {
                        "type": "object",
                        "properties": {
                            "groups": { "type": "object" },
                            "pairs": { "type": "array" },
                            "ligatures": { "type": "array" }
                        }
                    }
                },
                "required": ["kerning"]
            }
        },
        {
            "name": "kern_add",
            "description": "Set one kerning pair. left and right are glyph names or group names such as public.kern1.round.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "left": { "type": "string" },
                    "right": { "type": "string" },
                    "value": { "type": "number" }
                },
                "required": ["left", "right", "value"]
            }
        },
        {
            "name": "group_set",
            "description": "Set one kerning group. Use a public.kern1 name for the left side and public.kern2 for the right side.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "members": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["name", "members"]
            }
        },
        {
            "name": "ligature_add",
            "description": "Add a ligature, such as glyphs f and i becoming fi. It is written to UFO features and to a GSUB liga lookup in a TTF.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "glyphs": { "type": "array", "items": { "type": "string" } },
                    "name": { "type": "string" }
                },
                "required": ["glyphs", "name"]
            }
        },
        {
            "name": "features_set",
            "description": "Replace the OpenType feature text. Omit text to keep a UFO's features.fea. An empty string clears it.",
            "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } } }
        },
        {
            "name": "path_offset",
            "description": "Thicken or thin outlines. horizontal and vertical are separate, so stems can grow more than hairlines. Existing points move and none are added, which keeps the font compatible for blending. add_points with corner round is the one exception: sharp outside corners become arcs, which adds points. keep_metrics leaves the baseline, x-height, and cap height. preview returns the outlines and does not change the font.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "horizontal": { "type": "number", "default": 0 },
                    "vertical": { "type": "number", "default": 0 },
                    "gap": { "type": "number", "default": 0, "description": "Stop growth when edges are this close. 0 disables the limit." },
                    "corner": { "type": "string", "enum": ["miter", "round", "angle"], "default": "angle" },
                    "sidebearing": { "type": "boolean", "default": false },
                    "keep_metrics": { "type": "boolean", "default": true },
                    "add_points": { "type": "boolean", "default": false, "description": "Draw round joins as arcs. Needs corner round. Changes the point count." },
                    "names": { "type": "array", "items": { "type": "string" } },
                    "preview": { "type": "boolean", "default": false }
                }
            }
        },
        {
            "name": "path_stroke",
            "description": "Make an outline or inline style from the same offset. Each contour becomes two, so the result does not blend with the original master.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["outline", "inline"] },
                    "horizontal": { "type": "number", "default": 0 },
                    "vertical": { "type": "number", "default": 0 },
                    "gap": { "type": "number", "default": 0 },
                    "corner": { "type": "string", "enum": ["miter", "round", "angle"], "default": "angle" },
                    "sidebearing": { "type": "boolean", "default": false },
                    "keep_metrics": { "type": "boolean", "default": true },
                    "add_points": { "type": "boolean", "default": false, "description": "Draw round joins as arcs. Needs corner round." },
                    "names": { "type": "array", "items": { "type": "string" } },
                    "preview": { "type": "boolean", "default": false }
                },
                "required": ["kind"]
            }
        },
        {
            "name": "sidebearing_set",
            "description": "Set one glyph's left or right sidebearing. family true applies it to every open style of the family, with one undo step on each style.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": name,
                    "side": { "type": "string", "enum": ["left", "right"] },
                    "value": { "type": "number" },
                    "family": { "type": "boolean", "default": false }
                },
                "required": ["name", "side", "value"]
            }
        },
        {
            "name": "outline_check",
            "description": "Flag self-intersections, kinks, wrong contour direction, missing overshoot, and glyphs that sit off the baseline or cap height.",
            "inputSchema": none
        },
        {
            "name": "spacing_check",
            "description": "Measure sidebearings along the italic slant and flag pairs closer than min_gap.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "min_gap": { "type": "number", "default": 0 },
                    "pairs": { "type": "array", "items": { "type": "array", "items": { "type": "string" } } }
                }
            }
        },
        {
            "name": "font_diff",
            "description": "List glyphs that differ between two font files, with the point or value that changed.",
            "inputSchema": {
                "type": "object",
                "properties": { "a": path, "b": path },
                "required": ["a", "b"]
            }
        },
        {
            "name": "font_proof",
            "description": "Draw the open font to a PNG, with metric guides and spacing boxes. text omitted draws the encoded character set. compare draws a second font underneath.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": path,
                    "text": { "type": "string" },
                    "pixel_size": { "type": "number", "default": 72 },
                    "guides": { "type": "boolean", "default": true },
                    "boxes": { "type": "boolean", "default": true },
                    "compare": path,
                    "force": { "type": "boolean", "default": false }
                },
                "required": ["path"]
            }
        },
        {
            "name": "glyph_move",
            "description": "Move a glyph to a new index in the font. index is the position after the move.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": name,
                    "index": { "type": "integer", "minimum": 0 }
                },
                "required": ["name", "index"]
            }
        },
        {
            "name": "family_copy",
            "description": "Copy a family file and its styles into another folder. family renames every style. label is stored on the new family file, so a version folder is one step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": path,
                    "dir": path,
                    "family": { "type": "string" },
                    "label": { "type": "string" },
                    "force": { "type": "boolean", "default": false }
                },
                "required": ["path", "dir"]
            }
        },
        {
            "name": "undo",
            "description": "Undo the last edit on the active font.",
            "inputSchema": none
        },
        {
            "name": "redo",
            "description": "Redo an undone edit on the active font.",
            "inputSchema": none
        },
        {
            "name": "width_scale",
            "description": "Change a glyph's width and keep vertical stem thickness. Counters scale. Sidebearings scale. factor 1 leaves the glyph alone.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "factor": { "type": "number" },
                    "names": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["factor"]
            }
        },
        {
            "name": "slant",
            "description": "Slant the open font in place and shift each glyph so the ink stays centred in its advance. Sets italic and the italic angle.",
            "inputSchema": {
                "type": "object",
                "properties": { "degrees": { "type": "number" } },
                "required": ["degrees"]
            }
        },
        {
            "name": "font_measure",
            "description": "Live Style Genome measurements: vertical metrics, per-glyph stems and sidebearings.",
            "inputSchema": none
        },
        {
            "name": "genome_capture",
            "description": "Compute a Style Genome from sample glyphs and store it on the font lib. stem_tolerance is absolute units (default 4).",
            "inputSchema": {
                "type": "object",
                "properties": { "stem_tolerance": { "type": "number", "default": 4 } }
            }
        },
        {
            "name": "genome_check",
            "description": "Compare live stems to the captured Style Genome.",
            "inputSchema": none
        },
        {
            "name": "font_audit",
            "description": "Technical outline and spacing checks plus Style Genome design deviations.",
            "inputSchema": {
                "type": "object",
                "properties": { "min_gap": { "type": "number", "default": 0 } }
            }
        },
        {
            "name": "decision_record",
            "description": "Append a DesignDecision to the font lib for later learning.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": { "type": "string" },
                    "glyphs": { "type": "array", "items": { "type": "string" } },
                    "issue": { "type": "string" },
                    "observation": { "type": "string" },
                    "intervention": { "type": "string" },
                    "accepted": { "type": "boolean" },
                    "confidence": { "type": "number", "default": 0 }
                },
                "required": ["accepted"]
            }
        },
        {
            "name": "decision_list",
            "description": "List DesignDecisions stored on the active font.",
            "inputSchema": none
        }
    ]) {
        tools.extend(more);
    }
    Value::Array(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_IDS: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = TEMP_IDS.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("typefoundry-mcp-{tick}-{id}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn call(server: &mut Server, id: i64, tool: &str, arguments: Value) -> Value {
        let reply = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": { "name": tool, "arguments": arguments },
            }))
            .unwrap();
        reply["result"].clone()
    }

    fn payload(result: &Value) -> Value {
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn initialize_echoes_the_protocol_version() {
        let mut server = Server::new();
        let reply = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": "init-1",
                "method": "initialize",
                "params": { "protocolVersion": "2024-11-05", "capabilities": {} },
            }))
            .unwrap();
        assert_eq!(reply["id"], json!("init-1"));
        assert_eq!(reply["result"]["protocolVersion"], json!("2024-11-05"));
        assert!(reply["result"]["capabilities"]["tools"].is_object());
        assert_eq!(reply["result"]["serverInfo"]["name"], json!("typefoundry"));
        assert_eq!(reply["result"]["serverInfo"]["version"], json!("0.1.0"));
    }

    #[test]
    fn notifications_return_nothing() {
        let mut server = Server::new();
        let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(server.handle_message(&initialized).is_none());
        let unknown = json!({ "jsonrpc": "2.0", "method": "notifications/cancelled" });
        assert!(server.handle_message(&unknown).is_none());
        let ping = server
            .handle_message(&json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" }))
            .unwrap();
        assert_eq!(ping["result"], json!({}));
    }

    #[test]
    fn tools_list_includes_point_move() {
        let mut server = Server::new();
        let reply = server
            .handle_message(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
            .unwrap();
        let names: Vec<&str> = reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"point_move"));
        assert!(names.contains(&"points_set"));
        assert!(names.contains(&"path_offset"));
        assert!(names.contains(&"kern_add"));
        assert!(names.contains(&"font_proof"));
        assert!(names.contains(&"font_measure"));
        assert!(names.contains(&"genome_capture"));
        assert!(names.contains(&"font_audit"));
        assert!(names.contains(&"decision_record"));
        assert_eq!(names.len(), 44);
        assert!(names.contains(&"style_derive"));
        assert!(!names.iter().any(|name| name.contains("prompt")));
    }

    #[test]
    fn point_move_changes_the_open_font_and_save_reuses_the_path() {
        let dir = temp_dir();
        let path = dir.join("Tiny.ufo").to_string_lossy().replace('\\', "/");
        let mut server = Server::new();

        let created = call(&mut server, 1, "font_create", json!({ "name": "Tiny" }));
        assert_eq!(created["isError"], json!(false));
        let glyph = json!({
            "name": "H", "unicode": 72, "advance": 500,
            "contours": [{ "closed": true, "points": [
                { "x": 0, "y": 0, "kind": "on", "smooth": false },
                { "x": 100, "y": 0, "kind": "on", "smooth": false },
                { "x": 100, "y": 700, "kind": "on", "smooth": false }
            ]}]
        });
        // There is no put_glyph tool. Seed the font through the same session.
        assert!(
            server
                .session
                .execute_line(&json!({ "op": "put_glyph", "glyph": glyph }).to_string())
                .unwrap()
                .ok
        );

        let unsaved = call(&mut server, 2, "font_save", json!({}));
        assert_eq!(unsaved["isError"], json!(true));
        assert_eq!(
            call(&mut server, 3, "font_save", json!({ "path": path }))["isError"],
            json!(false)
        );

        let moved = call(
            &mut server,
            4,
            "point_move",
            json!({ "name": "H", "contour": 0, "point": 1, "x": 140, "y": -10 }),
        );
        assert_eq!(moved["isError"], json!(false), "{moved}");
        let point = &server.session().font().unwrap().glyphs[0].contours[0].points[1];
        assert_eq!((point.x, point.y), (140.0, -10.0));

        let refused = call(&mut server, 5, "font_save", json!({}));
        assert_eq!(refused["isError"], json!(true));
        assert!(
            payload(&refused)["error"]
                .as_str()
                .unwrap()
                .contains("force"),
            "{refused}"
        );
        assert_eq!(
            call(&mut server, 6, "font_save", json!({ "force": true }))["isError"],
            json!(false)
        );
        let reopened = call(&mut server, 7, "font_open", json!({ "path": path }));
        assert_eq!(reopened["isError"], json!(false));
        let glyph = payload(&call(&mut server, 8, "glyph_get", json!({ "name": "H" })));
        assert_eq!(glyph["data"]["contours"][0]["points"][1]["x"], json!(140.0));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bad_path_sets_is_error() {
        let mut server = Server::new();
        let result = call(
            &mut server,
            1,
            "font_open",
            json!({ "path": "no/such/Font.ufo" }),
        );
        assert_eq!(result["isError"], json!(true));
        assert_eq!(result["content"][0]["type"], json!("text"));
        assert_eq!(payload(&result)["ok"], json!(false));

        let missing_args = call(&mut server, 2, "point_move", json!({ "name": "H" }));
        assert_eq!(missing_args["isError"], json!(true));

        let unknown = server
            .handle_message(&json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "font_generate", "arguments": {} },
            }))
            .unwrap();
        assert_eq!(unknown["error"]["code"], json!(INVALID_PARAMS));
    }

    #[test]
    fn serve_writes_only_protocol_lines_to_output() {
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"font_open\",\"arguments\":{\"path\":\"missing.ufo\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"nope\"}\n",
            "{not json\n",
        );
        let mut output = Vec::new();
        let mut log = Vec::new();
        serve(input.as_bytes(), &mut output, &mut log).unwrap();

        let output = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 4, "{output}");
        for line in &lines {
            let message: Value = serde_json::from_str(line).unwrap();
            assert_eq!(message["jsonrpc"], json!("2.0"));
        }
        assert!(!output.contains("MCP server"));
        let log = String::from_utf8(log).unwrap();
        assert!(log.contains("typefoundry MCP server"));
        assert!(log.contains("unknown method nope"));
    }

    #[test]
    fn save_paths_are_kept_per_font() {
        let dir = temp_dir();
        let regular = dir.join("Wide.json").to_string_lossy().replace('\\', "/");
        let mut server = Server::new();
        call(&mut server, 1, "font_create", json!({ "name": "Wide" }));
        assert_eq!(
            call(&mut server, 2, "font_save", json!({ "path": regular }))["isError"],
            json!(false)
        );
        let derived = call(
            &mut server,
            3,
            "style_derive",
            json!({ "style": "Italic", "slant": 12 }),
        );
        assert_eq!(derived["isError"], json!(false), "{derived}");
        // The italic has never been saved, so a bare save must not reuse the regular's file.
        let bare = call(&mut server, 4, "font_save", json!({}));
        assert_eq!(bare["isError"], json!(true));

        let fonts = payload(&call(&mut server, 5, "font_list", json!({})));
        let regular_id = fonts["data"]["fonts"][0]["id"].clone();
        call(&mut server, 6, "font_select", json!({ "id": regular_id }));
        assert_eq!(
            call(&mut server, 7, "font_save", json!({ "force": true }))["isError"],
            json!(false)
        );
        let check = payload(&call(&mut server, 8, "family_check", json!({})));
        assert_eq!(check["data"]["styles"], json!(["Regular", "Italic"]));

        let _ = fs::remove_dir_all(dir);
    }
}
