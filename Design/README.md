# Design

The editor window is `typefoundry` (crate `App/crates/foundry-app`). It is a client of `foundry-api`. It reads and changes the font through session commands; it never writes font fields itself.

## The window

The window and the Start menu shortcut use the Type Foundry mark in `App/crates/foundry-app/assets/type-foundry-icon.png`.

The window opens on the **overview**: every glyph as a thumbnail at one shared scale. Double-click a glyph, press Enter, or press Tab to open it in the **editor**. The menus hold everything, and Help > Keyboard shortcuts lists the keys.

- **Menu bar.** File (New font, Open, Open UFO folder, Open SVG folder, Open family, Save, Save As, Close font, Make italic, New style from this font, Save family, Export family, Check family, Quit), Edit (Undo and Redo with step counts, select all, deselect, delete points, smooth or corner, on-curve or off-curve, reverse contour direction, align points), View (overview, editor, or both side by side, zoom, fit, panel and canvas toggles, review sheet place, onion skin, guides, Settings), Glyph (new, delete, previous, next), Tools (Select, Lasso, Pen, Rectangle, Oval, Guide), Effects (Transform, the six effects, round coordinates), and Help.
- **Toolbar.** Overview, Editor, Split, Select, Pen, Rectangle, Oval, Lasso, Guide, Undo, Redo, and Effects. Overview, Editor, and the drawing tools use a Gravity UI icon beside the name. Lasso and Guide use marks drawn for this window. The shortcut shows on hover. With two or more fonts open, a compare picker follows. Then the font name, a dot when there are unsaved changes, and the current glyph. File commands open Type Foundry or Three.js JSON, webfontjson, `.ttf`, `.otf`, `.ttc`, `.otc`, and `.woff` files, a `.ufo` folder, or a folder of SVG glyphs named with four hex digits. Save As writes Type Foundry JSON, UFO, or TrueType. The last folder persists between launches.
- **Font tabs.** One tab per open font, under the toolbar. With one family open the row shows the family once and each tab shows its style; with several families each tab shows both. A dot marks unsaved changes and × closes the font, asking first when it has changes. Ctrl+Tab and Ctrl+Shift+Tab cycle styles. Switching keeps the same glyph open when the other style has it. Make italic and New style… sit at the end of the row.
- **Families.** Make italic (the tab-row button, File > Make italic, or Ctrl+Shift+I) copies the open font as an italic style. The dialog leads with the style name and a slant slider from −30° to 30°, the same lean as Effects → Slant, default 12°. The font you copied stays open and is not changed. Effects → Slant has Make italic style, which opens that dialog at the slider's angle instead of leaning the open font. File > New style from this font (Ctrl+Shift+D) is the other member, such as Bold: name, weight, italic, and slant. File > Open family, Save family, Export family (TrueType, UFO, or JSON into a folder), and Check family. The check report lists blocking issues in ALERT and notes such as missing glyphs in AMBER. The compare picker draws another style's version of the glyph as a MUTED outline over the editor, and View > Preview every style adds a Styles section to the format sheet that sets the sample once per style. File commands open Type Foundry or Three.js JSON, webfontjson, `.ttf`, `.otf`, `.ttc`, `.otc`, and `.woff` files, or a `.ufo` folder; Save As writes Type Foundry JSON, UFO, or TrueType. The last folder persists between launches.
- **Overview.** A filter by name or character, a cell-size slider, and New glyph. Thumbnails are drawn once and cached until that glyph changes.
- **Glyph list.** The left panel, with its own filter. Click to select, double-click to edit.
- **Editor canvas.** Metric lines with labels, the advance box, black fill (nonzero, so counters stay open), and handles. View > Background, or Settings, switches the editor between White and Black. White is the bone paper with black fill. Black is a black paper with white fill. The overview grid stays on the bone paper. Scroll or pinch zooms around the pointer, right- or middle-drag pans, and double-click on empty space refits. A panel opening or resizing does not move the glyph under the pointer.
- **Select tool (V).** Click a point to select it, Shift-click to add or remove, drag empty space to box-select. Drag the selection to move it. Arrows nudge 1 unit, Shift-arrows 10. Delete removes points. Alt-click an outline to add a point there without changing the shape. Double-click an on-curve point to toggle smooth. Drag a guide to move it. Edit > Align points, and the inspector when two or more points are selected, align left, center, right, top, middle, or bottom, or distribute three or more. Alignment is one `set_points` command.
- **Lasso (L).** Drag a loop around points. The points inside are selected. Shift adds to the selection. The loop is drawn in FOCUS. No handles are added.
- **Guide (G).** Drag across the canvas for a horizontal guide, or up and down for a vertical one. Guides are yours: they show on every glyph of the family, they are not written into the font, and they persist between launches. View > Guides hides them. Drag a guide to move it. Delete removes the selected guide when no points are selected. A guide is a bright blue line (`#3D8BFF`) with an edge in the fill color, so it shows on the paper and on the letter. A selected guide is FOCUS.
- **Pen tool (P).** Click to start a contour and add on-curve points, Shift-click for off-curve points. Click the first point to close the contour. Esc stops.
- **Rectangle (R) and Oval (O).** Drag on the current glyph. The box is normalized, so the drag direction does not matter. Either side shorter than 4 units is ignored. Snapping follows the canvas setting. Each shape is one `add_contour`, so one undo. An amber outline follows the pointer. Esc cancels the drag. The new contour's points are selected.
- **Inspector.** Font name, units per em, and the four metrics. Style: family, style name, weight, italic, and italic angle, plus Make italic, New style, and Check family. The glyph's name, Unicode, and advance, with sidebearings, ink box, and counts shown, plus Reverse, Round, and Delete. For one selected point: X, Y, on-curve or off-curve, and smooth. For several: Align, Smooth, Corner, Delete, and Transform.
- **Effects (Ctrl+E).** Slant, Scale, Rotate, Move, Flip horizontal, and Flip vertical, applied to the selected points, this glyph, or all glyphs. An AMBER outline previews the result on the canvas before Apply. Apply is one `transform` command, so one undo step even for the whole font. On Slant, Make italic style opens the italic dialog at that angle and does not change the open font.
- **Onion skin.** View > Onion skin draws the previous glyph and the next glyph beside the one you are editing, at their advances, as MUTED outlines with no handles and no points. It can be turned off. Turning it on refits the view so the neighbors are in frame.
- **Workspace.** View > Overview and editor, or the Split control on the toolbar, shows the glyph overview and the editor at the same time. Drag the bar between them. Overview, Editor, Ctrl+1, and Ctrl+2 return to a single pane.
- **Review sheet.** The preview is its own panel. View > Review sheet turns it off. Bottom, Right, or Window places it: Bottom and Right are resizable docks, and Window is a panel you can move and resize on its own. Closing that window hides the sheet. The copy column holds headline and paragraph blocks: type into them, switch a block's role, add one, or remove one. The format sheet beside it sets each block in the font, headlines at 48 pixels and paragraphs at 15, then a size waterfall of the first headline at 36, 24, 16, and 11. Lines wrap. Characters the font lacks show as a hollow box. Click a glyph in the sheet to select it. The copy persists between launches.
- **Settings (Ctrl+,).** Fill, stroke, metrics, point numbers, pointer coordinates, snapping moves to whole units, handle size, overview cell size, and which panels show. Settings, the preview copy, and the last folder persist between launches.
- **Status bar.** The last result in SIGNAL, or the error in ALERT, plus the tool, the selection count, and the zoom.

Every change goes through a session command (`move_points`, `split_segment`, `add_contour`, `transform`, `set_metrics`, and the rest in `documents/api.md`), so undo covers all of it. The same commands are open to plugins and the CLI. The window never writes font fields itself. On-curve corner points draw as diamonds and smooth points as circles. A ring marks each contour's first point.

A path given on the command line opens on launch: `typefoundry C:/fonts/Wide.ufo`.

On first launch on Windows the window adds `Type Foundry` to the Start menu, pointing at `C:\Users\Troy Havelin\AppData\Local\typefoundry-target\release\typefoundry.exe`. It skips that when the shortcut already exists.

## Chrome

Chrome follows the Havelin v2 system surface used by Frame Extractor. Only these colors are used:

| Token | Hex | Use |
| --- | --- | --- |
| PAGE | `#030303` | Canvas surround, text fields |
| PANEL | `#090907` | Toolbar, glyph list, status bar |
| RAISED | `#11110D` | Buttons |
| HAIRLINE | `#302A1E` | Panel borders, button edges |
| HAIRLINE_STRONG | `#5B4A2D` | Hovered button edge |
| INK | `#DED9CE` | Text |
| MUTED | `#9D988C` | Secondary text, metric lines |
| BONE | `#F3EEE4` | Canvas ground (the editor's white background) |
| GUIDE | `#3D8BFF` | Editor guides |
| FOCUS | `#D8FF00` | On-curve handles, selection, pressed edge |
| AMBER | `#E8B65A` | Off-curve handles and their tethers |
| SIGNAL | `#8AE6A3` | Selected handle, success status |
| ALERT | `#F07461` | Error status |
| INVERSE | `#030303` | Text on FOCUS |

Fonts are egui's defaults. Inter and IBM Plex are not vendored. Toolbar icons are the Gravity UI set (MIT, Yandex), vendored as SVG in `App/crates/foundry-app/icons` and tinted with the chrome text color. Lasso and Guide are marks drawn for this window, in that same folder.

## Canvas

The glyph canvas stays neutral: BONE ground, black fill (nonzero winding, so counters stay open), MUTED metric lines. Black background turns the ground black and the fill white. On-curve handles are FOCUS circles, off-curve handles are AMBER squares, and the selected handle is SIGNAL. Each handle has a thin black rim so FOCUS reads on BONE. Open contours draw in the fill color. Guides are GUIDE blue.

## Next

Interaction ideas worth borrowing from Shift, without copying its UI: select, pen, and hand tools, a glyph grid, and masters as sources you can preview between.

The product to design toward is a bench for making a font, blending two compatible faces, generating a starting face from a prompt or an image, and handing the same commands to a plugin or an agent.
