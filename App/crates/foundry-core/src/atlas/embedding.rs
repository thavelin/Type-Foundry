use super::*;
// key, label, domain, minimum robust scale, transform. Domain weighting prevents correlated widths from dominating.
pub(crate) const SPECS: &[(&str, &str, &str, f64, &str)] = &[
    (
        "proportions.x_cap",
        "X-height / cap height",
        "proportions",
        0.025,
        "ratio",
    ),
    (
        "proportions.cap_upm",
        "Cap height / em",
        "proportions",
        0.025,
        "ratio",
    ),
    (
        "proportions.ascender_x",
        "Ascender / x-height",
        "proportions",
        0.05,
        "ratio",
    ),
    (
        "proportions.descender_x",
        "Descender / x-height",
        "proportions",
        0.05,
        "ratio",
    ),
    (
        "weight.stem_x_height",
        "Stem / x-height",
        "weight",
        0.025,
        "ratio",
    ),
    (
        "stems.uppercase",
        "Cap stem / x-height",
        "weight",
        0.025,
        "length",
    ),
    (
        "stems.contrast",
        "Log O stroke contrast",
        "contrast",
        0.15,
        "log",
    ),
    (
        "stems.stress_angle",
        "O stress sine",
        "contrast",
        0.2,
        "sin",
    ),
    (
        "stems.stress_angle",
        "O stress cosine",
        "contrast",
        0.2,
        "cos",
    ),
    (
        "widths.o_x",
        "Lowercase O width / x-height",
        "width",
        0.08,
        "ratio",
    ),
    (
        "widths.n.advance",
        "N advance / x-height",
        "width",
        0.08,
        "length",
    ),
    (
        "widths.H.advance",
        "H advance / x-height",
        "width",
        0.08,
        "length",
    ),
    (
        "widths.O.advance",
        "O advance / x-height",
        "width",
        0.08,
        "length",
    ),
    (
        "widths.n.advance_relative_H",
        "N / H advance",
        "width",
        0.05,
        "ratio",
    ),
    (
        "overshoot.o.above",
        "Lowercase round overshoot / x-height",
        "overshoot",
        0.01,
        "length",
    ),
    (
        "overshoot.O.above",
        "Cap round overshoot / x-height",
        "overshoot",
        0.01,
        "length",
    ),
    (
        "overshoot.o.below",
        "Lowercase baseline overshoot / x-height",
        "overshoot",
        0.01,
        "length",
    ),
    (
        "arch.n.springing_x",
        "N arch spring / x-height",
        "arch",
        0.05,
        "ratio",
    ),
    (
        "arch.n.thickness_10",
        "N arch 10% thickness / x-height",
        "arch",
        0.025,
        "length",
    ),
    (
        "arch.n.thickness_20",
        "N arch 20% thickness / x-height",
        "arch",
        0.025,
        "length",
    ),
    (
        "arch.n.thickness_50",
        "N arch middle thickness / x-height",
        "arch",
        0.025,
        "length",
    ),
    (
        "arch.n.thinnest",
        "N minimum arch thickness / x-height",
        "arch",
        0.025,
        "length",
    ),
    (
        "inner.o.superellipse_exponent",
        "O counter superellipse exponent",
        "counter",
        0.25,
        "ratio",
    ),
    (
        "inner.O.superellipse_exponent",
        "Cap O counter superellipse exponent",
        "counter",
        0.25,
        "ratio",
    ),
    (
        "inner.o.tightest_radius",
        "O minimum inner radius / x-height",
        "counter",
        0.04,
        "length",
    ),
    (
        "inner.n.tightest_radius",
        "N minimum inner radius / x-height",
        "counter",
        0.04,
        "length",
    ),
    (
        "inner.o.handle_fraction",
        "O inner handle / chord",
        "counter",
        0.05,
        "ratio",
    ),
    (
        "serif.presence_score",
        "Foot extension / stem",
        "serif",
        0.15,
        "ratio",
    ),
    (
        "widths.n.lsb_stem",
        "N left bearing / stem",
        "spacing",
        0.2,
        "ratio",
    ),
    (
        "widths.n.rsb_stem",
        "N right bearing / stem",
        "spacing",
        0.2,
        "ratio",
    ),
    (
        "widths.o.lsb_stem",
        "O left bearing / stem",
        "spacing",
        0.2,
        "ratio",
    ),
    (
        "widths.o.rsb_stem",
        "O right bearing / stem",
        "spacing",
        0.2,
        "ratio",
    ),
    (
        "spacing.texture_greyness",
        "Pangram ink coverage",
        "texture",
        0.025,
        "ratio",
    ),
];
pub(crate) fn extract(s: &Study) -> Vec<Option<f64>> {
    SPECS
        .iter()
        .map(|(key, _, _, _, transform)| {
            let m = s.metrics.get(*key)?;
            let raw = if *transform == "length" {
                m.x_height_ratio
            } else {
                m.value
            }?;
            let v = match *transform {
                "log" => {
                    if raw <= 0. {
                        return None;
                    }
                    raw.ln()
                }
                "sin" => (2. * raw.to_radians()).sin(),
                "cos" => (2. * raw.to_radians()).cos(),
                _ => raw,
            };
            v.is_finite().then_some(v)
        })
        .collect()
}
pub(crate) fn quantile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.;
    }
    let at = (values.len() - 1) as f64 * p;
    let lo = at.floor() as usize;
    let hi = at.ceil() as usize;
    values[lo] + (values[hi] - values[lo]) * (at - lo as f64)
}
pub(crate) fn fit(families: &[Family]) -> Vec<Dimension> {
    fit_rows(&families.iter().map(|f| f.raw.clone()).collect::<Vec<_>>())
}
pub(crate) fn fit_rows(rows: &[Vec<Option<f64>>]) -> Vec<Dimension> {
    SPECS.iter().enumerate().map(|(i,(key,label,group,floor,transform))|{
        let mut values: Vec<_> = rows.iter().filter_map(|r|r[i]).collect();
        values.sort_by(f64::total_cmp);
        let center=quantile(&values,0.5);
        let scale=((quantile(&values,0.75)-quantile(&values,0.25))/1.349).max(*floor);
        Dimension {key:if matches!(*transform,"sin"|"cos"){format!("{key}.{transform}")}else{(*key).into()},
            label:(*label).into(),group:(*group).into(),
            method:format!("{transform}; family-balanced median/IQR scale with floor {floor}; winsorized z to +/-5; missing mask"),
            center,scale,measured_families:values.len()}
    }).collect()
}
pub(crate) fn encode(raw: &[Option<f64>], dims: &[Dimension]) -> (Vec<f64>, Vec<bool>) {
    (
        raw.iter()
            .zip(dims)
            .map(|(v, d)| {
                v.map(|v| ((v - d.center) / d.scale).clamp(-5., 5.))
                    .unwrap_or(0.)
            })
            .collect(),
        raw.iter().map(Option::is_some).collect(),
    )
}
pub(crate) fn distance(
    a: &[f64],
    am: &[bool],
    b: &[f64],
    bm: &[bool],
    dims: &[Dimension],
    min_shared: usize,
) -> Option<(f64, usize)> {
    let mut totals = [0.; 10];
    let mut counts = [0usize; 10];
    let mut shared = 0;
    for (i, d) in dims.iter().enumerate() {
        if am[i] && bm[i] {
            let group = match d.group.as_str() {
                "proportions" => 0,
                "weight" => 1,
                "contrast" => 2,
                "width" => 3,
                "overshoot" => 4,
                "arch" => 5,
                "counter" => 6,
                "serif" => 7,
                "spacing" => 8,
                "texture" => 9,
                _ => return None,
            };
            totals[group] += (a[i] - b[i]).powi(2);
            counts[group] += 1;
            shared += 1;
        }
    }
    let groups = counts.iter().filter(|n| **n > 0).count();
    if shared < min_shared || groups < 4 {
        return None;
    }
    let mean = totals
        .iter()
        .zip(counts)
        .filter(|(_, n)| *n > 0)
        .map(|(sum, n)| sum / n as f64)
        .sum::<f64>()
        / groups as f64;
    Some((mean.sqrt(), shared))
}
pub(crate) fn nearest(
    f: &Family,
    all: &[Family],
    dims: &[Dimension],
    min_shared: usize,
    limit: usize,
) -> Vec<Value> {
    let mut matches: Vec<_> = all
        .iter()
        .filter(|g| g.id != f.id)
        .filter_map(|g| {
            distance(&f.vector, &f.mask, &g.vector, &g.mask, dims, min_shared).map(|d| (d, g))
        })
        .collect();
    matches.sort_by(|a, b| a.0.0.total_cmp(&b.0.0).then(a.1.id.cmp(&b.1.id)));
    matches.into_iter().take(limit).map(|((distance,shared),g)|{
 let mut diffs:Vec<_>=dims.iter().enumerate().filter(|(i,_)|f.mask[*i]&&g.mask[*i]).map(|(i,d)|(i,(f.vector[i]-g.vector[i]).abs(),d)).collect();diffs.sort_by(|a,b|b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
 json!({"id":g.id,"family":g.name,"class":g.class,"distance":distance,"shared_dimensions":shared,"coverage":shared as f64/dims.len()as f64,"differences":diffs.iter().take(4).map(|(i,_,d)|json!({"metric":d.key,"label":d.label,"target":f.raw[*i],"other":g.raw[*i],"standardized_difference":f.vector[*i]-g.vector[*i]})).collect::<Vec<_>>()})
 }).collect()
}
pub(crate) fn project(families: &mut [Family], dims: &[Dimension]) -> Value {
    let n = dims.len();
    let mut cov = vec![vec![0.; n]; n];
    let denom = (families.len().saturating_sub(1)).max(1) as f64;
    let means: Vec<_> = (0..n)
        .map(|i| families.iter().map(|f| f.vector[i]).sum::<f64>() / families.len().max(1) as f64)
        .collect();
    for f in families.iter() {
        for i in 0..n {
            for j in 0..n {
                cov[i][j] += (f.vector[i] - means[i]) * (f.vector[j] - means[j]) / denom;
            }
        }
    }
    let multiply = |v: &[f64]| {
        cov.iter()
            .map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum::<f64>())
            .collect::<Vec<_>>()
    };
    let mut axes: Vec<Vec<f64>> = Vec::new();
    let mut eigenvalues = Vec::new();
    for axis in 0..2 {
        let mut v: Vec<_> = (0..n)
            .map(|i| ((i + 1 + axis) as f64 * 1.618).sin())
            .collect();
        for _ in 0..120 {
            let mut w = multiply(&v);
            for previous in &axes {
                let dot = w.iter().zip(previous).map(|(a, b)| a * b).sum::<f64>();
                for (i, x) in w.iter_mut().enumerate() {
                    *x -= dot * previous[i];
                }
            }
            let norm = w.iter().map(|v| v * v).sum::<f64>().sqrt();
            if norm < 1e-12 {
                v.fill(0.);
                break;
            }
            for x in &mut w {
                *x /= norm;
            }
            v = w;
        }
        let pivot = v
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|v| v.0)
            .unwrap_or(0);
        if v[pivot] < 0. {
            for x in &mut v {
                *x = -*x;
            }
        }
        let mv = multiply(&v);
        eigenvalues.push(v.iter().zip(mv).map(|(a, b)| a * b).sum::<f64>().max(0.));
        axes.push(v);
    }
    for f in families.iter_mut() {
        for (axis, v) in axes.iter().enumerate() {
            f.projection[axis] = v
                .iter()
                .enumerate()
                .map(|(i, v)| v * (f.vector[i] - means[i]))
                .sum();
        }
    }
    let total = (0..n).map(|i| cov[i][i]).sum::<f64>();
    json!({"method":"deterministic centered PCA; median-imputed masked coordinates for visualization only; 120 power iterations; stable sign","components":axes,"center":means,"explained_variance":eigenvalues.iter().map(|v|if total>0.{v/total}else{0.}).collect::<Vec<_>>(),"loadings":axes.iter().map(|axis|{let mut sorted:Vec<_>=axis.iter().enumerate().collect();sorted.sort_by(|a,b|b.1.abs().total_cmp(&a.1.abs()));sorted.into_iter().take(6).map(|(i,v)|json!({"metric":dims[i].key,"loading":v})).collect::<Vec<_>>()}).collect::<Vec<_>>(),"caution":"PCA proximity and blank plot regions are not full-space distances or novelty proof"})
}
pub(crate) fn cluster(
    families: &mut [Family],
    dims: &[Dimension],
    min_shared: usize,
) -> Vec<Value> {
    let mut remaining: BTreeSet<_> = (0..families.len())
        .filter(|i| families[*i].class != "unclassified" && families[*i].confidence >= 0.4)
        .collect();
    let mut out = Vec::new();
    while !remaining.is_empty() {
        let mut best: Option<(usize, Vec<usize>)> = None;
        for anchor in &remaining {
            let group: Vec<_> = remaining
                .iter()
                .copied()
                .filter(|j| {
                    families[*j].class == families[*anchor].class
                        && distance(
                            &families[*anchor].vector,
                            &families[*anchor].mask,
                            &families[*j].vector,
                            &families[*j].mask,
                            dims,
                            min_shared,
                        )
                        .is_some_and(|d| d.0 <= 0.9)
                })
                .collect();
            if best.as_ref().is_none_or(|(_, b)| group.len() > b.len()) {
                best = Some((*anchor, group));
            }
        }
        let Some((anchor, group)) = best else {
            break;
        };
        if group.len() < 3 {
            break;
        }
        let id = out.len();
        let members: Vec<_> = group.iter().map(|i| families[*i].id.clone()).collect();
        out.push(json!({"id":id,"class":families[anchor].class,"anchor":families[anchor].id,"families":members,"method":"greedy densest-anchor neighborhoods; full-space group-balanced RMS radius <=0.9; at least 3 families; no transitive chaining"}));
        for i in group {
            families[i].cluster = Some(id);
            remaining.remove(&i);
        }
    }
    out
}
