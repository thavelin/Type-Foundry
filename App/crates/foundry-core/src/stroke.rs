//! Persistent centerline strokes. Variable expansion and fitting reuse VectorCraft.
use crate::{Contour, Font, FoundryError, Glyph, Point, PointKind};
use kurbo::{BezPath, ParamCurve, ParamCurveExtrema, PathSeg, Shape, Vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_geom::{ArcPath, FillRule, PathData};
#[path = "stroke_width.rs"]
mod width;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StrokeCap {
    #[default]
    Butt,
    Round,
    Square,
    Angled,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StrokeJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WidthEase {
    #[default]
    Linear,
    Smooth,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidthPoint {
    pub t: f64,
    pub left: f64,
    pub right: f64,
    #[serde(default)]
    pub angle: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JoinThinning {
    pub percent: f64,
    pub distance: f64,
}
impl Default for JoinThinning {
    fn default() -> Self {
        Self {
            percent: 15.0,
            distance: 60.0,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stroke {
    pub centerline: Contour,
    pub widths: Vec<WidthPoint>,
    pub ease: WidthEase,
    pub cap: StrokeCap,
    pub cap_angle: f64,
    pub join: StrokeJoin,
    pub miter_limit: f64,
    /// None = font units. Some(name) = fractions of lib.stroke_stems[name].
    pub stem: Option<String>,
    pub join_thinning: Option<JoinThinning>,
}
impl Default for Stroke {
    fn default() -> Self {
        Self {
            centerline: Contour {
                closed: false,
                points: vec![],
            },
            widths: vec![
                WidthPoint {
                    t: 0.0,
                    left: 40.0,
                    right: 40.0,
                    angle: None,
                },
                WidthPoint {
                    t: 1.0,
                    left: 40.0,
                    right: 40.0,
                    angle: None,
                },
            ],
            ease: WidthEase::Linear,
            cap: StrokeCap::Butt,
            cap_angle: 0.0,
            join: StrokeJoin::Miter,
            miter_limit: 4.0,
            stem: None,
            join_thinning: None,
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StrokeOptions {
    #[serde(default = "tolerance")]
    pub tolerance: f64,
    pub fixed_structure: bool,
    pub keep_strokes: bool,
}
fn tolerance() -> f64 {
    0.5
}
impl Default for StrokeOptions {
    fn default() -> Self {
        Self {
            tolerance: 0.5,
            fixed_structure: false,
            keep_strokes: false,
        }
    }
}
impl StrokeOptions {
    pub fn standard() -> Self {
        Self {
            tolerance: 0.5,
            ..Self::default()
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokePreset {
    TaperStart,
    TaperEnd,
    TaperBoth,
    Swell,
    Pinch,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Preset {
    pub kind: StrokePreset,
    pub amount: f64,
    #[serde(default = "middle")]
    pub t: f64,
}
fn middle() -> f64 {
    0.5
}
fn err(message: &str) -> FoundryError {
    FoundryError::Edit(message.into())
}

pub fn validate_stroke(s: &Stroke) -> Result<(), FoundryError> {
    if s.centerline.points.len() < 2 || s.widths.is_empty() {
        return Err(err("stroke needs a centerline and width points"));
    }
    if !s.miter_limit.is_finite()
        || s.miter_limit < 1.0
        || !s.cap_angle.is_finite()
        || s.cap_angle.abs() >= 89.0
    {
        return Err(err(
            "invalid miter limit or cap angle (must be between -89 and 89 degrees)",
        ));
    }
    let mut last = -1.0;
    for w in &s.widths {
        if !w.t.is_finite()
            || !(0.0..=1.0).contains(&w.t)
            || w.t <= last
            || !w.left.is_finite()
            || !w.right.is_finite()
            || w.left < 0.0
            || w.right < 0.0
            || w.angle.is_some_and(|a| !a.is_finite())
        {
            return Err(err(
                "width points must be finite, nonnegative and strictly ordered at t in 0..1",
            ));
        }
        last = w.t;
    }
    if let Some(j) = &s.join_thinning
        && (!j.percent.is_finite()
            || !(0.0..=100.0).contains(&j.percent)
            || !j.distance.is_finite()
            || j.distance <= 0.0)
    {
        return Err(err(
            "join thinning needs percent 0..100 and positive distance",
        ));
    }
    let path = centerline_path(&s.centerline)?;
    if ArcPath::new(&PathData::from_bezpath(&path)).len() < 1e-6 {
        return Err(err("zero length stroke"));
    }
    if s.centerline.closed {
        let a = sample_width(s, 0.0);
        let b = sample_width(s, 1.0);
        if (a.left - b.left).abs() > 1e-8 || (a.right - b.right).abs() > 1e-8 || a.angle != b.angle
        {
            return Err(err(
                "closed stroke widths and nib angles must match at the seam",
            ));
        }
    }
    Ok(())
}
fn centerline_path(c: &Contour) -> Result<BezPath, FoundryError> {
    use kurbo::Point as KPoint;
    if c.points.is_empty()
        || c.points[0].kind != PointKind::On
        || c.points
            .iter()
            .any(|p| !p.x.is_finite() || !p.y.is_finite())
    {
        return Err(err("centerline must start on-curve and have finite points"));
    }
    let kp = |p: &Point| KPoint::new(p.x, p.y);
    let mut bp = BezPath::new();
    bp.move_to(kp(&c.points[0]));
    let mut off = vec![];
    for p in c
        .points
        .iter()
        .skip(1)
        .chain(c.closed.then_some(&c.points[0]))
    {
        if p.kind == PointKind::Off {
            off.push(kp(p));
            continue;
        }
        match off.as_slice() {
            [] => bp.line_to(kp(p)),
            [a, b] => bp.curve_to(*a, *b, kp(p)),
            _ => {
                return Err(err(
                    "centerline segments must be lines or cubics (two handles)",
                ));
            }
        }
        off.clear();
    }
    if !off.is_empty() {
        return Err(err("open centerline has trailing handles"));
    }
    if c.closed {
        bp.close_path();
    }
    Ok(bp)
}
fn sample_width(s: &Stroke, t: f64) -> WidthPoint {
    let a = &s.widths[0];
    if t <= a.t {
        let mut a = a.clone();
        a.t = t;
        return a;
    }
    for w in s.widths.windows(2) {
        if t <= w[1].t {
            let mut u = (t - w[0].t) / (w[1].t - w[0].t);
            if s.ease == WidthEase::Smooth {
                u = u * u * (3.0 - 2.0 * u);
            }
            let lerp = |a: f64, b: f64| a + (b - a) * u;
            return WidthPoint {
                t,
                left: lerp(w[0].left, w[1].left),
                right: lerp(w[0].right, w[1].right),
                angle: match (w[0].angle, w[1].angle) {
                    (None, None) => None,
                    (a, b) => Some(lerp(
                        a.unwrap_or_else(|| b.unwrap_or(0.0)),
                        b.unwrap_or_else(|| a.unwrap_or(0.0)),
                    )),
                },
            };
        }
    }
    let mut a = s.widths.last().unwrap().clone();
    a.t = t;
    a
}
/// Adapter used by the reused VectorCraft width expander. Values are actual side widths.
struct SampledProfile {
    points: Vec<(f64, f64, f64)>,
}
impl SampledProfile {
    fn around(&self, t: f64) -> ((f64, f64), (f64, f64)) {
        let v = if t <= self.points[0].0 {
            (self.points[0].1, self.points[0].2)
        } else {
            self.points
                .windows(2)
                .find(|w| t <= w[1].0)
                .map(|w| {
                    let u = (t - w[0].0) / (w[1].0 - w[0].0);
                    (
                        w[0].1 + (w[1].1 - w[0].1) * u,
                        w[0].2 + (w[1].2 - w[0].2) * u,
                    )
                })
                .unwrap_or_else(|| {
                    let p = self.points.last().unwrap();
                    (p.1, p.2)
                })
        };
        (v, v)
    }
}
fn preset(s: &mut Stroke, p: &Preset) -> Result<(), FoundryError> {
    if !p.amount.is_finite() || !(0.0..=1.0).contains(&p.amount) || !(0.0..=1.0).contains(&p.t) {
        return Err(err("preset amount and t must be in 0..1"));
    }
    let mut ts: Vec<f64> = s.widths.iter().map(|w| w.t).chain([0.0, 1.0]).collect();
    match p.kind {
        StrokePreset::Swell | StrokePreset::TaperBoth => ts.push(0.5),
        StrokePreset::Pinch => ts.push(p.t),
        _ => {}
    }
    ts.sort_by(f64::total_cmp);
    ts.dedup();
    s.widths = ts
        .into_iter()
        .map(|t| {
            let mut w = sample_width(s, t);
            let f = match p.kind {
                StrokePreset::TaperStart => 1.0 - p.amount * (1.0 - t),
                StrokePreset::TaperEnd => 1.0 - p.amount * t,
                StrokePreset::TaperBoth => 1.0 - p.amount * (2.0 * t - 1.0).abs(),
                StrokePreset::Swell => 1.0 + p.amount * (1.0 - (2.0 * t - 1.0).abs()),
                StrokePreset::Pinch => {
                    1.0 - p.amount
                        * if t <= p.t {
                            t / p.t.max(1e-12)
                        } else {
                            (1.0 - t) / (1.0 - p.t).max(1e-12)
                        }
                }
            };
            w.left *= f;
            w.right *= f;
            w
        })
        .collect();
    Ok(())
}
pub fn stroke_counts(g: &Glyph) -> Value {
    json!({"name":g.name,"points":g.contours.iter().map(|c|c.points.len()).sum::<usize>(),"contours":g.contours.len(),"strokes":g.strokes.len(),"centerline_points":g.strokes.iter().map(|s|s.centerline.points.len()).sum::<usize>()})
}
#[allow(clippy::too_many_arguments)]
pub fn stroke_edit(
    font: &mut Font,
    name: &str,
    index: Option<usize>,
    op: &str,
    stroke: Option<Stroke>,
    points: Vec<WidthPoint>,
    from_t: Option<f64>,
    p: Option<Preset>,
) -> Result<Value, FoundryError> {
    let mut g = font
        .glyph(name)
        .ok_or_else(|| FoundryError::MissingGlyph(name.into()))?
        .clone();
    let i = index.unwrap_or(g.strokes.len());
    match op {
        "add" => {
            let s = stroke.ok_or_else(|| err("stroke_add needs stroke"))?;
            validate_stroke(&s)?;
            g.strokes.push(s);
        }
        "set" => {
            let s = stroke.ok_or_else(|| err("stroke_set needs stroke"))?;
            validate_stroke(&s)?;
            *g.strokes
                .get_mut(i)
                .ok_or_else(|| err("stroke index out of range"))? = s;
        }
        "remove" => {
            if i >= g.strokes.len() {
                return Err(err("stroke index out of range"));
            }
            g.strokes.remove(i);
        }
        "width" => {
            let s = g
                .strokes
                .get_mut(i)
                .ok_or_else(|| err("stroke index out of range"))?;
            if let Some(t) = from_t {
                let pos = s
                    .widths
                    .iter()
                    .position(|w| (w.t - t).abs() < 1e-9)
                    .ok_or_else(|| err("from_t width point not found"))?;
                s.widths.remove(pos);
            }
            for w in points {
                if let Some(pos) = s.widths.iter().position(|v| (v.t - w.t).abs() < 1e-9) {
                    s.widths[pos] = w;
                } else {
                    s.widths.push(w);
                }
            }
            s.widths.sort_by(|a, b| a.t.total_cmp(&b.t));
            if s.widths.is_empty() {
                return Err(err("width profile cannot be empty"));
            }
            if let Some(p) = p {
                preset(s, &p)?;
            }
            validate_stroke(s)?;
        }
        _ => return Err(err("unknown stroke edit")),
    }
    let mut report = stroke_counts(&g);
    report["stroke"] = json!(i);
    font.insert_glyph(g)?;
    Ok(report)
}
fn scale(font: &Font, s: &Stroke) -> Result<f64, FoundryError> {
    match &s.stem {
        None => Ok(1.0),
        Some(name) => font
            .lib
            .get("stroke_stems")
            .and_then(|v| v.get(name))
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or_else(|| {
                err("named stem missing: set lib.stroke_stems[name] to positive font units")
            }),
    }
}
fn constant(s: &Stroke) -> bool {
    s.widths
        .iter()
        .all(|w| w.left == s.widths[0].left && w.right == s.widths[0].right && w.angle.is_none())
}
fn uniform_outline(bp: &BezPath, s: &Stroke, k: f64, tol: f64) -> BezPath {
    let cap = match s.cap {
        StrokeCap::Butt | StrokeCap::Angled => kurbo::Cap::Butt,
        StrokeCap::Round => kurbo::Cap::Round,
        StrokeCap::Square => kurbo::Cap::Square,
    };
    let join = match s.join {
        StrokeJoin::Miter => kurbo::Join::Miter,
        StrokeJoin::Round => kurbo::Join::Round,
        StrokeJoin::Bevel => kurbo::Join::Bevel,
    };
    let style = kurbo::Stroke::new((s.widths[0].left + s.widths[0].right) * k)
        .with_caps(cap)
        .with_join(join)
        .with_miter_limit(s.miter_limit);
    let raw = kurbo::stroke(bp.iter(), &style, &kurbo::StrokeOpts::default(), tol);
    // kurbo may omit ClosePath on an open centerline outline.
    let mut raw = raw;
    if !matches!(raw.elements().last(), Some(kurbo::PathEl::ClosePath)) {
        raw.close_path();
    }
    raw
}
fn raw_outline(
    font: &Font,
    s: &Stroke,
    others: &[Stroke],
    tol: f64,
) -> Result<BezPath, FoundryError> {
    validate_stroke(s)?;
    let bp = centerline_path(&s.centerline)?;
    let k = scale(font, s)?;
    if constant(s)
        && s.widths[0].left == s.widths[0].right
        && s.join_thinning.is_none()
        && s.cap != StrokeCap::Angled
    {
        return Ok(uniform_outline(&bp, s, k, tol * 0.1));
    }
    let arc = ArcPath::new(&PathData::from_bezpath(&bp));
    let other_strokes = others;
    let others: Vec<ArcPath> = others
        .iter()
        .map(|s| centerline_path(&s.centerline).map(|p| ArcPath::new(&PathData::from_bezpath(&p))))
        .collect::<Result<_, _>>()?;
    let mut meetings = vec![];
    if let Some(j) = &s.join_thinning {
        for t in [0.0, 1.0] {
            let (p, _) = arc.at(t * arc.len());
            if others.iter().enumerate().any(|(i, a)| {
                a.nearest(p).is_some_and(|d| {
                    let other = &other_strokes[i];
                    let w = sample_width(other, d / a.len());
                    a.at(d).0.distance(p)
                        <= (w.left.max(w.right) * scale(font, other).unwrap_or(1.0)).max(1.0)
                })
            }) {
                meetings.push((t * arc.len(), j));
            }
        }
    }
    let n = ((arc.len() / tol.max(0.01)).sqrt().ceil() as usize * 8).clamp(64, 4096);
    let mut ts: Vec<f64> = (0..=n)
        .map(|i| i as f64 / n as f64)
        .chain(s.widths.iter().map(|w| w.t))
        .collect();
    if s.ease == WidthEase::Smooth || s.widths.iter().any(|w| w.angle.is_some()) {
        for pair in s.widths.windows(2) {
            let delta = (pair[1].left - pair[0].left)
                .abs()
                .max((pair[1].right - pair[0].right).abs())
                * k;
            let steps = ((delta * 6.0 / tol).sqrt().ceil() as usize).clamp(8, 4096);
            ts.extend(
                (1..steps).map(|i| pair[0].t + (pair[1].t - pair[0].t) * i as f64 / steps as f64),
            );
        }
    }
    ts.sort_by(f64::total_cmp);
    ts.dedup();
    let mut samples = vec![];
    let mut nib_left = vec![];
    let mut nib_right = vec![];
    let has_nib = s.widths.iter().any(|w| w.angle.is_some());
    for t in ts {
        let mut w = sample_width(s, t);
        let d = t * arc.len();
        for (at, j) in &meetings {
            let u = ((d - at).abs() / j.distance).min(1.0);
            let f = 1.0 - j.percent / 100.0 * (1.0 - u * u * (3.0 - 2.0 * u));
            w.left *= f;
            w.right *= f;
        }
        samples.push((t, w.left * k, w.right * k));
        if has_nib {
            let (p, v) = arc.at(d);
            let a = w
                .angle
                .unwrap_or_else(|| v.y.atan2(v.x).to_degrees() + 90.0)
                .to_radians();
            let v = Vec2::new(a.cos(), a.sin());
            nib_left.push(p + v * (w.left * k));
            nib_right.push(p - v * (w.right * k));
        }
    }
    let mut raw = if has_nib {
        let mut out = BezPath::new();
        if s.centerline.closed {
            out.move_to(nib_left[0]);
            for p in nib_left.iter().skip(1) {
                out.line_to(*p);
            }
            out.close_path();
            out.move_to(*nib_right.last().unwrap());
            for p in nib_right.iter().rev().skip(1) {
                out.line_to(*p);
            }
            out.close_path();
        } else {
            let mut ring = nib_left;
            let w = sample_width(s, 1.0);
            let (p, v) = arc.at(arc.len());
            let a = w
                .angle
                .unwrap_or(v.y.atan2(v.x).to_degrees() + 90.0)
                .to_radians();
            width::cap_points(
                &mut ring,
                p,
                Vec2::new(a.cos(), a.sin()),
                v,
                w.left * k,
                w.right * k,
                if s.cap == StrokeCap::Angled {
                    StrokeCap::Butt
                } else {
                    s.cap
                },
                tol * 0.05,
            );
            ring.extend(nib_right.iter().rev());
            let w = sample_width(s, 0.0);
            let (p, v) = arc.at(0.0);
            let a = w
                .angle
                .unwrap_or(v.y.atan2(v.x).to_degrees() + 90.0)
                .to_radians();
            width::cap_points(
                &mut ring,
                p,
                -Vec2::new(a.cos(), a.sin()),
                -v,
                w.right * k,
                w.left * k,
                if s.cap == StrokeCap::Angled {
                    StrokeCap::Butt
                } else {
                    s.cap
                },
                tol * 0.05,
            );
            out.move_to(ring[0]);
            for p in ring.iter().skip(1) {
                out.line_to(*p);
            }
            out.close_path();
        }
        out
    } else {
        let profile = SampledProfile { points: samples };
        let mut style = s.clone();
        if style.cap == StrokeCap::Angled {
            style.cap = StrokeCap::Butt;
        }
        width::width_outline(&bp, 2.0, &profile, &style, tol * 0.05)
    };
    if s.cap == StrokeCap::Angled && !s.centerline.closed {
        // Trim each side locally to the tilted cut, extending the outgoing side.
        // Moving just the cap vertices folds the dense side samples into little islands.
        use kurbo::PathEl;
        let mut ring: Vec<_> = raw
            .elements()
            .iter()
            .filter_map(|el| match el {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
                _ => None,
            })
            .collect();
        if ring.len() > 1 && ring[0].distance(*ring.last().unwrap()) < 1e-8 {
            ring.pop();
        }
        for t in [0., 1.] {
            let (p, v) = arc.at(t * arc.len());
            let normal = Vec2::new(-v.y, v.x);
            let w = sample_width(s, t);
            let angle = w
                .angle
                .unwrap_or(v.y.atan2(v.x).to_degrees() + 90.)
                .to_radians();
            let n = Vec2::new(angle.cos(), angle.sin());
            let targets = [p + n * (w.left * k), p - n * (w.right * k)];
            let ids: Vec<_> = targets
                .iter()
                .map(|q| {
                    (0..ring.len())
                        .min_by(|a, b| ring[*a].distance(*q).total_cmp(&ring[*b].distance(*q)))
                        .unwrap()
                })
                .collect();
            if ids[0] == ids[1] {
                return Err(err("angled cap has collapsed endpoints"));
            }
            let len = ring.len();
            if (ids[0] + 1) % len != ids[1] && (ids[1] + 1) % len != ids[0] {
                return Err(err("cannot identify angled terminal cross-section"));
            }
            let slope = s.cap_angle.to_radians().tan();
            let inward = if t == 0. { v } else { -v };
            let plane = (v - normal * slope) * if t == 0. { 1. } else { -1. };
            let extension = (w.left.max(w.right) * k * (slope.abs() + 1.) + tol).max(tol);
            let mut drop = vec![false; len];
            let mut replacements = vec![];
            for side in 0..2 {
                let start = ids[side];
                let other = ids[1 - side];
                let step = if (start + 1) % len == other {
                    len - 1
                } else {
                    1
                };
                let mut before = targets[side] - inward * extension;
                let mut index = start;
                let mut hit = None;
                for _ in 0..len {
                    let q = if index == start {
                        targets[side]
                    } else {
                        ring[index]
                    };
                    let a = (before - p).dot(plane);
                    let b = (q - p).dot(plane);
                    if b >= -1e-9 {
                        hit = Some(if (b - a).abs() > 1e-12 {
                            before.lerp(q, (-a / (b - a)).clamp(0., 1.))
                        } else {
                            q
                        });
                        break;
                    }
                    drop[index] = true;
                    before = q;
                    index = (index + step) % len;
                    if index == other {
                        break;
                    }
                }
                replacements.push((
                    start,
                    hit.ok_or_else(|| err("angled cut consumes the entire stroke"))?,
                ));
            }
            ring = ring
                .into_iter()
                .enumerate()
                .filter_map(|(i, q)| {
                    replacements
                        .iter()
                        .find(|(at, _)| *at == i)
                        .map(|(_, p)| *p)
                        .or_else(|| (!drop[i]).then_some(q))
                })
                .collect();
        }
        raw = BezPath::new();
        raw.move_to(ring[0]);
        for p in &ring[1..] {
            raw.line_to(*p);
        }
        raw.close_path();
    }
    Ok(raw)
}

fn fixed_cubic_parts(c: kurbo::CubicBez) -> Vec<kurbo::CubicBez> {
    let mut cuts = vec![0., 1.];
    cuts.extend(
        c.extrema()
            .into_iter()
            .filter(|t| *t > 1e-8 && *t < 1. - 1e-8),
    );
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-8);
    while cuts.len() < 6 {
        let widest = (0..cuts.len() - 1)
            .max_by(|a, b| (cuts[*a + 1] - cuts[*a]).total_cmp(&(cuts[*b + 1] - cuts[*b])))
            .unwrap();
        cuts.insert(widest + 1, (cuts[widest] + cuts[widest + 1]) * 0.5);
    }
    cuts.windows(2).map(|w| c.subsegment(w[0]..w[1])).collect()
}
// Fixed mode uses source/profile intervals and a constant subdivision count across
// weights. Avoid reserving unused extrema slots: each one used to multiply points by five.
fn append_fixed_cubic(raw: &mut BezPath, c: kurbo::CubicBez) {
    for c in fixed_cubic_parts(c) {
        raw.curve_to(c.p1, c.p2, c.p3);
    }
}
/// Fixed-layout offset fitting. Width changes never change the segment count or kind.
fn fixed_outline(font: &Font, s: &Stroke, tol: f64) -> Result<Vec<Contour>, FoundryError> {
    use kurbo::{CubicBez, ParamCurveArclen, ParamCurveDeriv};
    validate_stroke(s)?;
    let bp = centerline_path(&s.centerline)?;
    let segs: Vec<_> = bp.segments().collect();
    if s.widths.iter().any(|w| w.angle.is_some())
        || s.join_thinning.is_some()
        || s.cap == StrokeCap::Angled
        || segs
            .windows(2)
            .any(|w| vectorcraft_geom::bez::kink(&w[0], &w[1]))
    {
        return Err(err(
            "fixed_structure requires a smooth centerline without nib angle, angled cap or join thinning",
        ));
    }
    if s.centerline.closed && vectorcraft_geom::bez::kink(segs.last().unwrap(), &segs[0]) {
        return Err(err("fixed_structure requires a smooth closed seam"));
    }
    let k = scale(font, s)?;
    let zero = |w: &WidthPoint| w.left == 0.0 && w.right == 0.0;
    if s.widths
        .iter()
        .any(|w| zero(w) && (s.centerline.closed || (w.t != 0.0 && w.t != 1.0)))
        || s.widths.iter().all(zero)
        || s.widths.windows(2).any(|w| zero(&w[0]) && zero(&w[1]))
    {
        return Err(err(
            "fixed_structure cannot expand a collapsed interior profile point or interval",
        ));
    }
    if s.widths
        .iter()
        .any(|w| !zero(w) && (w.left + w.right) * k <= 1e-8)
    {
        return Err(err("fixed_structure width is too small to expand reliably"));
    }
    let lengths: Vec<f64> = segs.iter().map(|seg| seg.arclen(1e-7)).collect();
    let total: f64 = lengths.iter().sum();
    let mut sides = [vec![], vec![]];
    let mut acc = 0.0;
    for (seg, length) in segs.iter().zip(lengths) {
        if length < 1e-8 {
            return Err(err("fixed_structure cannot expand a zero length segment"));
        }
        // These boundaries depend on source geometry and profile positions, never widths.
        let mut marks = vec![0.0, 1.0];
        marks.extend(seg.extrema());
        for w in &s.widths {
            let distance = w.t * total - acc;
            if distance > 1e-7 && distance < length - 1e-7 {
                marks.push(seg.inv_arclen(distance, 1e-8));
            }
        }
        marks.sort_by(f64::total_cmp);
        marks.dedup_by(|a, b| (*a - *b).abs() < 1e-8);
        for span in marks.windows(2) {
            let mid = (acc + seg.subsegment(0.0..(span[0] + span[1]) * 0.5).arclen(1e-8)) / total;
            // Select the profile interval once, giving linear knots distinct one-sided slopes.
            let profile = s.widths.windows(2).find(|w| mid > w[0].t && mid < w[1].t);
            let eval = |u: f64, side: usize| {
                let (p, d, dd) = match *seg {
                    PathSeg::Line(l) => (l.eval(u), l.p1 - l.p0, Vec2::ZERO),
                    PathSeg::Quad(q) => (
                        q.eval(u),
                        q.deriv().eval(u).to_vec2(),
                        q.deriv().deriv().eval(u).to_vec2(),
                    ),
                    PathSeg::Cubic(c) => (
                        c.eval(u),
                        c.deriv().eval(u).to_vec2(),
                        c.deriv().deriv().eval(u).to_vec2(),
                    ),
                };
                let speed = d.hypot();
                if speed < 1e-10 {
                    return None;
                }
                let t = ((acc + seg.subsegment(0.0..u).arclen(1e-8)) / total).clamp(0.0, 1.0);
                let value = sample_width(s, t);
                let (width, sign) = if side == 0 {
                    (value.left, 1.0)
                } else {
                    (value.right, -1.0)
                };
                let slope = profile.map_or(0.0, |pair| {
                    let delta = if side == 0 {
                        pair[1].left - pair[0].left
                    } else {
                        pair[1].right - pair[0].right
                    };
                    let dt = pair[1].t - pair[0].t;
                    let v = ((t - pair[0].t) / dt).clamp(0.0, 1.0);
                    delta / dt
                        * if s.ease == WidthEase::Smooth {
                            6.0 * v * (1.0 - v)
                        } else {
                            1.0
                        }
                });
                let w = width * k * sign;
                let n = Vec2::new(-d.y, d.x) / speed;
                let dv = dd / speed - d * (d.dot(dd) / speed.powi(3));
                let dn = Vec2::new(-dv.y, dv.x);
                let derivative = d + dn * w + n * (slope * k * sign * speed / total);
                if derivative.dot(d) <= 1e-10 {
                    return None;
                }
                let q = p + n * w;
                if !q.x.is_finite()
                    || !q.y.is_finite()
                    || !derivative.x.is_finite()
                    || !derivative.y.is_finite()
                {
                    return None;
                }
                Some((q, derivative))
            };
            let steps = if matches!(seg, PathSeg::Line(_)) {
                1
            } else {
                8
            };
            for (side, chain) in sides.iter_mut().enumerate() {
                for i in 0..steps {
                    let a = span[0] + (span[1] - span[0]) * i as f64 / steps as f64;
                    let b = span[0] + (span[1] - span[0]) * (i + 1) as f64 / steps as f64;
                    let (p0, d0) = eval(a, side).ok_or_else(|| {
                        err("fixed_structure offset collapses or has a stationary tangent")
                    })?;
                    let (p3, d1) = eval(b, side).ok_or_else(|| {
                        err("fixed_structure offset collapses or has a stationary tangent")
                    })?;
                    let c =
                        CubicBez::new(p0, p0 + d0 * ((b - a) / 3.0), p3 - d1 * ((b - a) / 3.0), p3);
                    for j in 1..32 {
                        let t = j as f64 / 32.0;
                        let (q, _) = eval(a + (b - a) * t, side).ok_or_else(|| {
                            err("fixed_structure offset collapses inside a fit interval")
                        })?;
                        if c.eval(t).distance(q) > tol * 0.5 {
                            return Err(err(
                                "fixed_structure fit exceeds tolerance; increase tolerance or simplify the centerline/profile",
                            ));
                        }
                    }
                    let straight = matches!(seg, PathSeg::Line(_)) && s.ease == WidthEase::Linear;
                    if straight {
                        chain.push(PathSeg::Line(kurbo::Line::new(p0, p3)));
                    } else if constant(s) && s.ease == WidthEase::Linear {
                        chain.push(PathSeg::Cubic(c));
                    } else {
                        chain.extend(fixed_cubic_parts(c).into_iter().map(PathSeg::Cubic));
                    }
                }
            }
        }
        acc += length;
    }
    let start_width = sample_width(s, 0.0);
    let end_width = sample_width(s, 1.0);
    let mut raw = BezPath::new();
    raw.move_to(sides[0][0].start());
    let append = |raw: &mut BezPath, seg: PathSeg| match seg {
        PathSeg::Line(l) => raw.line_to(l.p1),
        PathSeg::Cubic(c) => raw.curve_to(c.p1, c.p2, c.p3),
        PathSeg::Quad(q) => raw.quad_to(q.p1, q.p2),
    };
    for seg in &sides[0] {
        append(&mut raw, *seg);
    }
    let cap = |raw: &mut BezPath, p: kurbo::Point, v: Vec2, n: Vec2, a: f64, b: f64| {
        let r = (a + b) / 2.0;
        if r == 0.0 {
            return;
        }
        let center = p + n * ((a - b) / 2.0);
        let start = p + n * a;
        let end = p - n * b;
        match s.cap {
            StrokeCap::Round => {
                let m = center + v * r;
                let h = 0.5522847498307936 * r;
                append_fixed_cubic(raw, CubicBez::new(start, start + v * h, m + n * h, m));
                append_fixed_cubic(raw, CubicBez::new(m, m - n * h, end + v * h, end));
            }
            StrokeCap::Square => {
                raw.line_to(start + v * r);
                raw.line_to(end + v * r);
                raw.line_to(end);
            }
            _ => raw.line_to(end),
        }
    };
    if s.centerline.closed {
        raw.close_path();
        raw.move_to(sides[1].last().unwrap().end());
    } else {
        let seg = *segs.last().unwrap();
        let p = seg.end();
        let v = vectorcraft_geom::bez::tangent(&seg, 1.0);
        cap(
            &mut raw,
            p,
            v,
            Vec2::new(-v.y, v.x),
            end_width.left * k,
            end_width.right * k,
        );
    }
    for seg in sides[1].iter().rev() {
        append(&mut raw, seg.reverse());
    }
    if !s.centerline.closed {
        let seg = segs[0];
        let v = vectorcraft_geom::bez::tangent(&seg, 0.0);
        cap(
            &mut raw,
            seg.start(),
            -v,
            Vec2::new(v.y, -v.x),
            start_width.right * k,
            start_width.left * k,
        );
    }
    raw.close_path();
    Ok(outline_contours(&PathData::from_bezpath(&raw), false))
}
/// Split every cubic at its x/y extrema, then derive smooth flags from the actual tangents.
fn contours(path: &PathData) -> Vec<Contour> {
    outline_contours(path, true)
}
fn outline_contours(path: &PathData, split_extrema: bool) -> Vec<Contour> {
    use kurbo::PathEl;
    let mut out = vec![];
    for sp in &path.subpaths {
        let bp = PathData::new(vec![sp.clone()]).to_bezpath();
        let mut pts = vec![];
        if let Some(PathEl::MoveTo(p)) = bp.elements().first() {
            pts.push(point(*p, PointKind::On));
        }
        for seg in bp.segments() {
            let mut ts = vec![0.0];
            ts.extend(
                seg.extrema()
                    .into_iter()
                    .filter(|t| split_extrema && *t > 1e-8 && *t < 1.0 - 1e-8),
            );
            ts.push(1.0);
            ts.sort_by(f64::total_cmp);
            ts.dedup();
            for w in ts.windows(2) {
                let part = seg.subsegment(w[0]..w[1]);
                match part {
                    PathSeg::Line(l) => pts.push(point(l.p1, PointKind::On)),
                    PathSeg::Quad(q) => {
                        let c = q.raise();
                        pts.extend([
                            point(c.p1, PointKind::Off),
                            point(c.p2, PointKind::Off),
                            point(c.p3, PointKind::On),
                        ]);
                    }
                    PathSeg::Cubic(c) => pts.extend([
                        point(c.p1, PointKind::Off),
                        point(c.p2, PointKind::Off),
                        point(c.p3, PointKind::On),
                    ]),
                }
            }
        }
        if pts.len() > 1
            && (pts[0].x - pts.last().unwrap().x).abs() < 1e-7
            && (pts[0].y - pts.last().unwrap().y).abs() < 1e-7
        {
            pts.pop();
        }
        let n = pts.len();
        for i in 0..n {
            if pts[i].kind != PointKind::On {
                continue;
            }
            let p = kurbo::Point::new(pts[i].x, pts[i].y);
            let a = p - kurbo::Point::new(pts[(i + n - 1) % n].x, pts[(i + n - 1) % n].y);
            let b = kurbo::Point::new(pts[(i + 1) % n].x, pts[(i + 1) % n].y) - p;
            pts[i].smooth = a.hypot() > 1e-8
                && b.hypot() > 1e-8
                && a.dot(b) > 0.0
                && a.cross(b).abs() / (a.hypot() * b.hypot()) < 1e-6;
        }
        out.push(Contour {
            closed: true,
            points: pts,
        });
    }
    out
}
fn point(p: kurbo::Point, kind: PointKind) -> Point {
    Point {
        x: p.x,
        y: p.y,
        kind,
        smooth: false,
    }
}
/// Preview and expansion share exactly one geometry path; all glyphs stage before commit.
pub fn expand_strokes(
    font: &mut Font,
    names: Option<&[String]>,
    options: &StrokeOptions,
    preview: bool,
) -> Result<Value, FoundryError> {
    if !options.tolerance.is_finite() || options.tolerance <= 0.0 {
        return Err(err("stroke tolerance must be positive and finite"));
    }
    let names: Vec<String> = names
        .map(|v| v.to_vec())
        .unwrap_or_else(|| font.glyphs.iter().map(|g| g.name.clone()).collect());
    let mut staged = vec![];
    let mut reports = vec![];
    for name in names {
        let mut g = font
            .glyph(&name)
            .ok_or_else(|| FoundryError::MissingGlyph(name.clone()))?
            .clone();
        if g.strokes.is_empty() {
            reports.push(stroke_counts(&g));
            continue;
        }
        let merged = if options.fixed_structure {
            if g.strokes.len() != 1 || !g.contours.is_empty() {
                return Err(err(
                    "fixed_structure cannot guarantee union topology: use one stroke and no existing contours",
                ));
            }
            g.contours = fixed_outline(font, &g.strokes[0], options.tolerance)?;
            let mut bp = BezPath::new();
            for c in &g.contours {
                bp.extend(centerline_path(c)?.iter());
            }
            PathData::from_bezpath(&bp)
        } else {
            let mut parts = vec![];
            for (i, s) in g.strokes.iter().enumerate() {
                let others: Vec<Stroke> = g
                    .strokes
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, s)| s.clone())
                    .collect();
                parts.push(PathData::from_bezpath(&raw_outline(
                    font,
                    s,
                    &others,
                    options.tolerance,
                )?));
            }
            let mut existing = BezPath::new();
            for c in &g.contours {
                existing.extend(centerline_path(c)?.iter());
            }
            if !existing.elements().is_empty() {
                parts.push(PathData::from_bezpath(&existing));
            }
            let mut iter = parts.into_iter();
            let mut combined = iter.next().unwrap_or_default();
            // Fallible curve-preserving union: never silently return an empty outline on failure.
            combined = vectorcraft_pathops::try_normalize(&combined, FillRule::NonZero)
                .map_err(|_| err("stroke normalization failed"))?;
            for p in iter {
                combined = vectorcraft_pathops::try_boolean(
                    &combined,
                    FillRule::NonZero,
                    &p,
                    FillRule::NonZero,
                    vectorcraft_pathops::BoolOp::Union,
                    options.tolerance * 0.1,
                )
                .map_err(|_| err("stroke union failed"))?;
            }
            combined = vectorcraft_pathops::simplify(&combined, options.tolerance * 0.4);
            g.contours = contours(&combined);
            combined
        };
        // VectorCraft uses y-down winding. Normalize to TypeFoundry's y-up outer CCW convention.
        let nest = crate::geometry::contour_nesting(&g.contours);
        for (i, c) in g.contours.iter_mut().enumerate() {
            if (crate::geometry::signed_area_points(&c.points) > 0.0) != nest[i].1 {
                c.points.reverse();
                c.points.rotate_right(1);
            }
        }
        let issues = crate::outline_valid(&g, None);
        if !issues.is_empty() {
            return Err(err(&format!(
                "expanded outline failed validation: {}",
                issues[0].detail
            )));
        }
        let mut report = stroke_counts(&g);
        let bounds = merged.to_bezpath().bounding_box();
        let zone = g.unicode.and_then(char::from_u32).or_else(|| {
            let mut c = g.name.chars();
            let first = c.next()?;
            c.next().is_none().then_some(first)
        });
        let mut overshoot = serde_json::Map::new();
        let bottom = if zone.is_some_and(|c| "gjpqy".contains(c)) {
            ("descender", font.metrics.descender)
        } else {
            ("baseline", font.metrics.baseline)
        };
        overshoot.insert(bottom.0.into(), json!((bottom.1 - bounds.y0).max(0.)));
        let top = if zone.is_some_and(|c| c.is_uppercase() || c.is_ascii_digit()) {
            Some(("cap_height", font.metrics.cap_height))
        } else if zone.is_some_and(|c| "bdfhklt".contains(c)) {
            Some(("ascender", font.metrics.ascender))
        } else if zone.is_some_and(char::is_lowercase) {
            Some(("x_height", font.metrics.x_height))
        } else {
            None
        };
        if let Some((label, height)) = top {
            overshoot.insert(label.into(), json!((bounds.y1 - height).max(0.)));
        }
        report["overshoot"] = json!(overshoot);
        if options.fixed_structure {
            let stroke = &g.strokes[0];
            let collapsed = |t| {
                let w = sample_width(stroke, t);
                w.left == 0.0 && w.right == 0.0
            };
            report["structure_key"] = json!({"version":3,"centerline":stroke.centerline,"width_positions":stroke.widths.iter().map(|w|w.t).collect::<Vec<_>>(),"ease":stroke.ease,"cap":stroke.cap,"collapsed_caps":[collapsed(0.0),collapsed(1.0)],"curve_subdivisions":8,"extrema_slots":if constant(stroke) && stroke.ease == WidthEase::Linear {1}else{5},"compact_constant_profile":constant(stroke) && stroke.ease == WidthEase::Linear});
        }
        report["outlines"] = json!(g.contours);
        reports.push(report);
        if !options.keep_strokes {
            g.strokes.clear();
        }
        staged.push(g);
    }
    let count: usize = reports
        .iter()
        .map(|r| r["points"].as_u64().unwrap_or(0) as usize)
        .sum();
    let cs: usize = reports
        .iter()
        .map(|r| r["contours"].as_u64().unwrap_or(0) as usize)
        .sum();
    if !preview {
        for g in staged {
            font.insert_glyph(g)?;
        }
    }
    Ok(
        json!({"glyphs":reports,"points":count,"contours":cs,"fixed_structure":options.fixed_structure,"compatible":options.fixed_structure,"tolerance":options.tolerance,"preview":preview}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn line(a: (f64, f64), b: (f64, f64), w: f64) -> Stroke {
        Stroke {
            centerline: Contour {
                closed: false,
                points: vec![
                    point(a.into(), PointKind::On),
                    point(b.into(), PointKind::On),
                ],
            },
            widths: vec![
                WidthPoint {
                    t: 0.0,
                    left: w / 2.0,
                    right: w / 2.0,
                    angle: None,
                },
                WidthPoint {
                    t: 1.0,
                    left: w / 2.0,
                    right: w / 2.0,
                    angle: None,
                },
            ],
            ..Stroke::default()
        }
    }
    fn font(strokes: Vec<Stroke>) -> Font {
        let mut f = Font::new("Strokes", 1000).unwrap();
        f.insert_glyph(Glyph {
            name: "test".into(),
            unicode: None,
            advance: 700.0,
            contours: vec![],
            strokes,
        })
        .unwrap();
        f
    }
    fn expand(f: &mut Font) -> Value {
        expand_strokes(f, None, &StrokeOptions::default(), false).unwrap()
    }
    fn clean(f: &Font) {
        assert!(crate::outline_valid(&f.glyphs[0], None).is_empty());
        assert!(
            crate::check_outlines(f)
                .iter()
                .all(|i| !matches!(i.code, "intersection" | "kink" | "direction")),
            "{:?}",
            crate::check_outlines(f)
        );
        assert_eq!(
            crate::check_smoothness(f, None, None, 1.0, 6.0, 8.0, 2.5, true, 10)
                .unwrap()
                .totals
                .breaks,
            0
        );
    }
    #[test]
    fn fixed_line_is_four_points_and_reports_only_its_zone() {
        let mut f = font(vec![line((0., 0.), (0., 500.), 80.)]);
        f.glyphs[0].name = "o".into();
        f.glyphs[0].unicode = Some('o' as u32);
        let r = fixed(&mut f);
        assert_eq!(r["points"], 4);
        assert!(
            f.glyphs[0].contours[0]
                .points
                .iter()
                .all(|p| p.kind == PointKind::On)
        );
        assert!(r["glyphs"][0]["overshoot"].get("x_height").is_some());
        assert!(r["glyphs"][0]["overshoot"].get("cap_height").is_none());
    }
    #[test]
    fn angled_cap_applies_requested_angle_to_a_curved_stroke() {
        let mut s = line((400., 400.), (400., 100.), 86.);
        s.centerline
            .points
            .insert(1, point((100., 480.).into(), PointKind::Off));
        s.centerline
            .points
            .insert(2, point((100., 20.).into(), PointKind::Off));
        s.cap = StrokeCap::Angled;
        s.cap_angle = 28.;
        let f = font(vec![s.clone()]);
        let raw = raw_outline(&f, &s, &[], 0.5).unwrap();
        let arc = ArcPath::new(&PathData::from_bezpath(
            &centerline_path(&s.centerline).unwrap(),
        ));
        for t in [0., 1.] {
            let (p, v) = arc.at(t * arc.len());
            let n = Vec2::new(-v.y, v.x);
            let plane = v - n * 28_f64.to_radians().tan();
            let cuts: Vec<_> = raw
                .segments()
                .filter(|s| {
                    (s.start() - p).dot(plane).abs() < 1e-6 && (s.end() - p).dot(plane).abs() < 1e-6
                })
                .collect();
            assert!(!cuts.is_empty(), "missing angled terminal cut");
        }
        let mut f = font(vec![s]);
        expand(&mut f);
        assert_eq!(
            f.glyphs[0].contours.len(),
            1,
            "angled terminal must not create islands"
        );
        clean(&f);
    }
    #[test]
    fn straight_rectangle() {
        let mut f = font(vec![line((0.0, 0.0), (500.0, 0.0), 80.0)]);
        let r = expand(&mut f);
        assert_eq!(r["contours"], 1);
        assert_eq!(r["points"], 4);
        let bounds = crate::geometry::ink_bounds(&f.glyphs[0]);
        assert_eq!(bounds, (0.0, 500.0, -40.0, 40.0));
        clean(&f);
    }
    #[test]
    fn circle_ring() {
        let bp = kurbo::Circle::new((300.0, 300.0), 200.0).to_path(0.0001);
        let center = contours(&PathData::from_bezpath(&bp)).remove(0);
        let mut st = line((0.0, 0.0), (1.0, 0.0), 80.0);
        st.centerline = center;
        let mut f = font(vec![st]);
        expand(&mut f);
        assert_eq!(f.glyphs[0].contours.len(), 2);
        for c in &f.glyphs[0].contours {
            let b = centerline_path(c).unwrap().bounding_box();
            let r = (b.width() + b.height()) / 4.0;
            assert!(
                (r - 240.0).abs() < 0.5 || (r - 160.0).abs() < 0.5,
                "radius {r}"
            );
        }
        clean(&f);
    }
    #[test]
    fn taper_end_width() {
        let mut s = line((0.0, 0.0), (500.0, 0.0), 80.0);
        preset(
            &mut s,
            &Preset {
                kind: StrokePreset::TaperEnd,
                amount: 0.75,
                t: 0.5,
            },
        )
        .unwrap();
        assert_eq!(sample_width(&s, 1.0).left, 10.0);
        let mut f = font(vec![s]);
        expand(&mut f);
        let pts = &f.glyphs[0].contours[0].points;
        let at: Vec<_> = pts
            .iter()
            .filter(|p| (p.x - 500.0).abs() < 1e-5 && p.kind == PointKind::On)
            .collect();
        assert!(at.iter().any(|p| (p.y - 10.0).abs() < 0.1));
        assert!(at.iter().any(|p| (p.y + 10.0).abs() < 0.1));
        clean(&f);
    }
    #[test]
    fn crossing_strokes_union() {
        let mut f = font(vec![
            line((0.0, 0.0), (500.0, 500.0), 80.0),
            line((0.0, 500.0), (500.0, 0.0), 80.0),
        ]);
        let r = expand(&mut f);
        assert_eq!(r["contours"], 1);
        clean(&f);
    }
    #[test]
    fn fixed_weights_same_structure() {
        let mut counts = vec![];
        for w in [80.0, 120.0, 180.0] {
            let mut f = font(vec![line((0.0, 0.0), (500.0, 0.0), w)]);
            let options = StrokeOptions {
                fixed_structure: true,
                ..StrokeOptions::default()
            };
            let r = expand_strokes(&mut f, None, &options, false).unwrap();
            assert_eq!(r["compatible"], true);
            counts.push((
                r["points"].clone(),
                f.glyphs[0]
                    .contours
                    .iter()
                    .flat_map(|c| c.points.iter().map(|p| p.kind))
                    .collect::<Vec<_>>(),
            ));
            clean(&f);
        }
        assert!(counts.windows(2).all(|w| w[0] == w[1]));
    }
    #[test]
    fn fixed_curved_weights() {
        let mut counts = vec![];
        for w in [80.0, 120.0, 180.0] {
            let mut s = line((0.0, 0.0), (600.0, 0.0), w);
            s.centerline
                .points
                .insert(1, point((150.0, 300.0).into(), PointKind::Off));
            s.centerline
                .points
                .insert(2, point((450.0, 300.0).into(), PointKind::Off));
            let mut f = font(vec![s]);
            let r = expand_strokes(
                &mut f,
                None,
                &StrokeOptions {
                    fixed_structure: true,
                    ..StrokeOptions::default()
                },
                false,
            )
            .unwrap();
            counts.push(r["points"].clone());
            clean(&f);
        }
        assert!(counts.windows(2).all(|w| w[0] == w[1]));
    }
    #[test]
    fn preview_and_keep() {
        let mut f = font(vec![line((0.0, 0.0), (500.0, 0.0), 80.0)]);
        let before = f.clone();
        let r = expand_strokes(&mut f, None, &StrokeOptions::default(), true).unwrap();
        assert_eq!(f, before);
        let r2 = expand_strokes(
            &mut f,
            None,
            &StrokeOptions {
                keep_strokes: true,
                ..StrokeOptions::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(r["glyphs"], r2["glyphs"]);
        assert_eq!(f.glyphs[0].strokes.len(), 1);
    }
    #[test]
    fn invalid_edit_atomic() {
        let mut f = font(vec![line((0.0, 0.0), (500.0, 0.0), 80.0)]);
        let before = f.clone();
        assert!(
            stroke_edit(
                &mut f,
                "test",
                Some(0),
                "width",
                None,
                vec![WidthPoint {
                    t: 0.3,
                    left: -1.0,
                    right: 2.0,
                    angle: None
                }],
                None,
                None
            )
            .is_err()
        );
        assert_eq!(f, before);
    }
    #[test]
    fn legacy_and_stroke_json_roundtrip() {
        let f = font(vec![line((0.0, 0.0), (500.0, 0.0), 80.0)]);
        let raw = serde_json::to_string(&f).unwrap();
        let loaded: Font = serde_json::from_str(&raw).unwrap();
        assert_eq!(f, loaded);
        let mut raw = serde_json::to_value(f).unwrap();
        raw["glyphs"][0].as_object_mut().unwrap().remove("strokes");
        let old: Font = serde_json::from_value(raw).unwrap();
        assert!(old.glyphs[0].strokes.is_empty());
    }
    #[test]
    fn caps_and_joins() {
        for cap in [
            StrokeCap::Butt,
            StrokeCap::Round,
            StrokeCap::Square,
            StrokeCap::Angled,
        ] {
            for join in [StrokeJoin::Miter, StrokeJoin::Round, StrokeJoin::Bevel] {
                let mut s = line((0.0, 0.0), (500.0, 0.0), 80.0);
                s.cap = cap;
                s.join = join;
                s.cap_angle = 25.0;
                s.centerline
                    .points
                    .push(point((500.0, 500.0).into(), PointKind::On));
                let mut f = font(vec![s]);
                expand(&mut f);
                clean(&f);
            }
        }
    }
    #[test]
    fn named_stem_and_smooth_profile() {
        let mut s = line((0.0, 0.0), (500.0, 0.0), 1.0);
        s.stem = Some("main".into());
        s.ease = WidthEase::Smooth;
        let mut f = font(vec![s]);
        f.lib.insert("stroke_stems".into(), json!({"main":80.0}));
        expand(&mut f);
        assert_eq!(crate::geometry::ink_bounds(&f.glyphs[0]).3, 40.0);
    }
    #[test]
    fn all_presets() {
        for kind in [
            StrokePreset::TaperStart,
            StrokePreset::TaperEnd,
            StrokePreset::TaperBoth,
            StrokePreset::Swell,
            StrokePreset::Pinch,
        ] {
            let mut s = line((0.0, 0.0), (500.0, 0.0), 80.0);
            preset(
                &mut s,
                &Preset {
                    kind,
                    amount: 0.5,
                    t: 0.3,
                },
            )
            .unwrap();
            validate_stroke(&s).unwrap();
            let mut f = font(vec![s]);
            expand(&mut f);
            clean(&f);
        }
    }
    #[test]
    fn zero_tapers_and_nibs() {
        for amount in [0.0, 1.0] {
            let mut s = line((0.0, 0.0), (500.0, 0.0), 80.0);
            preset(
                &mut s,
                &Preset {
                    kind: StrokePreset::TaperBoth,
                    amount,
                    t: 0.5,
                },
            )
            .unwrap();
            let mut f = font(vec![s]);
            expand(&mut f);
            clean(&f);
        }
        for cap in [StrokeCap::Butt, StrokeCap::Round, StrokeCap::Square] {
            let mut s = line((0.0, 0.0), (500.0, 0.0), 80.0);
            s.cap = cap;
            for w in &mut s.widths {
                w.angle = Some(60.0);
            }
            let mut f = font(vec![s]);
            expand(&mut f);
            clean(&f);
        }
    }
    #[test]
    fn thinning_reduces_join_ink() {
        let stem = line((0.0, 0.0), (0.0, 500.0), 100.0);
        let mut arm = line((50.0, 250.0), (400.0, 250.0), 80.0);
        let f = font(vec![]);
        let full = raw_outline(&f, &arm, std::slice::from_ref(&stem), 0.5)
            .unwrap()
            .area()
            .abs();
        arm.join_thinning = Some(JoinThinning {
            percent: 50.0,
            distance: 100.0,
        });
        let thin = raw_outline(&f, &arm, std::slice::from_ref(&stem), 0.5)
            .unwrap()
            .area()
            .abs();
        assert!(thin < full - 100.0, "{thin} {full}");
        let mut f = font(vec![stem, arm]);
        expand(&mut f);
        clean(&f);
    }
    #[test]
    fn deterministic_preview() {
        let mut f = font(vec![line((0.0, 0.0), (500.0, 500.0), 80.0)]);
        let a = expand_strokes(&mut f, None, &StrokeOptions::default(), true).unwrap();
        let b = expand_strokes(&mut f, None, &StrokeOptions::default(), true).unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }
    #[test]
    fn existing_counter_stays_open() {
        let square = |a: f64, b: f64, reverse: bool| {
            let mut pts = vec![
                point((a, a).into(), PointKind::On),
                point((b, a).into(), PointKind::On),
                point((b, b).into(), PointKind::On),
                point((a, b).into(), PointKind::On),
            ];
            if reverse {
                pts.reverse();
            }
            Contour {
                closed: true,
                points: pts,
            }
        };
        let mut f = font(vec![line((600.0, 0.0), (600.0, 500.0), 80.0)]);
        f.glyphs[0].contours = vec![square(0.0, 500.0, false), square(100.0, 400.0, true)];
        expand(&mut f);
        assert_eq!(f.glyphs[0].contours.len(), 3);
        assert_eq!(
            crate::geometry::contour_nesting(&f.glyphs[0].contours)
                .iter()
                .filter(|v| !v.1)
                .count(),
            1
        );
        clean(&f);
    }
    #[test]
    fn fixed_closed_ring_weights_and_caps() {
        for cap in [StrokeCap::Butt, StrokeCap::Round, StrokeCap::Square] {
            let mut counts = vec![];
            for w in [80.0, 120.0, 180.0] {
                let mut st = line((0.0, 0.0), (500.0, 0.0), w);
                st.cap = cap;
                let mut f = font(vec![st]);
                let r = expand_strokes(
                    &mut f,
                    None,
                    &StrokeOptions {
                        fixed_structure: true,
                        ..StrokeOptions::default()
                    },
                    false,
                )
                .unwrap_or_else(|e| panic!("{cap:?} {w}: {e}"));
                counts.push(r["points"].clone());
                clean(&f);
            }
            assert!(counts.windows(2).all(|w| w[0] == w[1]));
        }
        let center = contours(&PathData::from_bezpath(
            &kurbo::Circle::new((300.0, 300.0), 200.0).to_path(0.0001),
        ))
        .remove(0);
        let mut counts = vec![];
        for w in [80.0, 120.0, 180.0] {
            let mut st = line((0.0, 0.0), (1.0, 0.0), w);
            st.centerline = center.clone();
            let mut f = font(vec![st]);
            let r = expand_strokes(
                &mut f,
                None,
                &StrokeOptions {
                    fixed_structure: true,
                    ..StrokeOptions::default()
                },
                false,
            )
            .unwrap();
            assert_eq!(r["contours"], 2);
            counts.push(r["points"].clone());
            clean(&f);
        }
        assert!(counts.windows(2).all(|w| w[0] == w[1]));
    }
    fn variable(width: f64, ease: WidthEase) -> Stroke {
        let mut s = line((0.0, 0.0), (600.0, 250.0), width);
        s.ease = ease;
        s.widths = vec![
            WidthPoint {
                t: 0.0,
                left: width * 0.2,
                right: width * 0.3,
                angle: None,
            },
            WidthPoint {
                t: 0.35,
                left: width * 0.7,
                right: width * 0.45,
                angle: None,
            },
            WidthPoint {
                t: 0.8,
                left: width * 0.4,
                right: width * 0.25,
                angle: None,
            },
            WidthPoint {
                t: 1.0,
                left: width * 0.15,
                right: width * 0.3,
                angle: None,
            },
        ];
        s
    }
    fn fixed(f: &mut Font) -> Value {
        expand_strokes(
            f,
            None,
            &StrokeOptions {
                fixed_structure: true,
                ..StrokeOptions::default()
            },
            false,
        )
        .unwrap()
    }
    fn assert_extrema_are_anchors(f: &Font) {
        for c in &f.glyphs[0].contours {
            for seg in centerline_path(c).unwrap().segments() {
                assert!(
                    seg.extrema()
                        .into_iter()
                        .all(|t| !(1e-6..1.0 - 1e-6).contains(&t)),
                    "unsplit extrema in {seg:?}"
                );
            }
        }
    }
    #[test]
    fn variable_fixed_weights_are_blendable() {
        for ease in [WidthEase::Linear, WidthEase::Smooth] {
            for cap in [StrokeCap::Butt, StrokeCap::Round, StrokeCap::Square] {
                let mut masters = vec![];
                let mut keys = vec![];
                for width in [80.0, 120.0, 180.0] {
                    let mut s = variable(width, ease);
                    s.cap = cap;
                    let mut f = font(vec![s]);
                    let r = fixed(&mut f);
                    assert_eq!(r["compatible"], true);
                    keys.push(r["glyphs"][0]["structure_key"].clone());
                    assert_extrema_are_anchors(&f);
                    clean(&f);
                    masters.push(f);
                }
                assert!(keys.windows(2).all(|w| w[0] == w[1]));
                for pair in masters.windows(2) {
                    assert!(crate::compatibility(&pair[0], &pair[1]).is_empty());
                    let blend = crate::blend_fonts(&pair[0], &pair[1], 0.5).unwrap();
                    clean(&blend);
                }
            }
        }
    }
    #[test]
    fn variable_fixed_curves_use_arc_length_knots() {
        let mut masters = vec![];
        let mut keys = vec![];
        for width in [80.0, 120.0, 180.0] {
            let mut s = variable(width, WidthEase::Smooth);
            s.centerline.points = vec![
                point((0.0, 0.0).into(), PointKind::On),
                point((100.0, 250.0).into(), PointKind::Off),
                point((400.0, 250.0).into(), PointKind::Off),
                point((700.0, 0.0).into(), PointKind::On),
            ];
            let arc = ArcPath::new(&PathData::from_bezpath(
                &centerline_path(&s.centerline).unwrap(),
            ));
            let w = s.widths[1].clone();
            let (p, v) = arc.at(arc.len() * w.t);
            let n = Vec2::new(-v.y, v.x);
            let expected = [p + n * w.left, p - n * w.right];
            let mut f = font(vec![s]);
            let r = fixed(&mut f);
            keys.push(r["glyphs"][0]["structure_key"].clone());
            for expected in expected {
                assert!(
                    f.glyphs[0]
                        .contours
                        .iter()
                        .flat_map(|c| &c.points)
                        .filter(|p| p.kind == PointKind::On)
                        .any(|p| kurbo::Point::new(p.x, p.y).distance(expected) < 0.001),
                    "missing arc-length width landmark {expected:?}"
                );
            }
            assert_extrema_are_anchors(&f);
            clean(&f);
            masters.push(f);
        }
        assert!(keys.windows(2).all(|w| w[0] == w[1]));
        for pair in masters.windows(2) {
            assert!(crate::compatibility(&pair[0], &pair[1]).is_empty());
            clean(&crate::blend_fonts(&pair[0], &pair[1], 0.5).unwrap());
        }
    }
    #[test]
    fn fixed_extrema_slots_survive_profile_shape_changes() {
        let mut masters = vec![];
        let mut counts = vec![];
        let mut keys = vec![];
        for rise in [0.0, 100.0, 300.0] {
            let mut s = line((0.0, 0.0), (100.0, 100.0), 80.0);
            s.ease = WidthEase::Smooth;
            s.widths[0].left = 10.0;
            s.widths[1].left = 10.0 + rise;
            let mut f = font(vec![s]);
            let r = fixed(&mut f);
            counts.push(r["points"].clone());
            keys.push(r["glyphs"][0]["structure_key"].clone());
            assert_extrema_are_anchors(&f);
            masters.push(f);
        }
        assert!(counts.windows(2).all(|w| w[0] == w[1]));
        assert!(keys.windows(2).all(|w| w[0] == w[1]));
        for p in masters.windows(2) {
            assert!(crate::compatibility(&p[0], &p[1]).is_empty());
        }
    }
    #[test]
    fn fixed_profiles_reject_collapse_without_mutation() {
        let mut s = variable(120.0, WidthEase::Smooth);
        s.widths[1].left = 0.0;
        s.widths[1].right = 0.0;
        let mut f = font(vec![s]);
        let before = f.clone();
        assert!(
            expand_strokes(
                &mut f,
                None,
                &StrokeOptions {
                    fixed_structure: true,
                    ..StrokeOptions::default()
                },
                false
            )
            .is_err()
        );
        assert_eq!(f, before);
    }
    #[test]
    fn fixed_variable_closed_profile() {
        let center = contours(&PathData::from_bezpath(
            &kurbo::Circle::new((300.0, 300.0), 200.0).to_path(0.0001),
        ))
        .remove(0);
        let mut keys = vec![];
        let mut masters = vec![];
        for width in [80.0, 120.0, 180.0] {
            let mut s = line((0.0, 0.0), (1.0, 0.0), width);
            s.centerline = center.clone();
            s.ease = WidthEase::Smooth;
            s.widths.insert(
                1,
                WidthPoint {
                    t: 0.5,
                    left: width * 0.6,
                    right: width * 0.4,
                    angle: None,
                },
            );
            let mut f = font(vec![s]);
            let r = fixed(&mut f);
            assert_eq!(r["contours"], 2);
            keys.push(r["glyphs"][0]["structure_key"].clone());
            assert_extrema_are_anchors(&f);
            masters.push(f);
        }
        assert!(keys.windows(2).all(|w| w[0] == w[1]));
        for p in masters.windows(2) {
            assert!(crate::compatibility(&p[0], &p[1]).is_empty());
        }
    }
    #[test]
    fn fixed_variable_presets_named_stems_and_fit_failure() {
        for kind in [
            StrokePreset::TaperStart,
            StrokePreset::TaperEnd,
            StrokePreset::TaperBoth,
            StrokePreset::Swell,
            StrokePreset::Pinch,
        ] {
            let mut s = line((0.0, 0.0), (600.0, 0.0), 1.0);
            s.ease = WidthEase::Smooth;
            s.stem = Some("main".into());
            preset(
                &mut s,
                &Preset {
                    kind,
                    amount: 0.5,
                    t: 0.4,
                },
            )
            .unwrap();
            let mut keys = vec![];
            let mut masters = vec![];
            for w in [80.0, 120.0, 180.0] {
                let mut f = font(vec![s.clone()]);
                f.lib.insert("stroke_stems".into(), json!({"main":w}));
                let r = fixed(&mut f);
                keys.push(r["glyphs"][0]["structure_key"].clone());
                clean(&f);
                masters.push(f);
            }
            assert!(keys.windows(2).all(|w| w[0] == w[1]));
            for p in masters.windows(2) {
                assert!(crate::compatibility(&p[0], &p[1]).is_empty());
            }
        }
        let mut s = variable(180.0, WidthEase::Smooth);
        s.centerline
            .points
            .insert(1, point((100.0, 250.0).into(), PointKind::Off));
        s.centerline
            .points
            .insert(2, point((400.0, 250.0).into(), PointKind::Off));
        let mut f = font(vec![s]);
        let before = f.clone();
        let error = expand_strokes(
            &mut f,
            None,
            &StrokeOptions {
                fixed_structure: true,
                tolerance: 1e-10,
                ..StrokeOptions::default()
            },
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("tolerance"));
        assert_eq!(f, before);
    }
    #[test]
    fn fixed_zero_taper_tips_keep_their_own_structure() {
        for cap in [StrokeCap::Butt, StrokeCap::Round, StrokeCap::Square] {
            for kind in [
                StrokePreset::TaperStart,
                StrokePreset::TaperEnd,
                StrokePreset::TaperBoth,
            ] {
                let mut base = line((0.0, 0.0), (600.0, 0.0), 80.0);
                base.cap = cap;
                base.ease = WidthEase::Smooth;
                preset(
                    &mut base,
                    &Preset {
                        kind,
                        amount: 1.0,
                        t: 0.5,
                    },
                )
                .unwrap();
                let mut masters = vec![];
                let mut keys = vec![];
                for scale in [1.0, 1.5, 2.25] {
                    let mut s = base.clone();
                    for w in &mut s.widths {
                        w.left *= scale;
                        w.right *= scale;
                    }
                    let mut f = font(vec![s]);
                    let r = fixed(&mut f);
                    keys.push(r["glyphs"][0]["structure_key"].clone());
                    clean(&f);
                    masters.push(f);
                }
                assert!(keys.windows(2).all(|w| w[0] == w[1]));
                for p in masters.windows(2) {
                    assert!(crate::compatibility(&p[0], &p[1]).is_empty());
                }
            }
        }
        let mut zero = variable(80.0, WidthEase::Smooth);
        zero.widths[0].left = 0.0;
        zero.widths[0].right = 0.0;
        let mut f = font(vec![zero]);
        let a = fixed(&mut f);
        let mut positive = font(vec![variable(80.0, WidthEase::Smooth)]);
        let b = fixed(&mut positive);
        assert_ne!(
            a["glyphs"][0]["structure_key"],
            b["glyphs"][0]["structure_key"]
        );
    }
}
