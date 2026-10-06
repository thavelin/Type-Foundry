# Type Foundry

A local type-design kit. The same command API drives the font model, plugins, and AI. The first working piece is blending two compatible fonts into a new one.

This is not a fork of [Shift](https://github.com/shift-editor/shift). Shift is the reference for a modern editor: a Rust font core, with import and export around it. Scripting and an AI API are still on Shift's future list. Those are the parts this project builds first.

## Today

```text
foundry new --name "Wide" --upm 1000 --out wide.json
foundry check narrow.json wide.json
foundry blend narrow.json wide.json --t 0.5 --out mid.json
foundry blend Narrow.ufo Wide.ufo --t 0.5 --out Mid.ufo
foundry run --file commands.jsonl
foundry mcp
```

A `save` command whose path ends in `.ttf` writes an installable TrueType file from the open font. `open`, `check`, and `blend` also read `.ttf`, `.otf`, `.woff` (WOFF 1), and the first face of a `.ttc` or `.otc`. See `documents/font-formats.md`.

```json
{"op":"save","path":"Mid.ttf"}
```

`foundry run` reads one JSON command per line. That stream is the plugin and AI surface. See `documents/api.md`.

`typefoundry` is the drawing window. Open a JSON font, a UFO, a typeface JSON, a web font, or a folder of SVG glyphs named `0041.svg`, pick a glyph, and edit points or drag a rectangle or an oval onto it. The preview pane sets headlines and paragraphs in the font. Every edit is a session command. See `Design/README.md`.

`Fonts/Roboto-English.json` and `Fonts/Roboto-Cyrillic.json` are Roboto Regular cut to English (U+0020–U+007E) and Cyrillic (U+0400–U+04FF). The license is in `Fonts/NOTICE.md`.

`foundry-mcp`, or `foundry mcp`, is a stdio MCP server, so a chat client can open, inspect, move points, check, blend, and save. It works only on local files and uploads nothing.

Blend is linear interpolation. Both fonts need the same glyph names, contour counts, point counts, and point types. That is the same rule variable-font masters use. Two unrelated typefaces will be refused until a later matching step exists. `open`, `save`, `check`, and `blend` accept a `.ufo` directory as well as the JSON working file. `open` also reads typeface JSON and web fonts (`.ttf`, `.otf`, `.woff`, and webfontjson). WOFF2 is refused. `save` also writes `.ttf`.

## Layout

- `App/` — Rust workspace. `foundry-core` holds the font, `foundry-api` runs commands, `foundry-cli` is the `foundry` binary, `foundry-app` is the `typefoundry` window, `foundry-mcp` is the MCP server.
- `documents/api.md` — the command contract.
- `Design/` — the window and its chrome.

From `App/`:

```text
cargo run -p foundry-cli -- check a.json b.json
cargo run -p foundry-app --release
cargo build --release -p foundry-mcp
powershell -ExecutionPolicy Bypass -File scripts/check.ps1
```
