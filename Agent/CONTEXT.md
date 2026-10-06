# Type Foundry

A local professional type kit. Design a font, blend two compatible faces into a new one, generate starting outlines from a prompt or an image, export real fonts, and drive all of that from plugins and from an agent.

## Folder map

- `App/` — Rust workspace. `crates/foundry-core` is the font model, blend, UFO exchange, and TrueType export. `crates/foundry-api` is the command session. `crates/foundry-cli` is the `foundry` binary. `crates/foundry-app` is the `typefoundry` drawing window. `crates/foundry-mcp` is the `foundry-mcp` stdio MCP server.
- `Agent/CONTEXT.md` — this file.
- `Design/` — the window as built, its chrome tokens, and editor direction.
- `documents/api.md` — the command contract.
- `documents/shift-reference.md` — what we take from Shift, and what we do not copy.
- `documents/Keys/` — credentials, gitignored. None yet.
- `Fonts/` — Roboto English and Roboto Cyrillic, as Type Foundry JSON. See `Fonts/NOTICE.md`.

## Collaboration

PM notes live here. Product code lives in `App/`. Plugins, the CLI, and agents share `foundry-api`. Do not add a second way to change a font.

## Project facts

- Path: `T:\troy-freeform\TypeFoundry`
- Remote: `git@github.com:thavelin/Type-Foundry.git` — https://github.com/thavelin/Type-Foundry
- Stack: Rust edition 2024, stable MSVC, serde, norad 0.18 (UFO), eframe 0.36 on glow plus rfd 0.15 (window). The MCP server is hand-rolled JSON-RPC with no async runtime.
- Version: `v0.1-dev`. Build day 2.
- Run from `App/`: `cargo run -p foundry-cli -- --help`. Window: `cargo run -p foundry-app --release`. MCP: `cargo run -p foundry-mcp` or `foundry mcp`.
- Check: `powershell -ExecutionPolicy Bypass -File App/scripts/check.ps1` from the repo root. This machine's execution policy rejects unsigned scripts.
- Build output: `C:\Users\Troy Havelin\AppData\Local\typefoundry-target` via `App/.cargo/config.toml`. This share creates programs without execute permission, so the target directory stays on `C:`.
- Hub: https://app.notion.com/p/3ef627d6cfdc81e4a936e4f714b7aff0 — Projects database, priority Next.
- Agent workspace: https://app.notion.com/p/3ef627d6cfdc8187905bfeaf3fed4d0e
- Design page: https://app.notion.com/p/3ef627d6cfdc818abbe0e6be4187020d
- Owner's Notes: the `Owner's Notes` section at the bottom of that hub page. Agents do not edit it.
- Domain guardrail: local font authoring only. Do not upload fonts, outlines, reference images, or prompts. Do not copy another foundry's outlines or Shift's source into this project.

## How a font is stored

The working file is JSON, `format` `typefoundry.font`, `version` 1. A glyph is contours of `on` and `off` points, an advance, and an optional unicode. `open`, `save`, `check`, and `blend` also read and write a `.ufo` directory through the same commands. `open` also reads a Three.js typeface JSON file, a webfontjson file, `.ttf`, `.otf`, `.ttc`, `.otc`, and WOFF 1 (`.woff`) and WOFF2 (`.woff2`), both unpacked into an sfnt with `wuff`. Embedded OpenType is refused. Saving over `.otf`, `.ttc`, `.otc`, `.woff`, `.woff2`, or `.eot` is refused. UFO import keeps the default layer, sorts glyph names, and keeps the first Unicode value. Anchors and guidelines are ignored. Components are flattened into their glyph on import. Kerning groups, kerning pairs, ligatures, and `features.fea` are kept. A save over an existing UFO updates fontinfo and the default layer and leaves lib data, images, and other layers in place. Images and implied-on qcurves are refused. `save` to a `.ttf` path writes an installable TrueType file: cubics become quadratics, open contours are closed with a straight edge, and Unicode outside the Basic Multilingual Plane is refused. The name table includes a unique identifier (name ID 3) and a version (name ID 5). An empty unique id is the PostScript name plus the version, with no foundry prefix. An empty vendor id is four spaces. Windows Font Viewer rejects a file that omits the unique name. Glyph data is padded so every `loca` offset is even. `save`, `blend`, and the family exports refuse to replace an existing file unless `force` is set, and then keep the previous file as `name.bak`. The drawing window saves with `force`.

## How blend works

`t` 0 is the first font, `t` 1 is the second, `t` 0.5 is the midpoint. Values outside that range extrapolate. The blend is refused unless both fonts share units per em, glyph names, unicode, contour counts, closed flags, point counts, point kinds, and smooth flags. That is the variable-font master rule. Unrelated typefaces are a later matching problem, not a silent morph.

## Strategic focus

The product is a full type bench: edit outlines, blend faces, generate from a prompt or an image, export installable fonts, and let plugins and agents use one API.

Frozen for now:

- No account, sync, store, or upload.
- No fork of Shift and no dependency on Shift crates. Shift is the architecture reference. Its public app already draws, interpolates masters, and exports variable TrueType. Scripting and an AI API are still on its future list. That gap is ours.
- JSON `typefoundry.font` stays the working document. UFO is an exchange path on the same load and save calls, not a second editor model.
- Generation calls a provider through `put_glyph`. It does not live inside the geometry crate. In-app model calls use SpaceXAI when that day comes.

## Decisions

- Rust owns the font. The `typefoundry` window is a client of `foundry-api`. A drag, a new rectangle, and a new oval are session commands.
- The JSON command stream is the plugin and agent API. `foundry-mcp` wraps it for chat clients. It does not get its own font mutations.
- Blend refuses incompatible outlines and reports why.
- House UI uses the Havelin v2 system surface for chrome. The glyph canvas stays neutral: BONE ground, black fill, FOCUS / AMBER / SIGNAL handles.
- A folder of filled SVG glyphs, each named with four hex digits (`0041.svg` is A), opens through `Font::load`. Tight crops are scaled so the flat x-height is 500 in a 1000-unit em and share one baseline.
- Toolbar icons are the Gravity UI set (MIT, Yandex), vendored as SVG. The window draws them locally and does not fetch them.
- The window, the executable, and the existing Start menu shortcut use Troy's mark at `App/crates/foundry-app/assets/type-foundry-icon.png`.
- Cargo target directory stays on `C:`.

## Session log

### 2026-10-06 — Components, WOFF2, offset arcs, PDF proof, Open recent

- Focus: the five leftovers from the 2026-10-05 entry, plus File > Open recent. Persistent sessions stay out. Variable fonts stay later.
- Components: a UFO glyph's components are flattened into its contours on import, moved by the component transform (UFO spec order). A missing base or a component that uses itself is named. A save writes the flattened outline, not the reference.
- WOFF2: `wuff` (pure Rust, MIT) decodes it into an sfnt, so `open`, `check`, `blend`, and webfontjson read it. A broken file is named. Test builds a real WOFF2 with `ttf2woff2` (dev-dependency only).
- Offset adds points: `add_points` with `corner: round` on `offset` and `stroke`. Each sharp outside corner becomes two on-curve ends and one cubic. Needs both amounts non-zero; with `keep_metrics`, metric-line corners stay sharp. Arcs go last, after gap limiting.
- PDF proof: `proof` writes a one-page vector PDF when the path ends in `.pdf`. Curves are kept, no font is embedded, and the content stream is zlib-compressed. Rendered with PyMuPDF to check it draws.
- Open recent: File > Open recent, ten entries kept in the window settings (`recent`), updated on open and save, with Clear list. Missing files show disabled.
- Validation: `powershell -ExecutionPolicy Bypass -File App/scripts/check.ps1` passed. clippy clean.
- Not done: hinting. "Automatic hinting" was not defined in the notes and is not started. Variable fonts. Components are not kept as references.
- Git: committed locally on `main`, not pushed. `origin/main` (`734756a`) has two commits that are not here: `bbb79af` "Sanitize repository for public sharing" and its merge PR #5. They delete `Agent/CONTEXT.md`, `AGENTS.md`, `_CONTEXT.md`, `App/.cargo/config.toml`, and `documents/day-2-agent-prompt.md`, and they remove personal paths from `README.md`, `documents/api.md`, `svgfont.rs`, and `main.rs`. This branch still carries those personal paths in `Agent/CONTEXT.md`, so a push to `main` would undo the sanitize. Push is held until Troy decides how to reconcile.
- Not committed on purpose: the untracked `_CONTEXT.md` files (generated by a local model, with inaccurate content) and `documents/screenshots/`.

### Next plans

1. Decide hinting. Options: skip for now; a basic automatic hinter (large, limited quality); or keep an imported TTF's hinting and write it back until the outlines change.
2. Reconcile with origin before any push. Options: keep the session log and personal paths out of the repo (private file, or git-ignored); or rebase this work onto `734756a` and re-check the paths.
3. Exercise `add_points` through `foundry run` and the window with a screenshot, since only the unit tests cover it now.
4. Components as references. This changes the model and blending, so it needs Troy's decision first.
5. Variable fonts (`fvar` / `gvar`). Deferred by Troy.
6. Review the untracked `_CONTEXT.md` files. Keep or delete them.

### 2026-10-05 — Overwrite protection, metadata, kerning, and offset path

- Focus: the working-session list. Overwrite protection, export metadata, and kerning first, then the weight, italic, check, and proof tools that were blocking family work.
- `save`, `blend`, `new`, `save_family`, `export_family`, and `proof` refuse an existing file unless `force` is set. A forced write keeps `name.bak`. Saving a `.ufo` no longer deletes `features.fea`, lib data, images, or other layers. The window's Save and family export pass `force`. Command and MCP saves stay strict.
- Name-table fields live on the font: copyright, designer, licence, licence URL, version, a four-character vendor id, and the unique id. Dots stay in file names and PostScript names, so `v4.5` does not become `v45`. Width class is an OS/2 value from 1 to 9. A duplicate Unicode value is an error.
- Kerning groups and pairs, and ligatures such as `fi`, are stored in the JSON, written to UFO and to a TrueType `kern` table plus a `liga` lookup, and read back from UFO and from binary fonts.
- `offset` moves existing points, with separate horizontal and vertical amounts, so a weight change stays compatible. Metrics can stay put, growth can stop at a gap, and a preview returns the outlines without changing the font. `stroke` builds an outline or inline. `scale_width` changes width and keeps stem thickness. `slant` recenters each glyph in its advance.
- Also wired: family sidebearings with one undo step per style, outline and spacing checks, compatibility issues that name the contour and point, `foundry proof` to PNG, `foundry diff`, `move_glyph`, `copy_family` for a version folder, and `foundry run` JSON that escapes non-ASCII as `\u`. A missing JSON field is named in the error. `foundry info` prints metrics, style, coverage, and kerning counts. MCP is 38 tools.
- Validation: `powershell -ExecutionPolicy Bypass -File App/scripts/check.ps1` passed. 104 tests, 1 ignored Roboto regen, clippy clean.
- Not done: a variable font (`fvar`/`gvar`), components, WOFF2, hinting, a session that survives the process exiting, an offset mode that adds points, and a PDF proof. MCP does not wrap every older edit command. `foundry run` is still the full command stream. Not committed. Not pushed.

### 2026-10-04 — Editor tools are on main

- Commit `62c801c` (`62c801c899da14a8947d48dd311965b8f7a8160f`) is on `main`. Push `f9883a3..62c801c` went to `https://github.com/thavelin/Type-Foundry.git` with the one-shot safe.directory and `gh auth git-credential`. Origin stays SSH. No force-push.
- The release window was rebuilt and opened with the title Type Foundry, then the process was closed. The Start menu shortcut still targets `C:\Users\Troy Havelin\AppData\Local\typefoundry-target\release\typefoundry.exe`.
- That build has Make italic, family-wide guides, onion skin, the split overview and editor, a review sheet that can sit on the bottom, the right, or in a window, the lasso, point alignment, and File → Export family. TrueType export includes the unique name Windows requires.

### 2026-10-04 — Vostok Serif v3 italic

- Focus: take the installed v3 drawings and add a 10° italic so Windows shows Regular and Italic as one family.
- Used the debug `foundry` session: `open` `VostokSerif-v3.ttf`, `derive_style` style Italic slant 10, `family_check` ready with no issues, `export_family` TTF. The source font was not changed. Italic angle is −10. The shear is `tan(10°)`, the same math as Effects → Slant.
- Files: `T:\troy-freeform\Fonts\Vostok-Serif\VostokSerifv3-Regular.ttf` and `VostokSerifv3-Italic.ttf`. On H and o, Y stayed put and X matched the slant within half a unit, which is the TrueType integer rounding. Advances stayed put.
- Installed for this user. GDI accepted both files. One private collection reported a single family with Regular and Italic. After install, Windows lists `Vostok Serif v3` and both `FontStyle.Regular` and `FontStyle.Italic` construct. The earlier single-file v3 install was replaced. The older `Vostok Serif` Regular install is still there. Not committed.

### 2026-10-04 — Vostok Serif v3 installs

- Focus: Troy saved edits as `VostokSerif-Regular3.ttf` and Windows would not install it.
- Cause: that file, plus `VostokSerif-Regular_v2.ttf` and `Vostok-Serif.ttf`, was written by the release app from 7:07 PM. Its name table has four records and no name ID 3. GDI `AddFontResourceEx` returned 0. The family string had also stacked on each save, so the open name was `Vostok Serif Regular Regular 3`.
- The edited outlines were re-exported with the fixed writer to `T:\troy-freeform\Fonts\Vostok-Serif\VostokSerif-v3.ttf`. Family `Vostok Serif v3`, style Regular. Name ID 3 is `Havelin: Vostok Serif v3 Regular`. Every glyph matched the rejected Regular3 file. `Regular3.ttf` was left in place.
- Installed for this user at `C:\Users\Troy Havelin\AppData\Local\Microsoft\Windows\Fonts\VostokSerif-v3.ttf`. Registry name `Vostok Serif v3 Regular (TrueType)`. `AddFontResource` returned 1. The earlier `Vostok Serif Regular` install was left alone.
- The Start menu exe is still the 7:07 PM build, and that process (PID 25212) was left running. The next Save As from that window will fail the same way until the release exe is rebuilt.

### 2026-10-04 — Make italic, guides, and a split workspace

- Focus: with a font open, making an italic family member had to be obvious, and the editor needed guides, onion skin, a split view, a movable review sheet, a lasso, and point alignment.
- Italic: the tab row leads with Make italic. File > Make italic (Ctrl+Shift+I), the inspector, and Effects → Slant → Make italic style open the same dialog. The slant slider is −30° to 30°, default 12°, the same lean as Effects → Slant. It copies the font with `derive_style`. The open font is not changed. File > New style (Ctrl+Shift+D) is the other member, such as Bold.
- Guides: the Guide tool (G) drags a horizontal or vertical line. Guides are stored with the view, keyed by family name, shown on every glyph, and never written into the font. View > Guides hides them. Delete removes the selected guide when no points are selected.
- Onion skin: View > Onion skin draws the previous and next glyphs beside the current one, MUTED outlines, no handles. Turning it on refits the view.
- Workspace: toolbar Split, or View > Overview and editor, shows the glyph grid and the editor together. The bar between them drags. The review sheet can sit on the bottom, on the right, or in its own window.
- Lasso (L) selects the points inside a loop. Shift adds. Align (Edit > Align points, and the inspector) is one `set_points` command, so one undo. Top is the greater font Y.
- Code: `App/crates/foundry-app/src/family_ui.rs`, `effects.rs`, `app.rs`, `canvas.rs`, `guides.rs`, `align.rs`, `settings.rs`, `panels.rs`, `icons/guide.svg`, `icons/lasso.svg`. `set_points` is in `foundry-core` `edit.rs`, `foundry-api`, and the MCP tool `points_set`. MCP is 19 tools. `documents/api.md` and `Design/README.md` match the window.
- Validation: the Windows check passed, 86 tests, 1 ignored regen test, clippy clean. New tests cover `set_points`, one undo for two points, the lasso hit test, guide hit testing, and alignment. The debug window opened with the title Type Foundry and was closed. The new controls were not clicked.
- Not committed. Not pushed. The TrueType install fix is still uncommitted on the same tree. Local `main` is the family merge `f9883a3`.

### Correction - window smoke

- The debug window was not opened. `typefoundry.exe` PID 25212 was already running from `C:\Users\Troy Havelin\AppData\Local\typefoundry-target\release\typefoundry.exe`. That process was left alone. The new editor is compiled into the debug build. The Start menu shortcut still launches the release exe, which does not include Make italic, guides, onion skin, the split, the movable review sheet, the lasso, or alignment. The new controls were not clicked.

### 2026-10-04 — Windows can install an exported TTF

- Focus: Font Viewer rejected `T:\troy-freeform\Fonts\Vostok-Serif\SVG.ttf`.
- Cause: the name table had no unique font identifier (name ID 3). The outlines, the checksum, and DirectWrite were fine. GDI and Font Viewer refuse a TrueType file without that record. Name ID 5 is `Version 1.000`. Glyphs are padded so `loca` offsets are even.
- Code: `App/crates/foundry-core/src/ttf.rs`. Save As `.ttf` and Export family both use that writer.
- Validation: the Windows check passed, 81 tests, 1 ignored regen test, clippy clean. The square test asks GDI to accept the file. Font Viewer opened the re-exported Vostok as `Vostok Serif Regular (TrueType)` with an Install button, then the process was closed.
- File: `T:\troy-freeform\Fonts\Vostok-Serif\VostokSerif-Regular.ttf`. `SVG.ttf` was left in place. Not committed. Not pushed. Local `main` is the family merge `f9883a3` plus this uncommitted fix.

### 2026-10-05 — Several fonts and families (cloud)

- Focus: edit a Regular and an Italic side by side and ship them as one family. Branch `claude/font-families`.
- Model: every font has a `style` (family, style name, weight, italic, italic angle). Files without it load as the Regular of a family named after the font. TrueType, OpenType, and UFO import read it. TrueType and UFO export write it with proper style linking: name IDs 1, 2, 16, and 17, OS/2 weight and `fsSelection`, `head.macStyle`, `post.italicAngle`, and the `hhea` caret slope.
- Session: holds many open fonts, each with its own undo history and unsaved-changes flag. Existing commands act on the active font. New commands: `fonts`, `select_font`, `close_font`, `set_style`, `derive_style` (with an optional slant), `family_check`, `save_family` and `open_family` (a `typefoundry.family` file beside `Family-Style.json` members), and `export_family` (TTF, UFO, or JSON, refused on blocking issues). `glyph` and `index` take an optional `font`.
- MCP: 9 more tools for those commands. `font_save` with no path now remembers a path per font, so a derived style can never overwrite another style's file.
- Window: font tabs, New style dialog, Style section in the inspector, open, save, and export family, a family check report, a compare layer that draws another style behind the glyph, and a preview line per style.
- Validation: fmt, tests, and clippy `-D warnings` passed in the Linux container. Xvfb smoke on Roboto English: made an Italic at 12 degrees, compared the Italic and Regular R, previewed both, the check came back ready, and switching tabs kept the glyph. `foundry run` exported Regular, Italic, and Bold Italic TTFs. `fc-scan` read all three as one family, Roboto Draft, with the right weights and slants.
- Next: interpolating between styles, a weight axis with masters, and a variable-font export. Kerning stays later.

### 2026-10-04 — Pushed the drawing slice

- Focus: put the local slice on `main`.
- Commit `168f996` (`168f996bc9caa9e56b53b3c7238f8a98b0b419c8`) is on `main`. It adds rectangle and oval, the preview pane, Gravity UI toolbar icons, SVG folder import, and the window mark. Pushed `dbca862..168f996` to `https://github.com/thavelin/Type-Foundry.git` `HEAD:main` over HTTPS. Origin stays the SSH remote. The tracking line may still say `origin/main` is gone. That was left alone.
- Vostok outlines were not committed. The earlier session entries below still say "not pushed" because that was true when they were written.

### 2026-10-04 — Window icon

- Focus: use Troy's mark as the Type Foundry icon.
- The source is `E:\random\Type-Foundry-Icon.png`, a 256×256 image. A copy lives at `App/crates/foundry-app/assets/type-foundry-icon.png`. The window decodes it for the title-bar icon. The Windows build embeds that PNG in an icon resource, so the executable and the Start menu shortcut use it too. The existing shortcut still points at the C: release exe. Its icon location is that exe.
- Validation: `the_app_icon_is_the_square_mark` passed. The release exe contains the PNG, opened with the title `Type Foundry`, and was closed. The glyph view was not clicked.
- Not pushed.

### Correction - Window icon validation

- The full Windows check later passed: 73 tests (6 api, 12 app lib, 10 app bin, 1 cli, 38 core, 6 mcp), 1 ignored regen test, clippy clean. `the_app_icon_is_the_square_mark` is one of the 10 app-bin tests. The release exe that contains the PNG was linked before a one-line clippy change in `app_icon` (`as_chunks`). The icon pixels are the same, and that exe was not rebuilt after the line. Not pushed.

### 2026-10-04 — SVG folder to a font

- Focus: open Troy's first font, `T:\troy-freeform\Fonts\Vostok-Serif\SVG`, and line the letters up.
- `Font::load` reads a folder of filled SVGs. The file name is four hex digits (`0041.svg`, also `U+0041.svg` or `uni0041.svg`). A folder named `SVG` takes the parent name, so this one opens as Vostok Serif. Strokes that were not expanded to fills are refused. `usvg` 0.45 parses the paths with text features off.
- The 90 Vostok files are tight crops at one scale. Flat letters sit on y = 0. Round letters split their extra height as overshoot. `p`, `q`, and `g` hang from the round x-height; `y` hangs from the flat x-height; `j` shares the descender. The flat x-height becomes 500 units at 1000 UPM. Cap height, ascender, and descender are measured from the aligned ink. Each crop gets 40 units of sidebearing on both sides. A space is added at 250 because the folder has no `0020.svg`.
- File > Open SVG folder… picks the directory. `foundry info` accepts the same path.
- Validation: the Windows check passed. 72 tests (6 api, 21 app, 1 cli, 38 core, 6 mcp), 1 ignored regen test, clippy clean. The Vostok folder itself is one of those tests: 91 glyphs, x and H on the baseline, o overshoots, p descends, the period sits on the baseline, the comma hangs. The release window opened that folder, the process title was `Vostok Serif - Type Foundry`, and the process was closed. The glyphs were not clicked.
- Not pushed. The rectangle, oval, preview pane, and Gravity UI icons are in the same local tree. `main` is still `dbca862`.
- Deferred: kerning, a real space drawing, optical sidebearings per letter, and components. Generation stays frozen.
- Next: Troy looks at Vostok Serif in the window. Push waits until he asks.

### 2026-10-04 — Gravity UI toolbar icons

- Focus: replace the text-only toolbar and Tools menu with Gravity UI icons, kept beside the names.
- Nine SVGs from the Gravity UI set (MIT, Copyright (c) 2022 YANDEX LLC) live in `App/crates/foundry-app/icons`, with `LICENSE` beside them. `foundry-app` rasterizes them with `resvg` 0.45 (`default-features = false`). `currentColor` becomes white, then egui tints the icon with the chrome text color. The window does not fetch icons.
- Mapping: Overview `layout-cells`, Editor `pencil-to-square`, Select `location-arrow`, Pen `pencil`, Rectangle `square`, Oval `circle`, Undo `arrow-rotate-left`, Redo `arrow-rotate-right`, Effects `magic-wand`. Hover text still carries the shortcut sentence. The Tools menu uses the same four tool icons.
- Validation: the Windows check passed. 65 tests (6 api, 21 app, 1 cli, 31 core, 6 mcp), 1 ignored regen test, clippy clean. `icons::tests::every_toolbar_icon_rasterizes` passed. The release `typefoundry.exe` opened `Fonts/Roboto-English.json`, the process title was `Roboto English - Type Foundry`, and the process was closed. The icon buttons were not clicked. The Start menu shortcut was left alone.
- Not pushed. This sits on the same unpushed tree as the rectangle, oval, and preview pane. `main` is still `dbca862`. MCP is still the original 9 tools.
- Deferred: icon-only buttons, icons on the rest of the menus, and the other 790 Gravity UI icons. MCP edit commands, a collection face index, components, kerning, WOFF2, OTF export. Generation stays frozen.
- Next: Troy tries the toolbar. Push waits until he asks.

### 2026-10-04 — Rectangle, oval, and the preview pane

- Focus: glyph creation tools, and a preview that can hold headlines and paragraphs.
- Rectangle (R) and Oval (O) drag onto the current glyph. Each drag is one `add_contour`, so one undo. The box is normalized. A side shorter than 4 units is refused. On-curve rectangle corners are not smooth. The oval is four cubic quadrants (kappa 0.5522847498307936), 12 points, closed, first point at the right. An amber ghost follows the drag. Esc cancels it. The new points are selected.
- The bottom strip is now a resizable preview pane (`preview.rs`). The copy column is headline and paragraph blocks (add, remove, retype, switch role). The format sheet sets headlines at 48px and paragraphs at 15px, then a size waterfall of the first headline at 36, 24, 16, and 11. Lines wrap in `lay_text`. Missing characters stay a hollow box. Click a glyph in the sheet to select it. The copy persists under the eframe key `preview_copy`.
- While a text field is focused, Ctrl+Z stays with that field. Font undo still uses Ctrl+Z when nothing is being typed.
- `foundry-app/src/lib.rs` no longer claims the window writes only with `move_point`.
- Validation: the Windows check passed. 64 tests (6 api, 20 app, 1 cli, 31 core, 6 mcp), 1 ignored regen test, clippy clean. The release `typefoundry.exe` opened `Fonts/Roboto-English.json`, the title became `Roboto English — Type Foundry`, and the process was closed. Rectangle, oval, and the sheet were not clicked in the window. Their geometry and wrapping are covered by the new unit tests.
- Not pushed. The Start menu shortcut was left alone. MCP is still the original 9 tools.
- Deferred: MCP coverage of the edit commands, a collection face index, components, kerning, WOFF2, OTF export. Generation stays frozen.
- Next: Troy tries the new tools and the pane. More drawing tools wait on what he asks for.

### 2026-10-04 — Windows check of the editor

- Focus: PR #3 is on `main`. Confirm the editor on this PC.
- Local `main` fast-forwarded from `b43b9e5` to `521a3e0`. The branch adds edit commands and undo (`9c49359`), fixes typeface.js curve order and regenerates the two Roboto files (`d2c18f2`), and rebuilds the window (`362351f`).
- typeface.js `q` and `b` put the end point first and the control points after, which matches three.js `Font`. The Roboto subsets still load as 95 English glyphs and 255 Cyrillic glyphs.
- Validation: the Windows check passed. 57 tests (6 api, 13 app, 1 cli, 31 core, 6 mcp), 1 ignored regen test, clippy clean. The release `typefoundry.exe` opened `Fonts/Roboto-English.json` and the window title became `Roboto English — Type Foundry`. The process was then closed. The existing Start menu shortcut was left alone.
- Not re-done here: the Linux smoke of box select, nudge, slant, and the pen. Those remain self-reported. MCP is still the original 9 tools. `foundry-app/src/lib.rs` still says the window writes only with `move_point`; the window sends the new edit commands.
- Next: add the new edit commands to the MCP server. A collection face index is still open. Generation stays frozen.

### 2026-10-04 — Editor UI (cloud)

- Focus: turn the bare window into an editor. Branch `claude/editor-ui`.
- Engine: 20 new session commands (point, contour, glyph, and font edits, `transform` with a per-glyph `anchor`, `round_coordinates`, `index`, `undo`, `redo`, `checkpoint`, `history`). Edits check their input before changing anything. Undo keeps 200 steps, and drags coalesce into one step.
- Window: a menu bar, toolbar, overview grid of cached thumbnails, select and pen tools, box selection, Alt-click to split a segment, an inspector, an Effects dialog with a live preview, a text preview strip, a settings window, and keyboard shortcuts. Split into `app`, `canvas`, `grid`, `panels`, `effects`, and `settings` modules.
- Fixed on the way: the typeface.js importer read `q` and `b` control points before the end point, which scrambled every curve. The two `Fonts/Roboto-*.json` subsets were regenerated from the json-fonts source.
- Validation: fmt, `cargo test --workspace`, and clippy `-D warnings` passed in the Linux container. Smoke under Xvfb on Roboto English: overview, editor, box select plus nudge (one undo step), undo, a 12-degree slant on all 95 glyphs from Effects, a new glyph drawn with the pen and closed, a point added by Alt-click and dragged, smooth toggled, and the result saved.
- Next: the MCP server still lists its original 9 tools. Add the new edit commands there. Then component support, kerning, and a Bold effect (path offsetting).

### 2026-10-04 — WOFF 1, after the binary-import merge

- Focus: PR #2 is on main. Confirm it on this PC, then take the first next step in `documents/font-formats.md`.
- Confirmed `906c0d3` (merge of `f53daaf` onto `a1568d6`). The Windows check passed before any new code: 40 tests (4 api, 8 app, 1 cli, 21 core, 6 mcp), 1 ignored, clippy clean. Save As already lists TrueType in `save_as_dialog`. The native dialog was not clicked. Saving a `.ttf` through the session was already covered by `a_saved_ttf_opens_with_names_unicodes_and_points`.
- Shipped WOFF 1 in `sfnt::unpack_woff`. `flate2` inflates a table when its compressed length is shorter than the original, and a stored table is copied. The sfnt is rebuilt and `ttf-parser` reads it. WOFF2 stays refused. `.woff` stays read-only. A webfontjson file can embed WOFF 1. `documents/api.md`, `README.md`, and `Design/README.md` no longer say that raw `.woff2` opens.
- Validation: the Windows check passed again. 44 tests (core 25 passed, 1 ignored), clippy clean. The new tests wrap `write_ttf` output, compress at least one table, and compare outlines with the `.ttf` and with the hand-built CFF font. No third-party font.
- Deferred: picking a collection face, variable-font masters, WOFF2, OTF export, and pen and select tools. Generation stays frozen.
- Next: optional face index on `open`, `check`, and `blend`.

### 2026-10-04 — Binary font import (cloud)

- Focus: open more formats through the same `Font::load`. Branch `claude/font-import`.
- Shipped `.ttf`, `.otf` (CFF or CFF2), and face 0 of `.ttc` / `.otc` through `open`, `check`, and `blend`, read with `ttf-parser` (now a normal dependency). Composites are decomposed. Implied on-curve points become real. Variable fonts give their default instance. WOFF and WOFF2 are refused by name. Saving to a binary extension other than `.ttf` is refused. TrueType export now writes `post` format 2, so names survive a round trip.
- Validation: fmt, `cargo test --workspace` (34 tests), and clippy `-D warnings` passed in the Linux container. Tests build their own fonts, including a hand-assembled CFF font. No third-party font is committed. A smoke read 58 local fonts with no failures, including a 45,000-glyph CJK `.ttc` in about 0.34 s.
- Next: see `documents/font-formats.md`. WOFF 1 and choosing a collection face are the cheap steps. Reading variable-font instances as blend masters is the valuable one.

### 2026-10-04 — Roboto English, Roboto Cyrillic, and web font import

- Focus: bring in the two sample faces Troy asked for, and open the JSON font formats those repos use.
- Shipped `Fonts/Roboto-English.json` (U+0020–U+007E, 95 glyphs) and `Fonts/Roboto-Cyrillic.json` (U+0400–U+04FF, 255 glyphs). Both are Roboto Regular read from https://github.com/7dir/json-fonts `fonts/cyrillic/roboto/Roboto_Regular.json`. That repo has no separate English file. The other scripts in the source file were left out. License: `Fonts/NOTICE.md` and `Fonts/LICENSE-APACHE.txt`.
- `open` now reads typeface JSON (`m` `l` `q` `b` `z`), webfontjson (`css` with a base64 `@font-face`, including the `callback({...})` wrapper), and `.ttf`, `.otf`, `.woff`, `.woff2`. A multi-face web font imports the regular face. `.eot` is refused. Save will not overwrite `.otf`, `.woff`, or `.woff2`.
- The Myriad Pro files in https://github.com/ahume/webfontjson were not copied. That repo is the JSON wrapper, not a font library.
- Validation: `powershell -ExecutionPolicy Bypass -File App/scripts/check.ps1` passed. 34 tests, plus one ignored regen test, clippy clean. The Roboto files load as 95 English glyphs and 255 Cyrillic glyphs. The Open dialog filters were not clicked.
- Next: TrueType in the window Save As dialog, then pen and select tools. Generation stays frozen.

### 2026-10-04 — Windows check of the merged window and MCP

- Focus: PR #1 is on `main`. Confirm the cloud slice on this PC.
- `main` is `b7eef53`, the merge of `37719dc` onto `dd0ae32`. The local checkout was fast-forwarded to that commit. `foundry-core` was not changed by the window or MCP commits.
- Validation: `powershell -ExecutionPolicy Bypass -File App/scripts/check.ps1` passed. 28 tests (4 api, 8 app, 1 cli, 9 core, 6 mcp), clippy clean.
- The release `typefoundry.exe` launched with title `Type Foundry`. The Start menu shortcut `Type Foundry.lnk` was created on first launch and points at `C:\Users\Troy Havelin\AppData\Local\typefoundry-target\release\typefoundry.exe`. The process was then closed.
- Not done: Save As offers `.json` and `.ufo` only, so a `.ttf` was not saved from the window. `save` and `font_save` already write `.ttf`.
- Next: add TrueType to the window Save As dialog and proof one file. Then pen and select tools as session commands. Generation stays frozen.

### 2026-10-04 — Day 2 (cloud): drawing window and MCP server

- Focus: Blocks 2 and 3 of `documents/day-2-agent-prompt.md`. Block 1 was already on `main` (`82f747c`), so it was not rewritten. The cloud agent first built its own Block 1, found `main` had moved, and replayed only the window and MCP commits onto `dd0ae32`.
- Shipped `foundry-mcp` plus `foundry mcp` (`454055d`): 9 tools over `Session::execute`, stdout is protocol only, logs on stderr, `font_save` remembers the last path in the wrapper. Shipped the `typefoundry` window (`8a3317e`): glyph list, fitted canvas, scroll zoom, drag a handle to send `move_point`, open and save JSON or UFO, Start menu shortcut on first Windows launch.
- Validation: built in a Linux cloud container, so the check ran as its three commands (`cargo fmt --all -- --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`) with `RUSTUP_TOOLCHAIN=stable` and a local `CARGO_TARGET_DIR`. `rust-toolchain.toml` pins MSVC and there is no PowerShell there. All passed: 28 tests. `foundry-app` also passed clippy for `x86_64-pc-windows-msvc`, which covers the Windows-only shortcut code.
- Window smoke under Xvfb: the release `typefoundry` started with title `Type Foundry` and drew a UFO. Dragging a point and clicking Save wrote the new coordinate to the UFO. The Start menu shortcut was not exercised because it only runs on Windows.
- MCP smoke: one `initialize` line on stdin printed exactly one JSON object on stdout. `foundry mcp` is in `foundry --help`.
- Linux note: `foundry-app` turns on eframe's `x11` and `wayland` features under `cfg(target_os = "linux")` only. The Windows dependency set is the one the plan named.
- Next: run `check.ps1` and launch the window once on Troy's PC so the shortcut exists. Then proof a `.ttf` saved from the window. After that, pen and select tools (add and delete points, toggle smooth, undo) as new session commands, so plugins and MCP get them too. Generation stays frozen.


### 2026-10-04 — TrueType export

- Focus: the step after UFO, while the drawing window and MCP server are Claude's cloud slice from `documents/day-2-agent-prompt.md`.
- Shipped `Font::save` for a `.ttf` path, so `save` on the command stream writes an installable TrueType file. `.notdef` is glyph 0. Tests parse the file with `ttf-parser`.
- Validation: `powershell -ExecutionPolicy Bypass -File App/scripts/check.ps1` passed. 14 tests, clippy clean.
- Deferred: the window and MCP stay with the cloud agent. Generation stays frozen. OTF/CFF and non-BMP cmap are later.
- Next: land the cloud window and MCP on `main`, then proof a saved `.ttf` from the window.

### 2026-10-04 — Day 1

- Focus: stand up the project and the blend engine, using `thavelin/Type-Foundry` as the remote. Shift is the reference, not the codebase.
- Shipped the workspace, the `typefoundry.font` document, compatible-outline blending, and the `foundry` command stream (`new`, `info`, `check`, `blend`, `run`).
- Validation: `cargo test --workspace` 7/7 passed. `cargo clippy --workspace --all-targets -- -D warnings` passed. `cargo fmt --all -- --check` passed. CLI smoke blended Narrow advance 400 and Wide advance 800 into `H` advance 600 and wrote `Narrow / Wide @ 0.5`.
- Next: Troy picked UFO open/blend/save, the drawing window, and the MCP server. UFO landed in `82f747c`. The window and MCP landed in the Day 2 cloud slice.

### 2026-10-04 — UFO exchange

- Focus: finish the local UFO stub and keep every branch on `main`.
- Shipped UFO open, blend, and save on the existing commands, plus `move_point`. Components, images, and implied-on qcurves are refused. Tests use synthetic norad fonts.
- Validation: `powershell -ExecutionPolicy Bypass -File App/scripts/check.ps1` from the repo root.
- Next: the drawing window and the MCP server, in `documents/day-2-agent-prompt.md`. Generation stays frozen.
