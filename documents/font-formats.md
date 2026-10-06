# Font formats

Every format is read into, and written from, the same `typefoundry.font` model. JSON stays the working document. Nothing here is a second editor model. All of it runs locally, and nothing is uploaded.

## What works

| Format | Open | Save | Notes |
| --- | --- | --- | --- |
| `.json` (`typefoundry.font`) | yes | yes | The working document. Lossless. |
| `.ufo` (UFO 2 and 3) | yes | yes (UFO 3) | Default layer only. Components and images are refused. |
| `.ttf` (TrueType) | yes | yes | Import decomposes composites and keeps names. Export turns cubics into quadratics. |
| `.otf` (OpenType CFF or CFF2) | yes | no | Cubic outlines come in as two off-curve points. |
| `.ttc`, `.otc` (collections) | face 0 | no | Other faces are not read yet. |
| `.woff` (WOFF 1) | yes | no | Tables are inflated and rebuilt into an sfnt, then read like `.ttf` or `.otf`. |
| `.woff2` | yes | no | Unpacked with `wuff`, then read like `.ttf`. A damaged file is named in the error. |
| SVG folder | yes | no | One filled SVG per character, named with four hex digits (`0041.svg` is A). Save the result as `.json`, `.ufo`, or `.ttf`. |

`ttf-parser` 0.25 reads binary fonts. It was already in the workspace as the export test reader. WOFF 1 is the same tables inside a zlib wrapper, so `flate2` inflates them and the importer rebuilds a normal sfnt before the parser reads it. The parser draws outlines through a pen, so the importer collects `move`, `line`, `quad`, `curve`, and `close` into closed contours. Reading the `post` table in one pass keeps a 45,000-glyph CJK collection to about a third of a second. The parser's own name lookup is quadratic.

## SVG glyphs

Point File > Open SVG folder… at a directory of filled outlines, or pass that directory to `foundry info`. Each file name is the Unicode scalar in four hex digits. A folder named `SVG` takes the parent folder's name, so `Vostok-Serif/SVG` opens as Vostok Serif.

The letters are lined up from the drawings. Flat letters sit on one baseline. Round letters keep a little overshoot above and below the x-height or the cap height. Descenders hang below the baseline. The flat lowercase x-height becomes 500 units in a 1000-unit em, and the cap height, ascender, and descender are read from H, the ascenders, and the descenders. A missing space is added at 250 units. Each tight crop gets 40 units of sidebearing on both sides. A stroke that was not expanded to a fill is refused.

## What a binary import loses

Binary fonts are built for shipping, not editing. These things are not in the file, or are not brought in:

- **Smooth flags.** TrueType and CFF do not store them, so every point comes in as not smooth. Guessing from collinear handles could differ between two masters and break blending, so the importer does not guess.
- **Implied on-curve points.** TrueType can run two off-curve points in a row with an implied on-curve point between them. That point becomes a real on-curve point, which is the model's rule.
- **Components.** Composite glyphs (accented letters) are decomposed into outlines. UFO import refuses components instead, because a UFO is an editing source and decomposing it would hide a choice.
- **Variation data.** A variable font gives its default instance. Masters and axes are not imported.
- **Kerning, features (GSUB and GPOS), hinting, anchors, and the full name table.** None of these are in the model yet.

Two binary fonts blend only when their outlines are compatible, as with any other source. Static fonts from the same family are often not compatible, because their builds differ in glyph sets and point structure. `foundry check` says why.

## Possible next steps, in order of value

1. **Pick a face in a collection.** Add an optional `face` index to `open`, `check`, and `blend`, and list the faces in the error when a collection has more than one.
2. **Variable font instances and masters.** `ttf-parser` can set axis coordinates. Reading named instances as separate sources would give blend real masters from one file, which is where it matters most.
3. **WOFF 2.** Brotli plus the `glyf` and `loca` transforms. It needs a Brotli crate and a table rebuild, so it costs more than WOFF 1.
4. **OTF export.** CFF writing keeps cubics exact, where the TrueType path approximates them.
5. **Not planned:** Type 1 (`.pfa`, `.pfb`), `.dfont`, and bitmap formats. They are legacy, and a converter handles them better than this kit would.
