use super::*;
fn halton(mut index: usize, base: usize) -> f64 {
    let mut f = 1.;
    let mut out = 0.;
    while index > 0 {
        f /= base as f64;
        out += f * (index % base) as f64;
        index /= base;
    }
    out
}
pub(super) fn find(
    families: &[Family],
    dims: &[Dimension],
    options: &Options,
    faces: &[Face],
) -> Vec<Value> {
    let mut classes: BTreeMap<&str, Vec<&Family>> = BTreeMap::new();
    for f in families
        .iter()
        .filter(|f| f.confidence >= 0.45 && f.class != "unclassified")
    {
        classes.entry(&f.class).or_default().push(f);
    }
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for (class, mut members) in classes {
        if members.len() < 8 {
            continue;
        }
        members.sort_by(|a, b| a.id.cmp(&b.id));
        for sample in 1..=512 {
            let anchor =
                members[(halton(sample, 2) * members.len() as f64) as usize % members.len()];
            let mut adjacent: Vec<_> = members
                .iter()
                .copied()
                .filter(|f| f.id != anchor.id)
                .filter_map(|f| {
                    embedding::distance(
                        &anchor.vector,
                        &anchor.mask,
                        &f.vector,
                        &f.mask,
                        dims,
                        options.min_shared,
                    )
                    .map(|d| (d.0, f))
                })
                .collect();
            adjacent.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
            if adjacent.len() < 2 {
                continue;
            }
            let b = adjacent[(sample / 3) % adjacent.len().min(8)].1;
            let c = adjacent[(sample / 7 + 1) % adjacent.len().min(8)].1;
            if b.id == c.id {
                continue;
            }
            let w1 = 0.2 + 0.3 * halton(sample, 3);
            let w2 = 0.2 + 0.3 * halton(sample, 5);
            let w3 = 1. - w1 - w2;
            let mut raw: Vec<_> = (0..dims.len())
                .map(|i| {
                    anchor.raw[i]
                        .zip(b.raw[i])
                        .zip(c.raw[i])
                        .map(|((a, b), c)| a * w1 + b * w2 + c * w3)
                })
                .collect();
            // Respect algebraic constraints: a stress direction is on the unit circle,
            // and the n/H ratio is derived from the two target advances.
            let index = |key: &str| dims.iter().position(|d| d.key == key);
            if let Some((sin, cos)) =
                index("stems.stress_angle.sin").zip(index("stems.stress_angle.cos"))
                && let Some((s, c)) = raw[sin].zip(raw[cos])
            {
                let norm = s.hypot(c);
                if norm > 1e-6 {
                    raw[sin] = Some(s / norm);
                    raw[cos] = Some(c / norm);
                } else {
                    raw[sin] = None;
                    raw[cos] = None;
                }
            }
            if let Some((n, h)) = index("widths.n.advance").zip(index("widths.H.advance"))
                && let Some(r) = index("widths.n.advance_relative_H")
            {
                raw[r] = crate::analyze::ratio(raw[n], raw[h]);
            }
            let (vector, mask) = embedding::encode(&raw, dims);
            if mask.iter().filter(|v| **v).count() < options.min_shared {
                continue;
            }
            let key = raw
                .iter()
                .map(|v| v.map(|v| (v * 10000.).round() as i64))
                .collect::<Vec<_>>();
            if !seen.insert(format!("{class}:{key:?}")) {
                continue;
            }
            let mut nearest: Vec<_> = members
                .iter()
                .filter_map(|f| {
                    embedding::distance(
                        &vector,
                        &mask,
                        &f.vector,
                        &f.mask,
                        dims,
                        options.min_shared,
                    )
                    .map(|d| (d.0, d.1, *f))
                })
                .collect();
            nearest.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.id.cmp(&b.2.id)));
            if nearest.len() < 3 || nearest[0].0 < 0.25 || nearest[2].0 > 2.0 {
                continue;
            }
            let occupied = faces
                .iter()
                .filter_map(|face| {
                    embedding::distance(
                        &vector,
                        &mask,
                        &face.vector,
                        &face.mask,
                        dims,
                        options.min_shared,
                    )
                    .map(|d| (d.0, d.1, face))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.2.id.cmp(&b.2.id)));
            let Some((minimum, shared, nearest_face)) = occupied else {
                continue;
            };
            if minimum < 0.25 {
                continue;
            }
            let supported:Vec<_>=nearest.iter().take(3).map(|(d,shared,f)|json!({"id":f.id,"family":f.name,"distance":d,"shared_dimensions":shared})).collect();
            let targets:Vec<_>=raw.iter().enumerate().filter_map(|(i,v)|v.map(|v|json!({"metric":dims[i].key,"label":dims[i].label,"value":v,"transform":SPECS_TRANSFORM(i)}))).collect();
            let id = super::hash(
                serde_json::to_string(&json!({"class":class,"raw":raw}))
                    .unwrap_or_default()
                    .as_bytes(),
            );
            let brief = format!(
                "Explore an original {class} design at these measured targets. Draw new outlines from a construction skeleton; use the three nearby families to understand tradeoffs. Test n/o/H/O first, then text spacing. A collection gap is not proof of novelty, usability, or legal clearance."
            );
            candidates.push((minimum,id.clone(),json!({"id":id,"class":class,"score":minimum,"minimum_distance":minimum,"family_minimum_distance":nearest[0].0,"nearest_face":{"id":nearest_face.id,"family":nearest_face.family,"style":nearest_face.style,"distance":minimum,"shared_dimensions":shared},"support":supported,"targets":targets,"raw":raw,"vector":vector,"mask":mask,"brief":brief,"method":"deterministic combinations of 3 neighboring same-class families, constrained stress direction and advance ratio; full-space masked group-balanced RMS; nearest measured face across all styles >=0.25, third-nearest family <=2; >=8 families in class","interpretation":"underrepresented measured combination within this sample, not a new font or proof of an unoccupied global design region"})));
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut chosen: Vec<Value> = Vec::new();
    for (_, _, candidate) in candidates {
        let v: Vec<f64> = serde_json::from_value(candidate["vector"].clone()).unwrap_or_default();
        let m: Vec<bool> = serde_json::from_value(candidate["mask"].clone()).unwrap_or_default();
        let distinct = chosen
            .iter()
            .filter(|g| g["class"] == candidate["class"])
            .all(|g| {
                let b: Vec<f64> = serde_json::from_value(g["vector"].clone()).unwrap_or_default();
                let bm: Vec<bool> = serde_json::from_value(g["mask"].clone()).unwrap_or_default();
                embedding::distance(&v, &m, &b, &bm, dims, options.min_shared)
                    .is_none_or(|d| d.0 >= 0.25)
            });
        if distinct {
            chosen.push(candidate);
        }
        if chosen.len() >= options.gap_count {
            break;
        }
    }
    if options.gap_count == 0 {
        chosen.clear();
    }
    chosen
}
#[allow(non_snake_case)]
fn SPECS_TRANSFORM(i: usize) -> &'static str {
    embedding::SPECS[i].4
}
pub(super) fn principles(families: &[Family], dims: &[Dimension]) -> Vec<Value> {
    let mut classes: BTreeMap<&str, Vec<&Family>> = BTreeMap::new();
    for f in families {
        classes.entry(&f.class).or_default().push(f);
    }
    let mut out = Vec::new();
    for (class, members) in classes {
        if class == "unclassified" || members.len() < 5 {
            continue;
        }
        let mut metrics = Vec::new();
        for (i, d) in dims.iter().enumerate() {
            let mut values: Vec<_> = members.iter().filter_map(|f| f.raw[i]).collect();
            values.sort_by(f64::total_cmp);
            if values.len() < 5 || values.len() * 2 < members.len() {
                continue;
            }
            metrics.push(json!({"metric":d.key,"label":d.label,"p10":embedding::quantile(&values,0.1),"median":embedding::quantile(&values,0.5),"p90":embedding::quantile(&values,0.9),"families":values.len(),"group":d.group,"method":d.method}));
        }
        let mut correlations = Vec::new();
        for i in 0..dims.len() {
            for j in i + 1..dims.len() {
                if dims[i].group == dims[j].group {
                    continue;
                }
                let pairs: Vec<_> = members
                    .iter()
                    .filter_map(|f| f.raw[i].zip(f.raw[j]))
                    .collect();
                if pairs.len() < 12 {
                    continue;
                }
                let n = pairs.len() as f64;
                let a = pairs.iter().map(|v| v.0).sum::<f64>() / n;
                let b = pairs.iter().map(|v| v.1).sum::<f64>() / n;
                let aa = pairs.iter().map(|v| (v.0 - a).powi(2)).sum::<f64>();
                let bb = pairs.iter().map(|v| (v.1 - b).powi(2)).sum::<f64>();
                if aa * bb <= 1e-12 {
                    continue;
                }
                let r = pairs.iter().map(|v| (v.0 - a) * (v.1 - b)).sum::<f64>() / (aa * bb).sqrt();
                if r.abs() >= 0.65 {
                    correlations.push(json!({"a":dims[i].key,"b":dims[j].key,"r":r,"families":pairs.len(),"interpretation":"descriptive association among selected family representatives; not causal, no significance or generalization claim"}));
                }
            }
        }
        correlations.sort_by(|a, b| {
            b["r"]
                .as_f64()
                .unwrap_or(0.)
                .abs()
                .total_cmp(&a["r"].as_f64().unwrap_or(0.).abs())
        });
        correlations.truncate(8);
        out.push(json!({"class":class,"families":members.len(),"ranges":metrics,"associations":correlations,"interpretation":"empirical starting points for original design, not universal design rules; heuristic labels and sampling affect these ranges"}));
    }
    out
}
