# Command API

`foundry run` reads JSON commands, one per line, from a file or from stdin. Blank lines and lines that start with `#` are skipped. A leading byte-order mark is ignored. Each command prints one JSON response. In JSON, write Windows paths with forward slashes or escaped backslashes.

A plugin or an agent uses this stream. There is no second mutation API.

```text
foundry run --file commands.jsonl
```

## Response

```json
{"ok":true,"data":{"name":"Wide","upm":1000,"glyphs":[]}}
{"ok":false,"error":"glyph H is missing from Narrow","data":{"issues":[]}}
```

`ok` is false when the command did not do what was asked. Blend and check failures include `data.issues`.

## Commands

`create` starts an empty font in the session. `upm` defaults to 1000 and must be from 16 to 16384.

```json
{"op":"create","name":"Wide","upm":1000}
```

`open` also reads a folder of SVG glyphs named with four hex digits (`0041.svg` for A) and lines them up on a baseline and x-height. `open` and `save` read and write a project JSON file or a `.ufo` directory. `save` also writes an installable `.ttf` when the path ends in `.ttf`. `open` also reads a Three.js typeface JSON file (the json-fonts shape) and a webfontjson file (`callback({"css":"@font-face{...data:...}"})`). A webfontjson file may embed a `.ttf`, `.otf`, or WOFF 1 font. Those imports become a Type Foundry font in the session. A file with several `@font-face` rules imports the regular face. Embedded OpenType is refused. WOFF2 is unpacked like WOFF 1. Saving over `.otf`, `.woff`, or `.woff2` is refused. `check` and `blend` compare or blend JSON and UFO. JSON stays the working document. A UFO import keeps the default layer, sorts glyph names, and keeps the first Unicode value. It ignores anchors and guidelines. Kerning groups, kerning pairs, ligatures, and `features.fea` are kept. A save over an existing UFO updates fontinfo and the default layer and leaves lib data, images, and other layers in place. A component is flattened into its glyph on import, so the outline is kept but the reference is not. It refuses a glyph with an image, and it refuses a qcurve that is not exactly one off-curve point. One off-curve point is a quadratic. Two are a cubic. On the way out, the UFO family name is the font name. TrueType export turns cubics into quadratics, closes an open contour with a straight edge, and refuses a Unicode value outside the Basic Multilingual Plane.

`open`, `check`, and `blend` also read binary fonts: `.ttf`, `.otf` (CFF or CFF2 outlines), and the first face of a `.ttc` or `.otc` collection. The import keeps glyph order, names from `post` or the CFF charset, the lowest Unicode value per glyph, advances, units per em, and the vertical metrics. Composite glyphs are decomposed. TrueType implied on-curve points become real on-curve points. A variable font gives its default instance. Horizontal kern pairs and GSUB ligatures are imported. Hinting, smooth flags, and other faces in a collection are not imported. A glyph without a stored name is called `uniXXXX`, or `glyphNNNNN` when it has no Unicode. `.woff` (WOFF 1) and `.woff2` (WOFF2, decoded with the pure-Rust `wuff` crate) are unpacked into an sfnt and then read the same way. A damaged WOFF2 is named in the error. Saving to `.otf`, `.ttc`, `.otc`, `.woff`, or `.woff2` is refused, so nothing writes JSON under a binary extension. TrueType export now stores glyph names (`post` format 2), so a saved `.ttf` opens with the same names. See `documents/font-formats.md`.

`save`, `blend`, `save_family`, `export_family`, and `new` refuse to replace an existing file unless `force` is true. With `force`, the previous file is copied beside it as `name.bak` (or `name.bak2` when that already exists) and then replaced. Saving a `.ufo` does not delete `features.fea`, lib data, images, or other layers. The drawing window saves with `force`, because Save is an explicit overwrite. A command or MCP save stays strict.

```json
{"op":"open","path":"wide.json"}
{"op":"save","path":"wide.ufo"}
{"op":"save","path":"wide.ttf","force":true}
{"op":"open","path":"C:/fonts/Crimson Pro Regular.ttf"}
{"op":"open","path":"C:/fonts/Loma-Bold.otf"}
```

`info` describes the open font. `glyphs` lists names. `glyph` returns one glyph.

```json
{"op":"info"}
{"op":"glyphs"}
{"op":"glyph","name":"H"}
```

`put_glyph` inserts or replaces a glyph. This is the seam for prompt and image generation: a generator emits glyph JSON, and this command stores it.

```json
{"op":"put_glyph","glyph":{"name":"H","unicode":72,"advance":700,"contours":[{"closed":true,"points":[{"x":100,"y":0,"kind":"on","smooth":false},{"x":240,"y":0,"kind":"on","smooth":false},{"x":240,"y":700,"kind":"on","smooth":false},{"x":100,"y":700,"kind":"on","smooth":false}]}]}}
```

`set_advance` changes the advance of an existing glyph.

```json
{"op":"set_advance","name":"H","advance":680}
```

`move_point` sets one point to an absolute coordinate. `contour` and `point` are zero-based indexes.

```json
{"op":"move_point","name":"H","contour":0,"point":0,"x":110,"y":20}
```

### Editing

These commands change the open font. Each one checks its input before it changes anything, so a failed edit leaves the font as it was. Points are addressed as `[contour, point]` with zero-based indexes.

```json
{"op":"move_points","name":"a","points":[[0,1],[0,2]],"dx":10,"dy":0}
{"op":"set_points","name":"a","points":[{"contour":0,"point":1,"x":10,"y":0},{"contour":0,"point":2,"x":10,"y":40}]}
{"op":"insert_point","name":"a","contour":0,"index":3,"x":120,"y":40,"kind":"on","smooth":false}
{"op":"split_segment","name":"a","contour":0,"point":4,"t":0.5}
{"op":"delete_points","name":"a","points":[[0,2]]}
{"op":"set_point","name":"a","contour":0,"point":1,"kind":"off","smooth":false}
{"op":"add_contour","name":"a","contour":{"closed":false,"points":[{"x":0,"y":0,"kind":"on","smooth":false}]}}
{"op":"set_closed","name":"a","contour":1,"closed":true}
{"op":"reverse_contour","name":"a","contour":0}
```

- `set_points` puts each listed point at an absolute `(x, y)`. It is one undo step. A missing point or a non-finite coordinate changes nothing. The window uses it for Align.
- `insert_point` puts a point before `index`. An `index` equal to the point count appends. `kind` defaults to `on`.
- `split_segment` cuts the segment that ends at on-curve point `point`, at `t` from 0 to 1. A line gains one point. A quadratic or cubic is split exactly, so the shape does not change. The response gives the new on-curve point's index.
- `delete_points` removes points. A contour left empty is removed.
- `set_point` changes `kind`, `smooth`, or both. Off-curve points are never smooth.
- `reverse_contour` keeps a closed contour's first point first.

```json
{"op":"delete_glyph","name":"a"}
{"op":"rename_glyph","name":"a","new_name":"a.alt"}
{"op":"set_unicode","name":"a.alt","unicode":null}
{"op":"rename_font","name":"Wide Display"}
{"op":"set_metrics","x_height":520,"cap_height":710}
```

`set_metrics` takes any of `ascender`, `descender`, `cap_height`, and `x_height`, and leaves the others alone.

`transform` applies an affine matrix `[a, b, c, d, e, f]`, so `x' = a*x + c*y + e` and `y' = b*x + d*y + f`. Without `names` it changes every glyph. `points` limits it to a selection in one glyph. `anchor` is where the matrix is centered, worked out per glyph: `origin` (the default, font 0,0), `center` (the center of the points), or `advance` (half the advance across, centered vertically on the points). With `advance: true`, each advance is scaled by the matrix's horizontal scale.

```json
{"op":"transform","matrix":[1,0,0.2126,1,0,0]}
{"op":"transform","names":["a"],"matrix":[-1,0,0,1,0,0],"anchor":"advance"}
{"op":"transform","names":["a"],"points":[[0,1]],"matrix":[1,0,0,1,0,-10]}
{"op":"round_coordinates","names":["a"]}
```

The first line slants every glyph 12 degrees, the way the window's Slant effect does. `round_coordinates` rounds points and advances to whole units, for the named glyphs or all of them.

`index` lists every glyph's name, Unicode, group, advance, contour count, and point count in one response. `group` is one of `Uppercase`, `Lowercase`, `Figures`, `Punctuation`, `Symbols`, `Spaces`, `Marks`, `Ligatures`, or `Unencoded`. A ligature listed in the kerning block is `Ligatures`. A glyph with no Unicode value is `Unencoded`. The group is worked out from the name and Unicode value on each read and is not stored in the font.

### Undo

```json
{"op":"undo"}
{"op":"redo"}
{"op":"checkpoint"}
{"op":"history"}
```

Every edit above, plus `put_glyph`, `set_advance`, and `move_point`, can be undone. The session keeps 200 steps. Consecutive moves of the same points, consecutive `set_advance` on one glyph, and consecutive `set_metrics` share one step, so a drag or a slider undoes in one go. `checkpoint` ends that run. `create`, `open`, and `blend` start a fresh history. `history` returns `{"undo":n,"redo":n}`.

`check` compares two files. `blend` writes a new file and opens it in the session. `t` defaults to 0.5. `t` is 0 at the first font and 1 at the second. Values outside that range extrapolate.

```json
{"op":"check","a":"narrow.json","b":"wide.json"}
{"op":"blend","a":"narrow.json","b":"wide.json","t":0.5,"out":"mid.json"}
```

## Several fonts and families

A session can hold many open fonts. `create`, `open`, `blend`, `derive_style`, and `open_family` each add a font and make it active. Every other command acts on the active font, so a script that opens one font works as before. Each font keeps its own undo history and its own unsaved-changes flag. `create`, `open`, and `info` return the font's `id`.

```json
{"op":"fonts"}
{"op":"select_font","id":2}
{"op":"close_font"}
{"op":"close_font","id":3}
{"op":"glyph","name":"R","font":1}
{"op":"index","font":1}
```

`fonts` lists `id`, `name`, `family`, `style`, `weight`, `italic`, glyph count, `active`, and `dirty` for each open font. `close_font` closes the active font, or `id`, and the next one becomes active. Unsaved changes are lost. `glyph` and `index` can read any open font with `font`.

### Styles

Every font has a style: a family name, a style name, a weight from 1 to 1000 (400 is Regular, 700 is Bold), an italic flag, an italic angle in degrees counter-clockwise, so a right-leaning italic is negative, such as -12, and an OS/2 width class from 1 (ultra-condensed) to 9 (ultra-expanded). 5 is normal. A file saved before styles existed opens as the Regular of a family named after the font. Dots in the family or style name are kept, so `v4.5` stays `v4.5` in the file name and the PostScript name.

```json
{"op":"set_style","family":"Wide","style":"Bold","weight":700}
{"op":"set_style","italic":true,"italic_angle":-12}
{"op":"set_style","width":3}
{"op":"derive_style","style":"Italic","slant":12}
{"op":"derive_style","style":"Bold","weight":700}
{"op":"slant","degrees":12}
```

`set_style` can be undone. Changing the family or style name renames the font to `Family Style`. `width` is written to UFO `openTypeOS2WidthClass` and to the TrueType OS/2 width class. `derive_style` copies the active font as a new style of the same family and opens the copy. A nonzero `slant` leans every glyph that many degrees about the baseline, then shifts it so the ink stays centred in its advance, sets `italic`, and sets the italic angle to match. `slant` does the same lean on the open font.

`info` returns the style, the vertical metrics, name-table fields, coverage (`glyphs` and `encoded`), kerning counts, and the glyph names.

### Families

A family is every open font that shares the active font's family name. The family commands work on that set, in the order the fonts were opened, or on `ids` when given.

```json
{"op":"family_check"}
{"op":"save_family","path":"C:/fonts/Wide/Wide.family.json"}
{"op":"open_family","path":"C:/fonts/Wide/Wide.family.json"}
{"op":"export_family","dir":"C:/fonts/Wide/ttf","format":"ttf"}
```

- `family_check` reports `issues` and whether the family is `ready`. Blocking issues are styles with different family names, or two styles that would export to the same file name. Notes cover two styles with the same weight and italic, different units per em, different ascender or descender, italic with an angle of 0, glyphs missing from some styles, and glyphs whose Unicode differs between styles.
- `save_family` writes each style as `Family-Style.json` beside the family file, then the family file itself: `format` `typefoundry.family`, `version` 1, `family`, and `styles`, a list of file names relative to the family file.
- `open_family` opens every style the family file lists. A style may be any format `open` reads.
- `export_family` writes each style into `dir` as `Family-Style.ttf`, `.ufo`, or `.json`. It refuses before writing anything when the check finds a blocking issue, and it refuses before writing anything when a destination already exists and `force` is not set.
- `copy_family` copies a family file and its styles into `dir`. `family` renames every style. `label` is stored on the new family file, so making a version folder from the previous one is one command.
- `set_sidebearing` with `family` true sets that sidebearing on every open style. Each style gets its own undo step. Left shifts the outline and keeps the advance. Right changes the advance.

```json
{"op":"set_sidebearing","name":"f","side":"right","value":32,"family":true}
{"op":"copy_family","path":"C:/fonts/v4.5/Vostok.family.json","dir":"C:/fonts/v4.6","label":"v4.6"}
```

Exports carry the style so apps group the files as one family. In TrueType, name IDs 16 and 17 hold the family and style, and IDs 1 and 2 hold the four-style grouping older apps use: Regular, Italic, Bold, and Bold Italic share the family name, and any other weight becomes its own legacy family, such as `Wide Light`. OS/2 has the weight class and the italic, bold, and regular bits, `head` has the matching style bits, `post` has the italic angle, and `hhea` slopes the caret. UFO export writes `familyName`, `styleName`, `styleMapFamilyName`, `styleMapStyleName`, `openTypeOS2WeightClass`, and `italicAngle`. TrueType, OpenType, and UFO imports read the same fields back.

## Metadata, kerning, and weight

`set_info` sets the name-table fields. An omitted field stays as it is. An empty vendor id exports as four spaces. An empty unique id exports as the PostScript name plus the version, with no foundry prefix. A vendor id that is set must be four ASCII characters. Two glyphs may not share a Unicode value: the command fails and names both glyphs.

```json
{"op":"set_info","copyright":"Copyright 2026 Troy Havelin","designer":"Troy Havelin","version":"4.500","vendor":"VSTK","unique_id":"Vostok Serif v4.5"}
```

Kerning is stored on the font. `None` in the file means a UFO save leaves kerning and `features.fea` that are already on disk. An object, even an empty one, replaces them. A group name starting with `public.kern1.` is a left-side group and `public.kern2.` is a right side. A glyph may be in only one group on a side. TrueType `kern` format 0 stores pairs, so groups are expanded on export. Ligatures become a `liga` feature in the UFO and a GSUB lookup in the TTF.

```json
{"op":"set_group","name":"public.kern1.round","members":["o","c","e"]}
{"op":"add_kern","left":"public.kern1.round","right":"T","value":-30}
{"op":"add_ligature","glyphs":["f","i"],"name":"fi"}
{"op":"set_features","text":"feature liga {\nsub f i by fi;\n} liga;\n"}
```

`offset` (MCP `path_offset`) thickens or thins outlines. Default `mode` is `points`: point count and structure stay identical for blending. Contour nesting classifies outers vs holes; `counter_share` (default 0.85) biases growth into counters; `min_gap_ratio` / `gap_window` keep facing slits open; `outline_valid` retries with amounts ×0.93 and falls back per glyph. `zones` / `keep_metrics` pin baseline, x-height, cap, and glyph extremes. `italic:auto` works in the de-slanted frame. `mode:clean` is a geometric offset (not blend-compatible). `preview` and `family` follow the usual rules. Full field list: `documents/geometry-tools-spec.md`.

`smooth_outlines` / `check_smoothness` (MCP `outline_smooth` / `smoothness_check`) remove or measure tangent breaks while keeping structure. `mirror`, `symmetrize`, `check_symmetry`, and `glyph_from_mirror` cover symmetry (MCP `glyph_mirror`, `glyph_symmetrize`, `symmetry_check`, `glyph_from_mirror`).

`scale_width` changes width and keeps vertical stem thickness (ray-cast stems). Counters and sidebearings scale; advances round. `factor` must be greater than 0 and at most 4. `slant` shears about `pivot_y` (default `x_height/2`) and does not re-centre ink unless `recenter:true`.

```json
{"op":"offset","horizontal":18,"vertical":6,"gap":8,"corner":"miter","sidebearing":true}
{"op":"offset","names":["H"],"horizontal":12,"preview":true}
{"op":"smooth_outlines","names":["zero","six"]}
{"op":"check_smoothness","details":true}
{"op":"mirror","names":["parenleft"],"axis":"vertical"}
{"op":"glyph_from_mirror","preset":"brackets","replace":true,"family":true}
{"op":"stroke","kind":"outline","horizontal":16,"vertical":8}
{"op":"scale_width","factor":0.8}
{"op":"slant","degrees":10,"pivot_y":250}
```

`check_outlines` uses flattened-outline validity (self-intersections, contour crosses, inverted area) plus kinks, nesting-based direction, round-overshoot, and floating glyphs. `check_spacing` measures in the italic frame on flattened outlines and reports gap sizes. `diff` also reports kerning, features, and info changes.

```json
{"op":"check_outlines"}
{"op":"check_spacing","min_gap":8,"pairs":[["f",")"],["f","T"]]}
{"op":"diff","a":"v4.5.json","b":"v4.6.json"}
{"op":"proof","path":"proof.png","text":"Hamburgefonts","pixel_size":72}
{"op":"move_glyph","name":"fi","index":3}
```

### Style Genome and Foundry Audit

Fonts keep an open `lib` map for foundry data. Keys we use today:

- `com.typefoundry.styleGenome` — captured Style Genome
- `com.typefoundry.designDecisions` — array of DesignDecision records

`measure` returns live vertical metrics plus per-glyph advance, sidebearings, and vertical stem widths (same edge clustering as `scale_width`). `capture_genome` writes a genome from sample glyphs (`H`, `I`, `N`, `P`, `R`, `h`, `n`, `u` when present): median primary stem, median sidebearing, and metrics. `stem_tolerance` defaults to 4 units. `get_genome` returns the stored genome or `null`. `check_genome` compares live stems on those samples to the stored genome. `audit` returns technical outline and spacing issues plus design genome issues. `critique` turns those signals into ranked suggestions with confidence and a proposed intervention (deterministic — no remote model). `resolve_critique` accepts or rejects one suggestion and appends a DesignDecision. `record_decision` / `list_decisions` remain the direct store. `capture_genome`, `record_decision`, and `resolve_critique` are undoable edits.

```json
{"op":"measure"}
{"op":"capture_genome","stem_tolerance":4}
{"op":"get_genome"}
{"op":"check_genome"}
{"op":"audit","min_gap":0}
{"op":"critique","min_gap":0}
{"op":"resolve_critique","id":"design:stem:H","accepted":true,"issue":"stem","observation":"…","intervention":"…","glyphs":["H"],"confidence":0.88,"layer":"design"}
{"op":"record_decision","scope":"Vostok Serif","glyphs":["e"],"issue":"counter_closure","observation":"counter too dense","intervention":"open aperture","accepted":true,"confidence":0.9}
{"op":"list_decisions"}
```

`proof` draws the open font to a PNG, or to a one-page vector PDF when the path ends in `.pdf`, with metric guides and advance boxes. The PDF keeps the curves and embeds no font. `compare` is a second font file drawn underneath. An empty `text` draws the encoded character set.

## Project file

`format` is `typefoundry.font` and `version` is `1`. Points are `on` or `off`. A cubic segment is two `off` points between `on` points. A quadratic segment is one `off` point. Contours are closed or open.

## Blend rules

Blending refuses the pair when any of these differ: units per em, glyph name set, unicode, contour count, closed flag, point count, point kind, or smooth flag. The result keeps the first font's glyph order. Advances, coordinates, and vertical metrics are interpolated.

## CLI

These commands call the same operations:

```text
foundry new --name "Wide" --upm 1000 --out wide.json
foundry info wide.json
foundry check narrow.json wide.json
foundry blend narrow.json wide.json --t 0.5 --out mid.json --force
foundry proof wide.json --out proof.png --text Hamburgefonts
foundry diff narrow.json wide.json
```

`check` and `blend` exit 1 when the fonts are not compatible. `new`, `blend`, and `proof` take `--force` when the output already exists. `foundry run` writes JSON with non-ASCII characters escaped as `\u`, so PowerShell 5.1 can redirect it. A missing field is named in the error, for example a point that has no `smooth`.

## MCP server

`foundry-mcp` is a stdio MCP server over one command session. `foundry mcp` runs the same server. A chat client can create, open, inspect, move a point, check, blend, and save. There are no generation tools.

Every tool runs on local files only. No tool uploads a font, an outline, or anything else, and the server makes no network calls.

Stdout carries only protocol messages, one JSON-RPC object per line. Logs go to stderr. The server answers `initialize` (it echoes the client's `protocolVersion`), `ping`, `tools/list`, and `tools/call`. A message without an `id` is a notification and gets no reply.

| Tool | Arguments | Command |
| --- | --- | --- |
| `font_create` | `name`, `upm?` | `create` |
| `font_open` | `path` | `open` |
| `font_save` | `path?`, `force?` | `save` |
| `font_info` | none | `info` |
| `font_glyphs` | none | `glyphs` |
| `glyph_get` | `name` | `glyph` |
| `point_move` | `name`, `contour`, `point`, `x`, `y` | `move_point` |
| `font_check` | `a`, `b` | `check` |
| `font_blend` | `a`, `b`, `t?`, `out`, `force?` | `blend` |
| `font_list` | none | `fonts` |
| `font_select` | `id` | `select_font` |
| `font_close` | `id?` | `close_font` |
| `style_set` | `family?`, `style?`, `weight?`, `italic?`, `italic_angle?`, `width?` | `set_style` |
| `style_derive` | `style`, `weight?`, `italic?`, `slant?` | `derive_style` |
| `points_set` | `name`, `points` | `set_points` |
| `family_check` | none | `family_check` |
| `family_open` | `path` | `open_family` |
| `family_save` | `path`, `force?` | `save_family` |
| `family_export` | `dir`, `format`, `force?` | `export_family` |
| `info_set` | name-table fields | `set_info` |
| `kern_set` | `kerning` | `set_kerning` |
| `kern_add` | `left`, `right`, `value` | `add_kern` |
| `group_set` | `name`, `members` | `set_group` |
| `ligature_add` | `glyphs`, `name` | `add_ligature` |
| `features_set` | `text?` | `set_features` |
| `path_offset` | offset fields (`mode`, `counter_share`, `min_gap_ratio`, `zones`, `italic`, `family`, `preview`, …) | `offset` |
| `path_stroke` | `kind`, plus the offset fields | `stroke` |
| `outline_smooth` | `reference?`, `names?`, `axis_snap?`, `family?`, `preview?`, … | `smooth_outlines` |
| `smoothness_check` | `reference?`, `threshold?`, `names?`, `details?` | `check_smoothness` |
| `glyph_mirror` | `names?`, `axis?`, `center?`, `family?`, `preview?` | `mirror` |
| `glyph_symmetrize` | `names?`, `source?`, `mode?`, `tolerance?`, `family?` | `symmetrize` |
| `symmetry_check` | `names?`, `pairs?`, `center?`, `tolerance?` | `check_symmetry` |
| `glyph_from_mirror` | `source?`, `target?`, `transform?`, `preset?`, `replace?`, `family?` | `glyph_from_mirror` |
| `sidebearing_set` | `name`, `side`, `value`, `family?` | `set_sidebearing` |
| `outline_check` | none | `check_outlines` |
| `spacing_check` | `min_gap?`, `pairs?` | `check_spacing` |
| `font_diff` | `a`, `b` | `diff` |
| `font_proof` | `path`, `text?`, `pixel_size?`, `guides?`, `boxes?`, `compare?`, `force?` | `proof` |
| `glyph_move` | `name`, `index` | `move_glyph` |
| `family_copy` | `path`, `dir`, `family?`, `label?`, `force?` | `copy_family` |
| `undo` | none | `undo` |
| `redo` | none | `redo` |
| `width_scale` | `factor`, `names?` | `scale_width` |
| `slant` | `degrees`, `pivot_y?`, `recenter?` | `slant` |
| `font_measure` | none | `measure` |
| `genome_capture` | `stem_tolerance?` | `capture_genome` |
| `genome_check` | none | `check_genome` |
| `font_audit` | `min_gap?` | `audit` |
| `decision_record` | `accepted`, plus optional `scope`, `glyphs`, `issue`, `observation`, `intervention`, `confidence` | `record_decision` |
| `decision_list` | none | `list_decisions` |
| `font_critique` | `min_gap?` | `critique` |
| `critique_resolve` | `id`, `accepted`, plus optional decision fields | `resolve_critique` |

A tool result is `{"content":[{"type":"text","text":"..."}],"isError":false}`. The text is the command response JSON. `isError` is true when the command response has `ok: false`, or when the arguments do not fit the tool.

`font_save` takes the same paths as `save`, including `.ttf`. Without a `path` it saves the active font to the last path that font was opened from, saved to, or blended to. A new or derived style has no path until it is saved once. The paths live in the MCP wrapper, not in the session.

Claude Desktop (`claude_desktop_config.json`) or Cursor (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "typefoundry": {
      "command": "/path/to/foundry-mcp",
      "args": []
    }
  }
}
```

Claude Code:

```text
claude mcp add typefoundry -- /path/to/foundry-mcp
```

Build it first, from `App/`: `cargo build --release -p foundry-mcp`. Pass tool paths with forward slashes, for example `{"path":"C:/fonts/Wide.ufo"}`.

## Collection analyzer (study schema v2)

`foundry analyze` reads fonts from disk independently of the editor session. It does
not open a tab, change the active font, add undo history, save a font, export a font,
or upload anything. Source outlines exist only transiently in memory. Reports and
cache files contain measurements; optional PDF sheets contain a raster image,
not reusable vector outlines. Existing differing outputs are refused. Identical
outputs are accepted, so rerunning the same analysis is idempotent.

```text
foundry analyze /fonts --out /reports/study-01 --format md --cache-dir /reports/cache
foundry analyze A.ttf B.ufo --glyphs n,o,H,O --format csv
foundry analyze Variable.ttf --axes wght=600,wdth=90
foundry analyze /fonts --weight-match 0.16 --out /reports/matched --sheets png,pdf
```

`--out` is a **directory**, not a filename. It receives `collection.json`, one
`font-<source-id>.json` per analyzed face/location, and `report.json`, `report.csv`,
or `report.md`. JSON per-font and collection outputs are always written, regardless
of the selected convenience format. Without `--out`, the selected format goes to
stdout. A partial success still produces its measurements and errors, and the CLI
exits 1 if any inputs failed (0 for a complete scan, 2 for CLI syntax errors).

Paths can be individual `.ttf`, `.otf`, `.ttc`, `.otc`, WOFF1 `.woff`, `.ufo`, and
JSON files supported by `open`, or recursive directories. Collections analyze all
faces. UFOs and recognized SVG glyph folders are atomic inputs. Symlinks, `.git`,
and `target` folders are not followed. WOFF2 and EOT are refused by the analyzer,
including WOFF2 binary data disguised as TTF; this restriction does not change
`open`'s existing WOFF2 support. Bad paths and corrupt fonts are reported and the
remaining files continue. Binary reads share the engine's existing importer.

Variable fonts produce the default location plus each `fvar` named instance.
`--axes` replaces that list with one location; unspecified axes retain defaults.
Unknown axes, out-of-bounds coordinates and nonfinite values are errors.
`--weight-match` selects the default/named instance closest in measured n stem /
x-height, independently per collection face. Ties break by instance name. It is
mutually exclusive with `--axes`. Static faces retain their actual weight and
receive an explicit warning: changing their stem/x-height would require editing
outlines. This option never synthesizes a new weight or claims a static face has
been matched. Named coordinates and the selected instance are included in `source`.

`--glyphs` is a comma-separated list of source names, literal characters, or
`U+NNNN` tokens. It restricts per-glyph tables. The diagnostic reference glyphs
remain available for font-level measurements/classification; excluded width-table
entries are null. Unicode coverage and total glyph counts always describe the
whole font. Use `--cache-dir` to opt into persistent measurement caching. Cache keys
are SHA-256 over source content, measurement options, schema and algorithm version;
UFO/SVG packages hash sorted relative filenames and contents. Output locations and
format are excluded from keys. No timestamps or cache-hit flags enter results.
Corrupt cache entries are ignored; differing files are never overwritten.

### Run operations and MCP

All four operations use the same `foundry-core::analyze` implementation through
`foundry-api::Session::execute`; they need no open font and preserve session state.

```json
{"op":"analyze","paths":["/fonts"],"options":{"glyphs":["n","o","H","O"],"output":"/reports/study-01","format":"md","cache_dir":"/reports/cache","sheets":["png","pdf"]}}
{"op":"classify_font","path":"/fonts/Example.ttf"}
{"op":"compare_to_class","path":"/fonts/Target.ttf","class":"geometric sans","collection":["/reference-fonts"],"options":{}}
{"op":"nearest_fonts","path":"/fonts/Target.ttf","collection":["/reference-fonts"],"options":{}}
```

MCP names are `analyze_fonts(paths, options)`, `classify_font(path)`,
`compare_to_class(path, class, collection, options)`, and
`nearest_fonts(path, collection, options)`. Reference paths are required for the
last two because there is no hidden global collection or mutable class registry.
These tools have read-only, nondestructive, idempotent, local-only annotations.
`analyze_fonts` can write explicitly requested reports/cache; read-only refers to
font sources and editor state. Comparison/nearest operations ignore `options.output`.
Per-file failures appear in `errors`; a successful response can be partial. An
invalid request or absent comparison class yields the normal command error.

### Schemas and provenance

Per font: `format: "typefoundry.study"`, `version: 2`,
`algorithm: "outline-analyzer-0.1.0"`. Version 1 is deliberately not reused: an
independent letterform-study workflow is being developed in the main checkout.
This analyzer does not feed its v2 files to that workflow. The companion
[typefoundry.study.schema.json](typefoundry.study.schema.json) defines the serialized
contract. The aggregate uses `format: "typefoundry.collection-study"`, version 2.

Every numeric metric contains `value` (number or null), `units`, `method`, `glyphs`,
`reason` (string for unavailable values, otherwise null), `x_height_ratio`, and
`cap_height_ratio`. Lengths retain font units and both height normalizations;
dimensionless values have units `ratio`. Angles are degrees, areas are squared font
units, counts are `count`. Zero remains a measured zero; missing glyphs, empty
outlines, undefined divisions and ambiguous geometry return null with a reason.
No metrics use rendered pixels, hinting or optical raster comparisons.

Metadata includes family/style, weight/width classes, italic angle, UPM, declared
vertical metrics, raw OS/2/hhea metrics and line gaps, glyph count, complete cmap
coverage by Unicode 17.0 block, GSUB/GPOS feature tags, license, and RFN detection.
RFN detection searches the license string; it is not a legal determination, and a
missing license gives null. Source fields that do not exist are null. GPOS counts
are stored PairPos records and class matrix dimensions (including class zero),
with extension lookups resolved; contextual applicability and expanded class-pair
coverage are not implied. The legacy pair count and nonzero class cells are also
reported. Document feature tags are extracted from feature declarations.

### Measurement methods and limits

- Bounds and ray crossings solve actual line/quadratic/cubic Beziers using `kurbo`.
  Bounds use analytic extrema, not control-point hulls. Stem values are the median
  minimum filled horizontal span at 25/40/60% glyph height. H/e bars and o top use
  the minimum filled span on the central vertical ray. Contrast requires an o
  counter. Stress samples 180 radial directions; monoline o stress is undefined.
- Width tables cover a-z, A-Z and 0-9, including advances, exact ink widths,
  n/H-relative widths and bearings; every selected source glyph gets darkness,
  bearings, point count and quality measurements. Overshoots on o/O/s/a are signed
  exact extrema relative to x-height/cap-height and the baseline.
- The n arch profile samples the top filled interval at 10/20/50/90% across the
  counter gap defined at 30% height. Springing height and minimum arch thickness
  use 99 interior samples. Inner radii use analytic curvature sampled 99 times per
  segment; n uses concave segments of its open counter. Counter rise is explicitly
  defined in each method field; o/O currently report the distance below the counter
  top at the indicated inward offset. Superellipse fitting is axis-aligned least
  squares, p=0.50..8.00 in 0.01 increments. Handle fractions summarize first handles on the inner contour; n uses its
  concave open-counter segments.
- Terminal cut angles are short, corner-bounded straight segments in the expected
  opening region, with the rightmost candidate selected. The `terminals` map also
  lists candidate angles and a low heuristic confidence. Straight cuts classify as
  horizontal, vertical, or angled. Ball, taper, serif and spur terminal segmentation
  remains ambiguous and returns null rather than an invented label.
- Join/waist thinning is a central-band minimum/median horizontal-thickness proxy,
  not a skeleton-based anatomical join measurement. Foot extensions on n/H/I detect
  serifs; foot thickness and local curved/sloping segments distinguish bracketed,
  slab, hairline and wedge heuristically.
- Areas use exact signed Bezier integrals. Detected overlapping contours use adaptive
  integration of analytic nonzero-winding spans (relative tolerance 1e-8, recursion
  depth 16). Overlap flags use the engine's 12-step polygon crossing check plus
  same-winding nested-contour detection, so tiny intersections/tangencies can be
  missed. Darkness is ink area / advance and relative to n/H. Texture greyness is
  total area / (unshaped advances * declared ascender-minus-descender) for the fixed
  pangram plus digits; no shaping, kerning or hinting is applied.
- Quality reports >1-degree tangent breaks at source smooth points, near-zero Bezier
  segments (<1e-6 UPM), point counts, interior extrema needing explicit split points,
  clockwise/counterclockwise contour counts and approximate overlap flags. Binary
  import cannot preserve authoring smooth flags, so its kink count does not certify
  smoothness. Short segments include curves, using integrated arclength.

Classification is a deterministic, transparent ruleset, not a trained classifier.
It emits one of geometric sans, grotesque, humanist sans, serif text, slab, mono,
display or script, with three reasons and an **uncalibrated** confidence. Mono uses
equal ASCII alphanumeric advances; other classes use roundness, modulation/stress,
foot extensions and extreme proportions. Script requires declared cursive attachment
and a slanted design; it is conservative. Missing evidence lowers confidence.
Real designs that mix categories can be misclassified; inspect reasons and specimen.

### Collection calculations

Each style group reports min/median/max and the families/locations setting extrema.
Font-unit lengths are divided by each font's x-height before cross-font comparison;
ratios are unchanged. Nearest-family distance is RMS feature differences divided by
fixed scales: x/cap 0.15, cap/UPM 0.15, stem/x 0.08, contrast 1.0, o-width/x 0.25,
serif extension 0.4, n/H advance ratio 0.2. At least four shared features are required;
results exclude the target's own family and deduplicate other families, keeping five.
The three largest standardized differences accompany each match.

Outliers use a leave-one-family-out observed class range with at least three peer
faces; a font cannot define its own acceptable limits. A comparison target is not
added to its reference collection. Comparisons list in-range and out-of-range
metrics sorted by absolute deviation divided by max(class span, 5% median, 0.0001).
Possible universals require at least two classes, complete measurements on all faces,
and total range <=2% of max(abs(median), 0.01). They describe this sample, not a
universal law of type design; tiny samples should not be treated as normative.

### Optional sheets and validation

`--sheets png,pdf` requires `--out`. Each selected face/location gets a specimen,
n/o and H/O baseline overlays, blue-to-red horizontal thickness sampling, nHn/oHo/
HHH/OHO spacing strips, a greyness strip, and o/O inner-curvature combs. These
visuals are rasterized from transient outlines **after** measurement; they do not
supply any numeric metric. Missing specimen glyphs are skipped; reports record the
missing numeric evidence. PDFs embed pixels only. Source hashes are rechecked
before rendering to catch source changes during a scan.

Fixtures are original procedural shapes; no third-party outlines are included.
Tests cover geometric/humanist/serif/mono classes, a real gvar variable stem,
collections, WOFF1, UFO, Bezier bounds/area, overlap integration, UPM invariance,
missing glyphs, Unicode blocks, option parsing, source/session preservation,
MCP dispatch, corrupt-file continuation and identical cached reruns. Rebuild the
small binary fixtures with `tests/fixtures/generate_variable.py` (fontTools is needed
only to regenerate them). Sources for format/data definitions:
[WOFF 1.0](https://www.w3.org/TR/WOFF/#WOFFHeader),
[Unicode 17 Blocks](https://www.unicode.org/Public/17.0.0/ucd/Blocks.txt).

## Font Atlas: a persistent design knowledge base

The next layer consumes measurement reports, not outlines. It groups faces into
families, builds an interpretable measured feature space, discovers neighborhoods,
and proposes supported gaps as briefs for **original** construction. It does not
reconstruct another font, train a generative model, or make originality claims.

### Scan, resume, and build

```text
python tools/scan_font_library.py --foundry /path/to/foundry --store /local/font-worker --roots /fonts --jobs 3 --atlas-out /reports/font-atlas
foundry atlas /local/font-worker/studies /local/font-worker/sources.json --out /reports/font-atlas
foundry atlas-query /reports/font-atlas/latest.json --family "Vostok Serif v3"
foundry atlas-briefs /reports/font-atlas/latest.json --class "serif text" --limit 5
foundry atlas-evaluate original.ttf --atlas /reports/font-atlas/latest.json --brief GAP_ID
```

The worker is Python standard-library orchestration around the Rust CLI, with no
Python font parser or embedding implementation. It hashes source contents, stores
one measurement object per content/profile/engine identity, and retains every
source path as an alias in an indexed SQLite worker database. Byte-identical copies
are measured once per worker. Collection faces and variable locations stay distinct.
The atlas then deduplicates across workers by source SHA-256, collection face and
axis coordinates; a default and named instance at identical coordinates count once.
Where full/bulk objects coexist, the object with more available numeric metrics wins.

Worker persistence starts at `tools/migrations/001_font_library.sql`. Keep worker
SQLite on a local disk, rather than SMB. Raw measurement JSON and inventories are
portable. File locks prevent overlapping scans of one store and release on process
exit. A scan resumes completed objects; `--retry-errors` retries recorded failures.
`--inventory FILE` accepts `{"paths":["/absolute/font.ttf",...]}` for indexed NAS
sources. `--timeout` limits one unique file; timeout is an explicit status, not a
successful empty measurement. `progress.json` and `sources.json` retain counts,
unsupported formats, exceptions and coverage. Discovery skips build/dependency/test
folders, symlinks, archive directories and do-not-index paths. WOFF2/EOT are recorded
as unsupported, never silently unpacked or copied to a different format.

Large libraries use `analyze --profile atlas` with diagnostic/requested outlines.
The importer still reads every name, advance, Unicode mapping and metadata field,
but does not decompose irrelevant non-ASCII glyphs. Full `open` remains unchanged.
The atlas profile defers quadratic overlap census and returns null for area/darkness
on glyphs with more than 24 contours or 600 points. Greyness becomes null if a required
pangram area is unavailable. Each deferred result records its reason and the profile
is included in metadata/cache identity. Ordinary area values retain the full method;
per-font area results are memoized. Use the full profile on selected designs when
performing detailed quality work.

### Families, classes and neighborhoods

Family identity uses a normalized **exact** family name (case/spacing normalized;
style/version words are not guessed away). It retains raw name aliases and source
locations. Coincident family names from unrelated producers can still need review;
there is no automatic foundry attribution or silent superfamily merge.

One upright, regular-width face near weight 400 represents each family while fitting
normalization and projecting the map. Coverage is part of representative selection;
italic angle and explicit Italic/Oblique style names receive a penalty. All styles
remain in the atlas with their own feature vectors and within-family ranges. A
family with 80 variable instances receives one fitting vote, like a one-style family.
Raw class distributions are retained. Labels below confidence 0.4, or faces with
fewer than 12 measured dimensions, are withheld as `unclassified` in the atlas.
Classifier confidence is uncalibrated; this is not an authoritative taxonomy.

The explorer exports reviewed labels with evidence:

```json
{"FAMILY_ID_OR_EXACT_NAME":{"class":"humanist sans","reason":"Reviewed open apertures and oblique stress in the specimen"}}
```

Rebuild with `--labels labels.json` (run option `labels`) to apply them. Corrections
have explicit provenance and do not mutate source fonts or silently edit a snapshot.
Classes are geometric sans, grotesque, humanist sans, serif text, slab, mono, display,
script and unclassified. Family neighborhoods use a deterministic greedy densest-anchor
algorithm within a class, radius <=0.9 in full measured space, at least three families.
Membership is bounded by anchor distance; it does not chain indefinitely through a graph.

### Measured embedding contract

`format: "typefoundry.atlas"`, `version: 1`, embedding recipe
`measured-font-space-v1`; `space_id` hashes the fitted dimension definitions,
centers and scales. **Compare stored vectors only within the same space_id.** New
families can change calibration on rebuild. Raw features remain portable so a later
index can refit all observations together. Legacy v1 snapshots without space_id
load by deriving it from their existing calibration; unknown versions are refused.

There are 33 dimensions in 10 design domains: proportions, weight, contrast, width,
overshoot, arch, counter, serif, spacing and texture. These include height ratios,
n/H stem color, o contrast and double-angle stress sine/cosine, n/o/H/O widths,
overshoots, n arch profile, counter exponent/radius/handles, foot extension, bearings
and greyness. Lengths divide by x-height, contrast is log-transformed, and angles
use sine/cosine of twice the angle. Every dimension includes its label, domain,
transform, calibration and measured-family count. The definition order is versioned.

Calibration is family-balanced median and IQR/1.349 with a documented minimum scale
for each dimension. Coordinates are clipped to +/-5 standardized units. Every face
and family carries raw nullable values, numeric vector and availability mask. Missing
values are median-imputed only for PCA display and remain masked for distance. They
are never treated as measured zero. Distance is RMS within each shared design domain,
then RMS across domains; correlated width/arch columns cannot buy extra domain weight.
At least 12 shared dimensions across four domains are required by default. Nearest
results report shared coverage and the largest standardized differences. Default
neighbors=5; configurable 1..50 and `min_shared` 4..33.

The 2D explorer uses centered PCA with deterministic power iterations and stable
axis signs. Explained variance and loadings are retained. PCA is a lossy index view,
not the embedding itself: close dots can differ in hidden dimensions, and blank plot
pixels are not a design-gap test. You can instead choose actual measured axes, filter
classes, search families, inspect styles and see all numeric evidence and sources.
Chart.js 4.5.1 is vendored with its MIT license; the generated HTML embeds both data
and renderer and needs no network requests, font embeds, service, account or API key.

### Gap proposals and original-design loop

For each class with >=8 sufficiently confident families, deterministic candidates
start as convex combinations of three neighboring family representatives. Stress
sine/cosine targets are projected back to a unit direction (ambiguous zero directions
are masked), and the n/H advance ratio is derived from the two target advances.
These algebraic constraints avoid proposing impossible vectors; the full geometry
still needs construction and testing. Candidate
masks require shared measured evidence. Candidates must be >=0.25 distance from the
nearest measured face across **all** styles/classes, and <=2.0 from the third-nearest
same-class family; ranking and diversification use
**full-space** group-balanced distances, not the PCA projection. At most 24 are kept
by default (`--gaps` / `gap_count`, 0..200). Supporting neighbors, actual numeric
targets, transforms, mask, score and original-design instructions accompany each brief.

A gap means an underrepresented combination **in this collection**. It does not show
that the combination is globally new, technically feasible, attractive, or useful.
The score is not a probability or a typeface originality test. These are construction
hypotheses: draw a new skeleton, establish n/o/H/O, space them, then test words and
sizes. No reference outline coordinates enter briefs or the atlas.

`atlas-evaluate` reads an original font and compares it to a selected gap. It reports
full-space distance, shared dimensions, and signed per-feature differences sorted by
size, including whether a measured feature should increase/decrease or is near the
target. This closes the numerical iteration loop without changing a font or implying
that matching the vector guarantees quality. An exact ID or unique prefix >=8
characters selects a brief. JSON results can be retained as experiment evidence.

Class observations require >=5 families and report 10th/median/90th percentiles.
Cross-domain descriptive associations require >=12 paired families and |r|>=0.65.
They are exploratory observations with sample counts, not causal laws, significance
tests or universal advice. Classifier errors and collection bias affect them.

### Persistent outputs and command/MCP access

`--out` creates immutable `atlas-HASH.json` snapshots, an owned `explore.html`, and
an atomic `latest.json` pointer with full checksum. Existing unrelated output files
are refused. Readers validate the pointer checksum and array/mask shapes. Schema:
[typefoundry.atlas.schema.json](typefoundry.atlas.schema.json). The snapshots contain
measurement features, source paths/licenses, families, vectors, calibration, neighbor
graphs, neighborhoods, observations, briefs, coverage and failures; no font outlines.

Run operations (none change the active font or undo history):

```json
{"op":"atlas_build","paths":["/worker/studies","/worker/sources.json"],"options":{"output":"/atlas","min_shared":12,"neighbors":5,"gap_count":24}}
{"op":"atlas_query","path":"/atlas/latest.json","family":"Vostok Serif v3","limit":5}
{"op":"atlas_briefs","path":"/atlas/latest.json","class":"serif text","limit":5}
{"op":"atlas_evaluate","font":"original.ttf","path":"/atlas/latest.json","brief":"GAP_ID","options":{"profile":"atlas"}}
```

MCP tools: `build_font_atlas(paths, options)`, `query_font_atlas(path, family, limit,
min_shared)`, `font_design_briefs(path, class, limit)`, and
`evaluate_font_brief(font, path, brief, options)`. Build writes owned artifact files;
query/brief/evaluation tools are source-read-only. CLI `atlas` prints a compact build
summary. JSON/HTML are standalone exports; this is an exact small-collection vector
index, not a hosted vector database or learned latent font generator.

Validation covers family-balanced calibration, content deduplication/source aliases,
masked missing evidence, deterministic ordering, domain weighting, full-space gaps,
PCA degeneracy, reviewed labels, safe HTML serialization, pointer checksums/output
ownership, roundtrip queries and read-only original-font evaluation. Python tests
exercise migrations, discovery exclusions and actual engine scan/resume/deduplication.
`tools/test_atlas_browser.cjs` exercises search, filters, actual axes, family details,
class observations, coverage, gap briefs and desktop/mobile layouts against real data.

Family grouping corrections are separate from class labels. `--family-aliases FILE`
(run option `family_aliases`) accepts evidence-backed mappings:

```json
{"Example Book Italic":{"family":"Example","reason":"Shared PostScript root verified in the source name table"}}
```

The atlas keeps the original face names, all source aliases and grouping notes.
Only the family grouping key changes. This is useful for fonts whose legacy (or even
nominal typographic) family field contains weight/style words while the PostScript
root identifies the family. It does not silently strip style words from every font.
The explorer exposes grouping evidence. Family aliases change the fitted population
and therefore can change space_id; rebuild neighbors/gaps together after applying them.

The explorer can plot either family representatives or all measured face locations.
Calibration stays family-balanced in both views. Gap occupancy always checks all
measured styles, so an existing bold or italic cannot be ignored simply because it
is not the family representative. Briefs expose the closest measured face as well
as three supporting family representatives.
