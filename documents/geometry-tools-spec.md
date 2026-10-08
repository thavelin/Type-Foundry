# Geometry tools spec: offset rework, smooth_outlines, symmetry ops

Status: ready to implement. This spec covers three features and a fix list. It follows the conventions in `documents/api.md`:
- one JSON command per line through `foundry run`;
- `{"ok":…,"data":…}` responses;
- MCP tools named noun_verb and mapped to a run op;
- edits validate their input before changing anything, are undoable, and support `preview`.

Every number marked "measured" below comes from the Vostok Serif v4.3–v4.7 production work. That work was done with box-side Python prototypes of these algorithms, so the numbers are known-good targets.

Contents
1. Shared building blocks
2. `offset` rework (`path_offset`)
3. `smooth_outlines` (`outline_smooth`) and `check_smoothness` (`smoothness_check`)
4. Symmetry: `mirror`, `symmetrize`, `check_symmetry`, `glyph_from_mirror`
5. Test fixtures and acceptance
6. Fix list: other engine bugs found on Oct 8

---

## 1. Shared building blocks

Put these in a new `foundry-core/src/geometry.rs`. `offset.rs`, `smooth.rs` and `symmetry.rs` all use them.

### 1.1 Scope

Every command in this spec takes the same scope fields:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `names` | string[] | all glyphs | Glyphs to change. |
| `points` | `[[contour, point], …]` | none | A selection inside one glyph. Needs exactly one entry in `names`. |
| `contours` | int[] | none | Contours inside one glyph. Needs exactly one entry in `names`. |
| `family` | bool | false | Apply to every open style of the active font's family, as `set_sidebearing` does. |
| `preview` | bool | false | Return the resulting glyphs and change nothing. |

Rules:
- `family: true` makes one undo step per style, the same as `set_sidebearing family`. Without it, a command is one undo step on the active font.
- A command checks every glyph in scope before it edits anything. Unknown glyph, bad selection, or bad parameter → `ok:false` with the font untouched.
- `preview` returns `data.glyphs` (full glyph JSON as from `glyph`) plus the same report as a real run. History and the dirty flag stay unchanged.

### 1.2 Italic frame

An `italic` field (`"auto"` | `true` | `false`, default `"auto"`) controls whether geometry is computed in the de-slanted frame.
- `"auto"` uses the frame when the font's style is italic and `italic_angle ≠ 0`.
- In the frame, `x' = x − tan(−italic_angle)·(y − pivot_y)`. Compute in that frame, re-slant, then round when `round` is true.
- `pivot_y` defaults to `x_height / 2`. For Vostok that is 250, which is the v4.1 italic method: every Vostok italic is its upright sheared 10° about y=250, with sidebearings measured in that frame.
- Do not re-centre ink. This is the bug in today's `slant` (see §6).

### 1.3 Contour nesting and grow side (replaces `hole_flags`)

Today's `offset.rs::hole_flags` classifies a contour by testing its point centroid against the other contours. For the outer contour of an `o`, that centroid lies inside the counter, so the outer contour is classed as a hole. Correct rule:
1. Flatten each closed contour to a polygon: lines as-is, quadratic and cubic segments at 16 steps.
2. Nesting depth of contour i = the number of other contours whose polygon contains a test point of i. The test point is a point on i's boundary: the midpoint of i's longest flattened edge, nudged 0.01 units to its interior side. If that is ambiguous (it lies on another boundary), try the next-longest edge.
3. `outer = depth % 2 == 0`.
4. The interior side comes from the contour's own signed area: left of travel when the area is positive. It does **not** come from the font's direction convention, because Vostok mixes directions (period, hyphen and colon c1 are clockwise, everything else counter-clockwise).
5. Grow direction for adding weight: away from the interior for an outer contour, toward the interior for a hole.

### 1.4 Validity check (`outline_valid`)

`fn outline_valid(glyph) -> Vec<Issue>` flags:
- a self-intersecting contour (flattened polygon, ignoring touching neighbours);
- two contours that cross;
- a contour whose area sign flips against its source, meaning it inverted.

It works on the flattened curve, not the control polygon (today's `check_outlines` uses the control polygon and gives false crossings, see §6). Every command in this spec runs it on the result and falls back per glyph (see each section). Expose it as the base of `check_outlines` too.

### 1.5 Joins

For each on-curve point between two segments:
- the turn angle is the signed angle (degrees) from the incoming tangent to the outgoing tangent;
- the incoming tangent is the on-point minus the previous control point, or minus the previous on-point for a line;
- the outgoing tangent is the next control point (or on-point) minus this point.

A join is "curve–curve" when both segments are curves, and "line" when either one is a line. `fn joins(contour) -> Vec<Join{on, turn, seg_in, seg_out}>`.

### 1.6 Report shape (shared)

```json
{"ok":true,"data":{"changed":["o","H"],"unchanged":["space"],"fallback":[{"name":"M","reason":"self-intersection","action":"kept original"}],"report":{…}}}
```

---

## 2. `offset` rework (MCP `path_offset`)

### 2.1 Purpose

Make weight variants (bold or light) and contrast changes from an existing master. Points mode must keep the point structure identical, so the result blends with its source.

### 2.2 Bugs being fixed (measured on Vostok, Oct 8)

| # | Bug | Cause in `offset.rs` | Fix |
|---|---|---|---|
| B1 | With `gap>0` (the docs' own `gap:8` example), stems do not grow at all. | `limit_gaps` → `distance_toward` measures to the segment's *closest point*. Adjacent-but-one edges (serif faces, the stem's own perpendicular edges) count as "ahead" at ~0 distance. `near_point` only skips the two segments touching the point. | Replace with a ray cast against facing edges (§2.5 step 6). |
| B2 | With `gap:0`, `o O b d 0 @ #` shrink: `o` bounds 40..482 → 63..459 at h=23, v=5.5. | `hole_flags` uses the point centroid, so outer contours are classed as holes. | §1.3 nesting. |
| B3 | With `gap:0`, 19 Vostok Regular glyphs self-intersect at h=23, v=5.5. | No collision cap, no validity check, no fallback. | Gap guard plus `outline_valid` plus fallback (§2.5 steps 6–9). |
| B4 | Serif-corner notches, slanted serif ends, bent "straight" edges. | Each point moves independently, and nothing restores axis alignment or tangents. | Displacement smoothing plus post-pass (§2.5 steps 5 and 8). |

### 2.3 Command

```json
{"op":"offset","horizontal":23,"vertical":5.5}
{"op":"offset","horizontal":23,"vertical":5.5,"mode":"points","zones":{"overshoot":12},"counter_share":0.85,"min_gap_ratio":0.72,"corner":"keep","family":true}
{"op":"offset","names":["o"],"horizontal":10,"vertical":4,"preview":true}
{"op":"offset","horizontal":-8,"vertical":-3,"mode":"clean","corner":"round"}
```

MCP `path_offset`: the same fields. Keep the existing field names. `keep_metrics` stays as an alias that maps to `zones` all-on or all-off.

```json
{"name":"path_offset","inputSchema":{"type":"object","properties":{
 "horizontal":{"type":"number","default":0,"description":"Growth per side, in units, of vertical stems (the x component). Negative thins."},
 "vertical":{"type":"number","default":0,"description":"Growth per side of horizontal strokes (the y component)."},
 "mode":{"type":"string","enum":["points","clean"],"default":"points"},
 "zones":{"type":"object","properties":{"baseline":{"type":"boolean","default":true},"x_height":{"type":"boolean","default":true},"cap_height":{"type":"boolean","default":true},"descender":{"type":"boolean","default":true},"ascender":{"type":"boolean","default":true},"glyph_extremes":{"type":"boolean","default":true},"overshoot":{"type":"number","default":12}}},
 "keep_metrics":{"type":"boolean","default":true},
 "counter_share":{"type":"number","default":0.85,"minimum":0,"maximum":1},
 "gap":{"type":"number","default":0},
 "min_gap_ratio":{"type":"number","default":0.72,"minimum":0,"maximum":1},
 "gap_window":{"type":"number","default":60},
 "corner":{"type":"string","enum":["keep","miter","round","angle"],"default":"keep"},
 "miter_limit":{"type":"number","default":2.0},
 "smooth_sigma":{"type":"number","default":10},
 "post_smooth":{"type":"boolean","default":true},
 "sidebearing":{"type":"boolean","default":false},
 "italic":{"type":"string","enum":["auto","true","false"],"default":"auto"},
 "add_points":{"type":"boolean","default":false},
 "round":{"type":"boolean","default":true},
 "names":{"type":"array","items":{"type":"string"}},"points":{"type":"array"},"contours":{"type":"array"},
 "family":{"type":"boolean","default":false},"preview":{"type":"boolean","default":false}}}}
```

### 2.4 Parameters

| Field | Default | Meaning |
|---|---|---|
| `horizontal`, `vertical` | 0 | Growth per side. A point with unit normal n moves by `(n.x·h, n.y·v)` scaled by its corner factor. A vertical stem grows 2h in total, a horizontal bar 2v, and diagonals get the elliptical blend in between, which makes it contrast-aware. Both may be negative (thinning). Mixed signs are allowed. |
| `mode` | `points` | `points`: points move, nothing is added or removed, and the result blends with the source. `clean`: true geometric offset with joins, then curve re-fit. The point count changes and the response says `"compatible":false`. |
| `zones` | all on | A zone point keeps its y when it sits on a zone line or within `overshoot` of it on the outside, and its normal points out of the glyph across that zone (down at the baseline and descender, up at x-height, cap and ascender). Lowercase glyphs (by `index` group) use `x_height`; uppercase and figures use `cap_height`. With `glyph_extremes`, the glyph's own on-curve top and bottom also count as zones, which keeps figure, bracket and slash heights. |
| `counter_share` | 0.85 | How much of the growth goes into counters. Hole contours move `counter_share·amount` and outer contours move `(2 − counter_share)·amount`, so the total stem gain is unchanged but counters stay open. Measured v4.3: 0.9 for Bold, 0.8 for ExtraBold. 1.0 is a symmetric offset. Glyphs without holes are unaffected. |
| `gap` | 0 | Absolute minimum distance kept between facing edges. 0 = off. |
| `min_gap_ratio`, `gap_window` | 0.72, 60 | Gap guard: any two non-neighbour boundary samples closer than `gap_window` in the source keep at least `min_gap_ratio` of that distance. Measured v4.3: 72% kept H's inner-serif slits, the k foot slit, the 6 aperture and the b/p junctions open. 0 = off. |
| `corner` | `keep` | `keep` (alias `angle`) moves the corner point along its bisector by the amount, so the corner angle is kept. `miter` extends to the edge intersection, capped at `miter_limit × amount`. `round` in points mode is the same as `keep`. In clean mode it builds an arc, as does points mode with `add_points: true` (today's behaviour, not compatible). |
| `smooth_sigma` | 10 | Gaussian smoothing (σ in units of arc length) of the displacement field along each contour. 0 = off. Measured v4.3: σ=10 removed most serif-corner notches. |
| `post_smooth` | true | After moving, run the axis/tangent restore of §2.5 step 8, with the source as reference. |
| `sidebearing` | false | true: shift the outline right by h and grow the advance by 2h, so the gap to neighbouring glyphs stays the same. |
| `italic` | `auto` | §1.2. Stems are measured upright, so h acts on true stem width. |
| `round` | true | Round the result to whole units (points mode). |

### 2.5 Algorithm (points mode)

For each glyph in scope, starting from a copy:
1. **Frame.** Enter the italic frame if active (§1.2).
2. **Nesting.** Classify contours as outer or hole and find each one's grow side (§1.3). Ignore open contours (they are reported).
3. **Normals.**
   - On-point: unit bisector of the in and out tangent normals, pointing to the grow side.
   - Corner factor: `keep` = 1; `miter` = `1 / cos(θ/2)` capped at `miter_limit`, where θ is the turn.
   - Off-point: do not compute a normal. Offs are rebuilt in step 8.
4. **Raw displacement.** `d = (n.x·h, n.y·v) · factor · share`. In a glyph with at least one hole, share is `counter_share` for holes and `2 − counter_share` for outers. In a glyph without holes, share = 1.
5. **Smooth the field.** Gaussian-smooth `d` along arc length with σ=`smooth_sigma` (closed contours wrap). Then re-apply zone pinning: `d.y = 0` for zone points (step 4's zone rule). Points within 45 units inside a zone ramp linearly: `d.y` scales from 0 at the zone to full at 100 units. This is the v4.3 `zone_y` rule, measured to fix baseline drift and mixed serif heights.
6. **Collision cap.**
   - (a) For each moving on-point, cast a ray along `d` against the flattened boundary of all contours, both the source and the current proposal. Count only edges that face the point (edge outward normal · d < 0). If the hit edge also moves toward the point, the room is shared ½ each. Then `|d| ≤ max(0, hit − gap)`.
   - (b) Gap guard: for every pair of non-neighbour samples (more than 2 points apart on the same contour, or on different contours) with source distance `s < gap_window`, require `new ≥ min_gap_ratio·s`. If not, scale both displacements back proportionally. Iterate up to 3 passes.
7. **Apply.** Move the on-points.
8. **Restore.** This is the v4.3 `untangle`/`tidy` sequence, measured to fix notches and slanted serif ends:
   - (a) Axis runs: runs of points that share x (or y) within 0.5 in the source share the mean x (or y) of the moved points.
   - (b) Rebuild every off-curve in the chord frame of its two neighbouring on-points, using the source's (u, v) in that frame. A cubic's two handles each get their own (u, v). This is exact for an offset arc of equal angular span.
   - (c) If `post_smooth`, run the smooth pass of §3.4 with the source as reference and `axis_snap` on.
   - (d) Clamp each off-curve's projection onto its chord to [0.08, 0.92], which kills back-loops at junctions.
9. **Validate and fall back.** Run `outline_valid`. If it fails, retry the glyph with amounts × 0.93, up to 8 times (the v4.3 rule). If it still fails, keep the original glyph and add it to `fallback`. A reduced glyph is listed in `reduced` with its factor.
10. **Leave the frame.** Re-slant, round, and apply `sidebearing`.

Clean mode:
- Scale space by (1/h, 1/v), take a polygon offset of radius 1 on the flattened outline with the requested join (`round`, `miter` with limit, or `keep` = miter with limit 1.0), then scale back. Resolve self-overlaps with a union under the even-odd → non-zero rule.
- Re-fit: split at corners (turn > 30°), fit quadratics (or cubics, matching the source's segment type) with a tolerance of 0.5 units, and set `smooth` on joins under 6°.
- Zones: points within `overshoot` of a pinned zone snap to the source zone y.
- Report `compatible:false`.

### 2.6 Response

```json
{"ok":true,"data":{"changed":[…],"reduced":[{"name":"M","factor":0.93}],"fallback":[],"skipped_open":[],"compatible":true,
 "report":{"stems":{"H":[152,198]},"bars":{"H":[44,55]},"counters":{"o":[153,112]},"min_gap_kept":0.72}}}
```

`stems`, `bars` and `counters` are measured at mid-height on H, n and o when present: before → after. Use the ray-cast measure, not today's stem clustering (see §6).

### 2.7 Edge cases

- Open contours are not offset; they are listed in `skipped_open`.
- Contours with fewer than 3 points are skipped.
- Dots (a single convex contour that is round, area/bbox ≥ 0.7, and small, under 0.15 em): scale uniformly about the centroid by `1 + 2h/width` instead of offsetting. That keeps them round. Measured v4.3: 1.28 for Bold, 1.48 for ExtraBold, with i/j dots top-anchored.
- Thinning: a stroke can never go below `max(4, 0.25·source)` units (ray cast to the opposite edge). The point is clamped there and the glyph is reported.
- Mixed contour directions inside one glyph: handled by §1.3, never by assumption.
- `points` selection: only the selected on-points move. Their neighbour offs are rebuilt as in step 8b.
- A glyph with no contours is unchanged and listed in `unchanged`.

### 2.8 Compatibility guarantee

In points mode:
- point count, kinds, smooth flags, contour order and start points are unchanged;
- `check` against the source is compatible;
- `blend` source↔result at t = 0.25 / 0.5 / 0.75 passes `outline_valid`.

### 2.9 Tests (Vostok)

Fixtures: §5. "R46" = v4.6 Regular, "XB47" = v4.7 ExtraBold, and so on.

| # | Run | Expected |
|---|---|---|
| O1 | R46 H, h=23 v=5.5 | Stem 152 → 198 ±1 and crossbar 44 → 55 ±1 (equal to v4.3 Bold's measured 198/55). Baseline and cap points keep y. |
| O2 | R46 H, h=23 v=5.5 `gap:8` | Stem grows to ≥196. Today it stays at 152 (B1). |
| O3 | XB47 o, h=10 v=4 | Outer contour x-extent grows on both sides by 11.5 ±1 (share 1.15). The counter narrows by 8.5 ±1 per side. Ink area grows. Outer top/bottom y unchanged (−2.0 / 500.3). The outer contour is classed outer (B2 regression). |
| O4 | R46 o, h=23 v=5.5 | Outer x-extent 40..482 → ≈13.5..508.5 (today 63..459). Counter 184..337 → ≈204..317. |
| O5 | R46 whole font, h=23 v=5.5 | 0 glyphs fail `outline_valid` (today 19). `check` vs R46 is compatible. |
| O6 | R46 whole font, h ∈ {10, 20, 30} × v ∈ {4, 8} | 0 invalid, 0 fallback. Each run under 1 s in a release build. |
| O7 | R46 H, e, k, six, b, p at h=30 v=8 | Every inner-serif slit, e eye, k foot slit, 6 aperture and b/p junction keeps ≥72% of its source gap. |
| O8 | R46 period, colon, hyphen (clockwise contours) | They grow; they do not shrink. |
| O9 | R46 H, h=−10 | Stem 152 → 132 ±1. |
| O10 | v4.6 Italic H, `italic:auto`, h=20 | De-slanted stem 152 → 192 ±1. Advance unchanged. |
| O11 | R46 zero, h=20 | Outer top/bottom keep y (glyph_extremes). |
| O12 | Family of 8 Vostok styles, `family:true` | 8 undo steps; every style is still compatible with every other. |
| O13 | `preview:true` | `history` unchanged and font not dirty. Returned glyphs equal a real run's. |
| O14 | Real run then `undo` | Glyph JSON equals the input exactly. |
| O15 | `mode:"clean"`, R46 o | `compatible:false`, `outline_valid` clean, area within 2% of the points-mode result. |

### 2.10 Acceptance

- O1–O15 pass.
- The existing `offset.rs` tests still pass or are updated with a reason.
- `api.md` offset paragraph and MCP row are updated.

---

## 3. `smooth_outlines` (MCP `outline_smooth`) and `check_smoothness` (MCP `smoothness_check`)

### 3.1 Purpose

Remove tangent breaks ("waviness") and near-axis tilts while keeping weight, metrics and point structure.
- Measured on Vostok v4.6 → v4.7, as break count / total units:

  | Style | v4.6 | v4.7 |
  |---|---|---|
  | Regular | 161 / 429u | 0 |
  | Bold | 322 / 1333u | 11 / 52u |
  | ExtraBold | 397 / 2417u | 9 / 51u |

- Moves: up to 6u in Regular and up to 23u in ExtraBold. Ink area median change ≤ 0.37%. Heights change ≤ 2.6u.

### 3.2 Commands

```json
{"op":"check_smoothness"}
{"op":"check_smoothness","reference":"C:/fonts/Wide/Wide-Regular.json","names":["zero","six"],"details":true}
{"op":"smooth_outlines"}
{"op":"smooth_outlines","reference":"C:/fonts/Wide/Wide-Regular.json","family":true}
{"op":"smooth_outlines","names":["two"],"targets":[{"name":"two","contour":0,"points":[45,46],"value":45}],"preview":true}
```

`reference` is a file path, like `check` and `blend`, or `{"font":<id>}` for an open font. Omit it to use the glyph itself as its own reference (design-intent mode).

```json
{"name":"outline_smooth","inputSchema":{"type":"object","properties":{
 "reference":{"description":"Path, or {\"font\":id}. Omit = each glyph is its own reference."},
 "curve_threshold":{"type":"number","default":6},
 "line_threshold":{"type":"number","default":8},
 "keep_bend":{"type":"number","default":2.5},
 "axis_snap":{"type":"boolean","default":true},
 "axis_ref_max":{"type":"number","default":2.0},
 "axis_max":{"type":"number","default":10},
 "axis_max_tilted":{"type":"number","default":4},
 "axis_min_length":{"type":"number","default":25},
 "handle_axis":{"type":"number","default":3},
 "keep_heights":{"type":"boolean","default":true},
 "extrema_band":{"type":"number","default":12},
 "max_on_shift":{"type":"number","default":2},
 "protect_corners":{"type":"number","default":12},
 "targets":{"type":"array"},
 "set_smooth_flags":{"type":"boolean","default":false},
 "italic":{"type":"string","enum":["auto","true","false"],"default":"auto"},
 "round":{"type":"boolean","default":true},
 "names":{"type":"array","items":{"type":"string"}},"contours":{"type":"array"},
 "family":{"type":"boolean","default":false},"preview":{"type":"boolean","default":false}}}}
{"name":"smoothness_check","inputSchema":{"type":"object","properties":{
 "reference":{},"threshold":{"type":"number","default":1.5},
 "curve_threshold":{"type":"number","default":6},"line_threshold":{"type":"number","default":8},"keep_bend":{"type":"number","default":2.5},
 "names":{"type":"array","items":{"type":"string"}},"details":{"type":"boolean","default":false},"top":{"type":"integer","default":10}}}}
```

### 3.3 The metric (`check_smoothness`)

For each glyph, use the reference glyph with the same name (or the glyph itself). Reference and target must have the same structure; otherwise the glyph is listed under `incompatible`.

1. **Meant to be smooth.** A join is meant to be smooth when, in the reference, `|turn_ref| < curve_threshold` (6°) for curve–curve, or `< line_threshold` (8°) for a line join, and at least one side is a curve.
2. **Target.** The target turn is `turn_ref` if a line is involved and `|turn_ref| ≥ keep_bend` (2.5°), which keeps a deliberate bend. Otherwise it is 0.
3. **Deviation.** `deviation = min(handle_in, handle_out) · sin(|turn − target|)`. A line side counts as an infinite handle. This is the off-curve's distance from the ideal tangent line, in font units. It ignores huge angles on 1–6 unit handles, which are invisible.
4. **Break.** A join is a break when `deviation > threshold` (1.5).

Response:

```json
{"ok":true,"data":{"reference":"…","totals":{"breaks":322,"largest":14.0,"total":1333,"glyphs":75},
 "top":[{"name":"zero","breaks":10,"total":61.6},{"name":"six","total":52.3}],
 "glyphs":{"zero":{"breaks":10,"largest":8.5,"total":61.6,"joins":[{"contour":1,"point":9,"turn":12.5,"target":0,"deviation":6.1}]}},
 "incompatible":[]}}
```

`joins` appear only with `details`.

### 3.4 Algorithm (`smooth_outlines`)

This is the v4.7 production method. Per glyph and per closed contour, with R = the reference contour (or the input itself):
1. **Frame.** Enter the italic frame if active (§1.2). This is required for axis snap, because italic verticals are slanted.
2. **Axis snap** (`axis_snap`). For each line segment of length ≥ `axis_min_length`:
   - Condition: the reference line is within `axis_ref_max` (2°) of horizontal or vertical, the target line is between 0.05° and `axis_max` (10°) off that axis, and its endpoints differ by ≥ 1 unit. When the reference line itself is ≥ 1.5° off, the limit is `axis_max_tilted` (4°).
   - Target coordinate, in priority order:
     - (a) A `targets` override for that glyph, contour and point pair.
     - (b) The zone value (0, x-height, cap, ascender, descender) when a reference endpoint sits exactly on that zone **and** a target endpoint is within 2u of it.
     - (c) The coordinate of an anchored endpoint: one whose other neighbouring segment is a line ≥ 25 units long along the other axis, with slope ≤ 0.07.
     - (d) The rounded mean.
   - Reference lines 2–3° off an axis are deliberate (Vostok's `!` wedge) and are never snapped.
3. **Off-curve constraints.** For each join that is meant to be smooth, as in §3.3:
   - **Curve → line.** The curve's handle at the join must lie on the line through the on-point along the line's direction, rotated by the target bend.
   - **Curve–curve, reference tangent within `handle_axis` (3°) of horizontal or vertical.** Both handles lie on that axis line through the on-point.
   - **Curve–curve near an extreme** (`keep_heights`). This applies when the on-point is within `extrema_band` (12u) of its contour's top or bottom on-point y, and the free placement of step 5 would move it vertically by more than `max_on_shift` (2u).
     - Keep the on-point and rotate the handles instead.
     - If one handle is the overshoot control (beyond both of its own segment's on-points: above for a top, below for a bottom), it is the pivot and stays fixed. The other handle goes on the line through the on-point and the pivot.
     - Otherwise both handles go on the line through the on-point parallel to the current handle_b − handle_a.
     - Measured: without this rule, Vostok `three` lost 8u of height; with it, the sagging top control rises (+2.6u), matching the designer's hand fix.
4. **Solve each handle.**
   - One constraint: orthogonal projection.
   - Two: intersection of the two lines when their angle is > 15° and the intersection is within 60u. Otherwise the mean of the two projections.
   - When `round` is set, pick the floor/ceil combination that minimises the summed angle error, plus 0.01 × distance.
   - Cubic: each handle belongs to one join, so it has at most one constraint (simpler than quadratics, where one off is shared by two joins).
5. **Free smooth on-points.** These are the remaining meant-to-be-smooth curve–curve on-points not handled in step 3. Place each on the segment handle_a → handle_b at the reference ratio, clamped to [0.15, 0.85], rounding to minimise the turn.
6. **Corner protection** (`protect_corners`, 12°). For every join that is *not* meant to be smooth, if its turn changed by more than 12° from the input, revert the single point whose revert best restores it. Repeat until that join is within 12° (at most 3 passes). Points moved by axis snap are exempt.
7. **Validate.** If `outline_valid` fails, retry the glyph without axis snap. If it still fails, keep the original and add it to `fallback`.
8. **Leave the frame.** Re-slant and round.
9. **Smooth flags.** If `set_smooth_flags` is true, set `smooth:true` on the joins made smooth. Blending compares smooth flags, so this is only allowed with `family:true`, applied to the same joins in every style (the union across styles). Otherwise the command refuses.

Report: `check_smoothness` totals before and after, plus counts:

```json
{"ok":true,"data":{"changed":[…],"fallback":[],"report":{"before":{"breaks":322,"largest":14.0,"total":1333},"after":{"breaks":11,"largest":8.0,"total":52},"axis_snapped":182,"handles_moved":…,"on_points_moved":…,"corners_reverted":4,"max_move":21}}}
```

### 3.5 Edge cases

- **Reference with a different structure:** refuse that glyph (report it under `incompatible`) and leave it unchanged.
- **Italic styles:** smooth in the de-slanted frame. For a slanted family where all italics are derived by shear (as in Vostok), smoothing the upright and re-deriving the italic gives identical results. The italic's own residual breaks then come from integer rounding after the shear: Vostok shows 30–52 per italic, all ≤ 11.6u.
- **Very short handles (< 2u):** constraints still apply, but no metric break is counted (the deviation is tiny anyway).
- **Contours with one segment, open contours:** skipped.
- **Idempotence:** a second run changes no point by more than 1 unit, and `after` is not worse.

### 3.6 Compatibility guarantee

- Only coordinates change; points are never added or removed.
- Smooth flags are untouched unless `set_smooth_flags` is used with `family:true`.
- Advances are unchanged.

### 3.7 Tests (Vostok)

| # | Run | Expected |
|---|---|---|
| S1 | `check_smoothness` R46 (self) | 161 breaks, largest 8.8, total 429, 46 glyphs. |
| S2 | `check_smoothness` v4.6 Bold, reference R46 | 322 / 14.0 / 1333 / 75 glyphs. Top: zero 61.6, six 52.3, Q 45.0. |
| S3 | `check_smoothness` v4.6 ExtraBold, reference R46 | 397 / 24.9 / 2417. Top: zero 104.6, six 103.0. |
| S4 | `smooth_outlines` v4.6 Bold, reference R46 | Breaks ≤ 12, largest ≤ 8.5. zero, six, nine and three each have 0 breaks. `outline_valid` clean. `check` vs R46 compatible. Every contour's top/bottom within 3u. Ink area median change < 0.5%. |
| S5 | Same for v4.6 ExtraBold | Breaks ≤ 10, largest ≤ 10.5. Residuals only at protected corners (four, D, M). |
| S6 | `smooth_outlines` R46 (self), targets two(0,[45,46]) → 45 and five(0,[58,59]) → 181 | 0 breaks. Matches v4.7 Regular JSON at ≥ 98% of points exactly and everywhere within 1u. The `two` base p45–p46 is horizontal. |
| S7 | S4 again, with reference = v4.7 Regular | Within 1u of v4.7 Bold everywhere. |
| S8 | R46 `exclam` (wedge sides 2.96° off vertical) | Not snapped. |
| S9 | v4.6 Bold `three` | Top arch on-points keep y (696/698). The middle control rises. The curve top stays within 3u of v4.6. |
| S10 | Run S4's result again | No point moves more than 1u. |
| S11 | v4.6 BoldItalic, `italic:auto` | Result equals (S4 result sheared 10° about y=250, rounded) within 1u. |
| S12 | `set_smooth_flags:true` without `family` | `ok:false`, font unchanged. |
| S13 | `preview`, then `undo` after a real run | Same rules as O13/O14. |

### 3.8 Acceptance

- S1–S3 match exactly. The metric is deterministic and must reproduce the published numbers.
- S4–S13 pass.
- A ≤ 100-glyph font smooths in under 1 s (release build).

---

## 4. Symmetry operations

### 4.1 Purpose

Mirror, symmetrize, measure symmetry, and derive mirror-pair glyphs while keeping families blendable.

Measured facts on Vostok v4.7 (Hausdorff distance between outlines, in units) that shaped this design:
- **Pairs do not share point structure.** `(` has 26 points against `)` 22, `[` 22 against `]` 24, `{` 42 against `}` 33, and `six` 40+18 against `nine` 43+18. Any pair-matching or symmetrize logic that needs identical structure fails on real fonts. So symmetrize has a structure-preserving fit mode.
- **Mirrored pairs** (mirrored `(` against the existing `)`):

  | Pair | Regular | Bold | ExtraBold |
  |---|---|---|---|
  | `(` / `)` | 4.9 | 7.6 | 19.4 |
  | `[` / `]` | 3.9 ink-centred, 12.5 advance-centred | — | — |
  | `{` / `}` | 6.4 | — | — |
  | `<` / `>` | 2.5 | — | — |

  `six` rotated 180° against `nine`: 13.2.
- **Self-symmetry** (best vertical axis):

  | Glyph | Regular | ExtraBold |
  |---|---|---|
  | o | 2.5 | 16.2, axis 8u off centre |
  | O | 3.5 | 12.7 |
  | H | 5.0 | 19.0 |

  A, V, v, x and M are 44–102 in both, because of designed thick/thin diagonals. `/` against `\` differ by 41u and 53% area: a different design, not a mirror.

### 4.2 `mirror` (MCP `glyph_mirror`)

```json
{"op":"mirror","names":["parenleft"],"axis":"vertical"}
{"op":"mirror","names":["a"],"axis":{"type":"vertical","x":250}}
{"op":"mirror","names":["a"],"axis":{"type":"horizontal","y":250}}
{"op":"mirror","names":["a"],"axis":{"type":"line","from":[100,0],"to":[300,700]}}
{"op":"mirror","names":["a"],"axis":{"type":"angle","degrees":80,"through":[250,250]}}
{"op":"mirror","names":["a"],"contours":[1],"axis":"vertical","center":"ink"}
{"op":"mirror","names":["a"],"points":[[0,3],[0,4],[0,5]],"axis":{"type":"vertical","x":250}}
```

| Field | Default | Meaning |
|---|---|---|
| `axis` | `"vertical"` | `"vertical"` / `"horizontal"` / `{type:"vertical",x}` / `{type:"horizontal",y}` / `{type:"line",from,to}` / `{type:"angle",degrees,through}`. |
| `center` | `"advance"` | For a bare `"vertical"` or `"horizontal"`: `advance` = x at advance/2 (y = (ascender+descender)/2 for horizontal); `ink` = bounding-box centre of the scope; `origin` = 0. |
| `keep_direction` | true | A reflection flips winding. Reverse each fully mirrored contour with `reverse_contour` semantics (first point stays first), so direction and start point stay consistent. |
| `italic` | `auto` | Mirror in the de-slanted frame (§1.2). A vertical axis then means the slanted stem axis through the centre at `pivot_y` (250 for Vostok, consistent with v4.1). |
| `advance` | `"keep"` | `keep`, or `ink` (re-centre ink in the advance after mirroring). |
| `round` | true | |

Rules:
- A **point selection** reflects the selected positions only. No reordering, no reversal. This is meant for local shape edits.
- **Contour or glyph** scope: reflect, then reverse if `keep_direction`.
- **Compatibility:** the same `mirror` applied with `family:true` gives the same new structure in every style, so the family stays compatible. Applied to one style only, the response warns `"compatible_with_family":false` when the start point or direction changed relative to the other open styles.

### 4.3 `symmetrize` (MCP `glyph_symmetrize`)

Make a glyph exactly symmetric by copying one side, mirrored, onto the other, with **no change in point count or order**.

```json
{"op":"symmetrize","names":["o","O","zero"],"source":"left"}
{"op":"symmetrize","names":["H"],"axis":{"type":"vertical","x":350},"source":"average","tolerance":12}
{"op":"symmetrize","names":["o"],"family":true,"italic":"auto"}
```

| Field | Default | Meaning |
|---|---|---|
| `axis` | best fit | Default: the vertical axis minimising Hausdorff distance (search ±40u around the ink centre, 0.25u steps). An explicit axis takes the `mirror` forms. `center:"advance"` forces advance/2. |
| `source` | `"auto"` | `left`, `right`, `top`, `bottom`, `average`, or `auto`. `auto` takes the side with fewer `check_smoothness` breaks, and the left side on a tie. |
| `mode` | `"auto"` | `pairs` needs a structural pairing (step 1). `fit` works for any structure (step 2). `auto` tries `pairs`, then `fit`. |
| `tolerance` | 20 | Refuse a glyph whose initial deviation exceeds this (it is not near-symmetric, e.g. Vostok A/V/M at 44–102). |
| `max_fit_error` | 1.0 | Fit mode: the largest allowed distance between the fitted segment and the mirrored source curve. |
| `italic` | `auto` | §1.2. |

Algorithm:
1. **Pairs mode.**
   - For each contour, find its partner: itself, or the contour whose mirrored centroid is nearest.
   - Find the index shift k that pairs point i with partner point (k − i) mod n after reflection, with matching kinds and the smallest maximum distance.
   - If that maximum is ≤ `tolerance`, set each target-side point to the reflection of its source-side partner. With `average`, both get the mean of p and reflect(q).
   - Points paired with themselves (on the axis) snap onto the axis.
2. **Fit mode** (structure differs, the common case).
   - Build the mirrored source half: the outline of the source side clipped at the axis, then reflected.
   - Move each target-side on-point to its nearest point on the mirrored curve, then refit each target-side segment's off-curve(s) to the mirrored curve between its two on-points. Quadratic: least-squares single control point, with the end tangents kept when the join is meant to be smooth. Cubic: handle lengths along fixed tangents.
   - On-points within 1u of the axis snap onto it.
   - If any segment's fit error exceeds `max_fit_error`, revert that glyph and report it.
3. Run `outline_valid` and fall back per glyph.

The response gives, per glyph: mode used, axis, deviation before and after, and `max_fit_error`.

### 4.4 `check_symmetry` (MCP `symmetry_check`)

```json
{"op":"check_symmetry","names":["o","O","zero","H","A"]}
{"op":"check_symmetry","pairs":[["parenleft","parenright"],["bracketleft","bracketright"]],"center":"ink"}
```

Returns, per glyph:
- `axis_x` (best fit) and `axis_offset` (axis_x − advance/2);
- `hausdorff` (glyph against its own mirror about axis_x);
- `mean`;
- `structural` (true when pairs mode applies), with `worst` points `[{contour, point, partner, dev}]` when structural;
- `symmetric`: hausdorff ≤ `tolerance`, default 8.

For `pairs`: the Hausdorff distance and area difference (%) of mirror(a) against b, advance- or ink-centred. With `italic:auto` everything is measured in the de-slanted frame.

```json
{"ok":true,"data":{"glyphs":{"o":{"axis_x":261.0,"axis_offset":0.2,"hausdorff":2.5,"mean":0.9,"structural":false,"symmetric":true}},
 "pairs":[{"a":"parenleft","b":"parenright","hausdorff":4.9,"area_diff":1.1,"center":"advance"}]}}
```

### 4.5 `glyph_from_mirror` (MCP `glyph_from_mirror`)

Build a glyph from its mirror or rotation partner.

```json
{"op":"glyph_from_mirror","source":"parenleft","target":"parenright"}
{"op":"glyph_from_mirror","source":"six","target":"nine","transform":"rotate180"}
{"op":"glyph_from_mirror","preset":"brackets","family":true,"replace":true}
```

| Field | Default | Meaning |
|---|---|---|
| `source`, `target` | — | Glyph names. The target may exist (`replace` needed) or be new (`unicode` is then required, unless the target name is a standard AGL name). |
| `transform` | `"mirror_x"` | `mirror_x` (vertical axis), `mirror_y` (horizontal axis), or `rotate180` (about the ink-box centre, then placed so the ink box is centred where the target's was, or in the advance for a new glyph). |
| `center` | `"advance"` | Placement for mirror_x: `advance` (reflect about advance/2) or `ink` (reflect, then align to the existing target's ink centre). Measured: ink centring brings Vostok `[`→`]` from 12.5 to 3.9. |
| `advance` | `"source"` | `source`, or `target` (keep the existing target's advance). |
| `replace` | false | Overwrite an existing target. Without it the command refuses (`ok:false`, nothing changed). |
| `preset` | none | `"brackets"` runs `(`→`)`, `[`→`]`, `{`→`}`, `<`→`>`. `"figures"` runs `six`→`nine` (rotate180, a starting point for the designer). Unknown names in a preset are skipped and reported. |
| `italic` | `auto` | §1.2, so an italic `)` slants the same way as `(`. |
| `family` | false | Same op per style. The new structure is identical in every style, so the family stays compatible. |

Rules:
- `mirror_x` and `mirror_y` reverse contours (a reflection flips winding). `rotate180` does not, because a rotation keeps winding.
- Contour order and start points follow the source, so a family-wide run is compatible.
- Kerning is not copied. The report lists the target's existing kerning pairs, which may now be wrong.

### 4.6 Edge cases (all symmetry ops)

- **Open contours:** mirrored, never reversed.
- **Components:** none in the model (flattened on import).
- **Axis outside the glyph:** allowed for `mirror`. `symmetrize` refuses when no contour crosses or touches the axis.
- **Rounding:** reflecting about a half-unit axis keeps integer coordinates. About any other axis, round when `round` is set and report the largest rounding error.
- **Mixed directions inside a glyph:** each contour is reversed independently; this never depends on the font's convention.
- **Single style with `replace` on a glyph whose structure changes:** warn `compatible_with_family:false` and list the styles that no longer match.

### 4.7 Tests (Vostok v4.7)

| # | Run | Expected |
|---|---|---|
| Y1 | `check_symmetry` Regular o, O, zero, H | Hausdorff 2.5 / 3.5 / 3.9 / 5.0 (±0.3). All symmetric at tolerance 8. |
| Y2 | `check_symmetry` ExtraBold o | Hausdorff 16.2 ±0.5, axis_offset ≈ −8, not symmetric (bold derivation asymmetry). |
| Y3 | `symmetrize` ExtraBold o, source left, mode auto (fit) | Afterwards Hausdorff ≤ 2. Point count unchanged. `check` vs Regular compatible. Ink area change < 1%. |
| Y4 | `symmetrize` Regular A | Refused (deviation ~90 > 20). A unchanged. |
| Y5 | `glyph_from_mirror` Regular `(`→`)`, replace, center advance | Hausdorff to the old `)` ≤ 6 and area diff ≤ 1.5%. New `)` has 26 points. `check` against a family where every style got the same op is compatible. |
| Y6 | `glyph_from_mirror` Regular `[`→`]`, center ink | Hausdorff ≤ 5 (12.5 with center advance). |
| Y7 | Regular `{`→`}` and `<`→`>`, center ink | ≤ 7 and ≤ 3.5. |
| Y8 | `glyph_from_mirror` Regular six→nine, rotate180 | Hausdorff to the old nine ≤ 15 (measured 13.2). Winding unchanged (no reversal). |
| Y9 | `glyph_from_mirror` Italic `(`→`)`, italic auto | Hausdorff in the de-slanted frame ≤ upright result + 0.5 (measured 5.2 against 4.9). The new `)` slants right like the rest of the Italic. |
| Y10 | `mirror` Regular H twice (vertical, advance) | Identical to the input (direction and start point restored). |
| Y11 | `mirror` with `keep_direction:true` on a counter-clockwise contour | Still counter-clockwise, first point is the reflected first point. |
| Y12 | `preset:"brackets"`, `family:true`, replace, 8 Vostok styles | All 8 styles still mutually compatible (`check` Regular against each). 4 glyphs × 8 styles changed, 8 undo steps. |
| Y13 | `mirror` with a `points` selection | Point order unchanged, only those points move. |

### 4.8 Acceptance

- Y1–Y13 pass.
- `api.md` gets a "Symmetry" section.
- The MCP list includes `glyph_mirror`, `glyph_symmetrize`, `symmetry_check` and `glyph_from_mirror`, and the `tools/list` test asserts them.

---

## 5. Test fixtures and acceptance (all features)

- **Synthetic unit tests** (always run) go in each module, using small hand-built glyphs:
  - a square;
  - a ring (outer counter-clockwise plus a clockwise hole, and a variant with both clockwise, to test §1.3);
  - an H with serifs;
  - a quadratic O;
  - a cubic O;
  - a line+curve join with a 5° kink;
  - a 1.5°-tilted bar.
- **Vostok tests** are `#[ignore]` unless `TF_VOSTOK_DIR` is set. That folder holds the v4.6 and v4.7 JSON masters, read-only (on the dev machine: `E:/TypeFoundry-Fonts/work/v4.6` and `…/v4.7`). Tests must never write into that folder; write to a temp dir.
- **Golden numbers:** S1–S3 exact. Everything else within the stated tolerances.
- **Acceptance for the whole spec:**
  - `App\scripts\check.ps1` (fmt + test + clippy) is clean.
  - The new ops are available in `foundry run`, the CLI where relevant, and MCP.
  - `api.md` is updated.
  - Every new edit is undoable and refuses bad input without changing the font.
  - No new command writes files except through the existing `save`/`export` paths. Those keep the `force`/`.bak` rule.

Suggested file layout:
- `foundry-core/src/geometry.rs` (new)
- `foundry-core/src/offset.rs` (rework)
- `foundry-core/src/smooth.rs` (new)
- `foundry-core/src/symmetry.rs` (new)
- ops in `foundry-api/src/lib.rs`
- tool mapping and schemas in `foundry-mcp/src/lib.rs`

---

## 6. Fix list: other engine bugs found on Oct 8 (HEAD 1a7714d)

| # | Area | Problem | Fix |
|---|---|---|---|
| F1 | Release binaries | `typefoundry-target/release/foundry.exe` and `foundry-mcp.exe` are stale (built 10/04 21:29, 19 MCP tools, none of the new ops). Only the GUI is current. | Rebuild all three in the release script. `foundry --version` prints the git hash, and MCP `initialize` returns it in `serverInfo`. |
| F2 | Genome / `measure` / `scale_width` stem detection | Vertical edges are paired in sequence, including serif and off-curve edges. Measured: Regular genome stem 50.4 against a true 151. All stem critiques and `check_genome` are noise. | Measure stems by horizontal ray casts at 0.3/0.5/0.7 of x-height or cap height, through ink runs on flattened outlines. Take the median of the runs ≥ 0.5× the largest. |
| F3 | `scale_width` | Uses the same pairing: at 0.8 one H stem goes to 121 and the other stays 152. Advances become fractional. | Use F2 stems. Scale counters and sidebearings, keep stems, round advances. Test: Vostok H at 0.8 keeps both stems at 152 ±1. |
| F4 | `check_outlines` overshoot rule | Fires on any glyph whose curve top sits on a metric (A D J N V W X Y Z h k l u v w y ^ and a flat-topped `!`). | Only flag round tops and bottoms (an extremum on a curve with no straight run within 10u) that sit inside the zone instead of overshooting. |
| F5 | `check_outlines` crossings and direction | Crossings use the control polygon (false self-intersection on ExtraBold `a`). Direction is checked only on the largest contour, so colon is flagged in Regular but not ExtraBold, and the `?` dot is never checked. | Use `outline_valid` (§1.4). Check direction for every contour against nesting depth (§1.3). |
| F6 | `check_spacing` italic frame | Measures in the baseline-origin frame, which gives 85–94 false bearing flags per italic. Collision issues carry no gap value. The pair test uses the control polygon (`f'`/`f"` flagged at 20+ true units). | Measure in the §1.2 frame (pivot x_height/2) on flattened outlines. Report `gap` in units per issue. |
| F7 | `slant` / `derive_style slant` | Re-centres ink per glyph: 54/94 Vostok glyphs move > 15u against the v4.1 italic, `fT` and `f'` overlap, L–T 191 → 85, t–i opens 96 → 122. | Add `pivot_y` (default x_height/2) and `recenter` (default false). Shear about pivot_y and keep advances. Test: v4.7 Regular slanted 10° equals v4.7 Italic within 1u (apart from the −12 quote nudge). |
| F8 | `proof` PNG | Fills each contour separately, so every counter renders solid. The PDF is correct. | Fill with the non-zero (or even-odd) rule over all contours of a glyph in one path. |
| F9 | `resolve_critique` (MCP `critique_resolve`) | Overwrites the caller's `observation`/`intervention` with the suggestion's. Rejected suggestions reappear on the next `critique`. | Caller fields win when given. A rejected `id` is suppressed until the glyph changes (store a glyph hash with the decision). |
| F10 | `diff` | Ignores kerning and info: v4.5 against kerned v4.6 reports 0 changes. | Add kerning pairs, groups, features and info fields to the diff output. |
| F11 | `copy_family` | Keeps the old family file name and writes mixed `/` and `\` paths. | Name the file after the new family and always write `/`. |
| F12 | Legacy names for width | A Compressed style exports ID1/ID2 the same as Regular, and `family_check` only warns. Production post-patches ID1 to "Family Compressed" with an external script. | When width ≠ 5, put the width name into the legacy family (ID1 "Family Compressed"), as non-RIBBI weights already do. |
| F13 | `open_family` | On an already-open family it adds duplicate members, so `family_check`/export see duplicate file names. | Reuse open fonts with the same path. |
