use super::*;
fn normalized(m: &Metric) -> Option<f64> {
    if m.units == "font_units" {
        m.x_height_ratio
    } else if m.units == "ratio" {
        m.value
    } else {
        None
    }
}
fn label(s: &Study) -> String {
    format!(
        "{} / {} / {} / face {}",
        s.metadata["family"].as_str().unwrap_or(""),
        s.metadata["style"].as_str().unwrap_or(""),
        s.source.path,
        s.source.face
    )
}
fn ranges(fonts: &[&Study]) -> Value {
    let keys: std::collections::BTreeSet<_> = fonts.iter().flat_map(|s| s.metrics.keys()).collect();
    let mut out = serde_json::Map::new();
    for k in keys {
        let mut vs: Vec<_> = fonts
            .iter()
            .filter_map(|s| s.metrics.get(k).and_then(normalized).map(|v| (v, label(s))))
            .collect();
        vs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let n = vs.len();
        if n == 0 {
            continue;
        }
        let lo = vs[0].0;
        let hi = vs[n - 1].0;
        out.insert(k.clone(),json!({"min":lo,"median":(vs[(n-1)/2].0+vs[n/2].0)*0.5,"max":hi,"count":n,"units":"ratio; lengths divided by x-height","min_families":vs.iter().filter(|(v,_)|*v==lo).map(|(_,f)|f).collect::<Vec<_>>(),"max_families":vs.iter().filter(|(v,_)|*v==hi).map(|(_,f)|f).collect::<Vec<_>>()}));
    }
    Value::Object(out)
}
pub(super) fn nearest(target: &Study, fonts: &[Study]) -> Vec<Value> {
    use crate::atlas::embedding;
    // Fit one representative per family, so extra weights don't dominate normalization.
    let mut representatives: BTreeMap<String, &Study> = BTreeMap::new();
    let rank = |s: &Study| {
        (
            s.metadata["italic_angle"].as_f64().unwrap_or(0.) != 0.,
            s.metadata["weight_class"]
                .as_i64()
                .unwrap_or(400)
                .abs_diff(400),
        )
    };
    for s in fonts {
        let key = s.metadata["family"].as_str().unwrap_or("").to_lowercase();
        representatives
            .entry(key)
            .and_modify(|old| {
                if rank(s) < rank(old) {
                    *old = s;
                }
            })
            .or_insert(s);
    }
    let rows: Vec<_> = representatives
        .values()
        .map(|s| embedding::extract(s))
        .collect();
    let dims = embedding::fit_rows(&rows);
    let raw = embedding::extract(target);
    let (tv, tm) = embedding::encode(&raw, &dims);
    let mut matches = Vec::new();
    for other in fonts {
        if target.metadata["family"] == other.metadata["family"]
            || (target.source.sha256 == other.source.sha256
                && target.source.face == other.source.face
                && target.source.axes == other.source.axes)
        {
            continue;
        }
        let other_raw = embedding::extract(other);
        let (ov, om) = embedding::encode(&other_raw, &dims);
        let Some((distance, shared)) = embedding::distance(&tv, &tm, &ov, &om, &dims, 12) else {
            continue;
        };
        let mut diffs: Vec<_> = dims
            .iter()
            .enumerate()
            .filter(|(i, _)| tm[*i] && om[*i])
            .map(|(i, d)| (i, (tv[i] - ov[i]).abs(), d))
            .collect();
        diffs.sort_by(|a, b| b.1.total_cmp(&a.1));
        matches.push((distance,label(other),json!({"source":other.source,"family":other.metadata["family"],
            "distance":distance,"compared_features":shared,"dimensions":dims.len(),"embedding":crate::atlas::EMBEDDING,
            "method":"atlas 33-dimension transforms, masks and domain-balanced RMS; family-balanced fit to reference collection; >=12 shared dimensions across >=4 domains; distances comparable only within this fitted collection",
            "differences":diffs.iter().take(4).map(|(i,_,d)|json!({"metric":d.key,"target":raw[*i],"other":other_raw[*i],"standardized_difference":tv[*i]-ov[*i]})).collect::<Vec<_>>()})));
    }
    matches.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut seen = std::collections::BTreeSet::new();
    matches
        .into_iter()
        .filter(|(_, _, v)| seen.insert(v["family"].to_string()))
        .take(5)
        .map(|(_, _, v)| v)
        .collect()
}
fn deviations(s: &Study, ranges: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for (k, m) in &s.metrics {
        let Some(r) = ranges.get(k) else {
            continue;
        };
        let Some(v) = normalized(m) else {
            continue;
        };
        let (Some(lo), Some(hi), Some(mid)) =
            (r["min"].as_f64(), r["max"].as_f64(), r["median"].as_f64())
        else {
            continue;
        };
        let d = if v < lo {
            v - lo
        } else if v > hi {
            v - hi
        } else {
            0.
        };
        let scale = (hi - lo).max(mid.abs() * 0.05).max(0.0001);
        out.push(json!({"metric":k,"value":v,"min":lo,"max":hi,"in_range":d==0.,"deviation":d,"normalized_deviation":d/scale,"reference_count":r["count"]}));
    }
    out.sort_by(|a, b| {
        b["normalized_deviation"]
            .as_f64()
            .unwrap_or(0.)
            .abs()
            .total_cmp(&a["normalized_deviation"].as_f64().unwrap_or(0.).abs())
            .then(a["metric"].as_str().cmp(&b["metric"].as_str()))
    });
    out
}
pub(super) fn summarize(fonts: &mut [Study]) -> Value {
    let mut groups: BTreeMap<String, Vec<&Study>> = BTreeMap::new();
    for s in fonts.iter() {
        groups
            .entry(s.classification.class.clone())
            .or_default()
            .push(s);
    }
    let classes: BTreeMap<_, _> = groups
        .iter()
        .map(|(k, v)| (k.clone(), json!({"fonts":v.len(),"metrics":ranges(v)})))
        .collect();
    let mut universal = Vec::new();
    if classes.len() >= 2 {
        let all: Vec<_> = fonts.iter().collect();
        let rs = ranges(&all);
        for (k, r) in rs.as_object().into_iter().flatten() {
            let (Some(lo), Some(hi), Some(mid)) =
                (r["min"].as_f64(), r["max"].as_f64(), r["median"].as_f64())
            else {
                continue;
            };
            if r["count"].as_u64() == Some(fonts.len() as u64)
                && (hi - lo) <= 0.02 * mid.abs().max(0.01)
            {
                universal.push(json!({"metric":k,"min":lo,"max":hi,"criterion":"complete observations; range <=2% of max(abs(median),0.01)","caution":"collection observation, not a typographic universal"}));
            }
        }
    }
    let near: Vec<_> = fonts.iter().map(|s| nearest(s, fonts)).collect();
    let outliers: Vec<_> = fonts
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let peers: Vec<_> = fonts
                .iter()
                .enumerate()
                .filter(|(j, p)| {
                    *j != i
                        && p.classification.class == s.classification.class
                        && p.metadata["family"] != s.metadata["family"]
                })
                .map(|(_, p)| p)
                .collect();
            if peers.len() < 3 {
                vec![]
            } else {
                deviations(s, &ranges(&peers))
                    .into_iter()
                    .filter(|v| v["in_range"] == false)
                    .collect()
            }
        })
        .collect();
    for ((s, near), outliers) in fonts.iter_mut().zip(near).zip(outliers) {
        s.nearest = near;
        s.outliers = outliers;
    }
    json!({"classes":classes,"possible_universals":universal,"range_method":"font-unit lengths normalized by x-height; ratios unchanged; observed min/median/max, not population limits","outlier_method":"outside leave-one-family-out peer range; at least 3 peer faces"})
}
pub(super) fn compare(target: &Report, reference: &Report, class: &str) -> Result<Value, String> {
    let ranges = reference.collection["classes"]
        .get(class)
        .ok_or_else(|| format!("class {class:?} absent from reference collection"))?["metrics"]
        .clone();
    Ok(
        json!({"class":class,"fonts":target.fonts.iter().map(|s|json!({"source":s.source,"metrics":deviations(s,&ranges)})).collect::<Vec<_>>(),"errors":([target.errors.clone(),reference.errors.clone()].concat())}),
    )
}
