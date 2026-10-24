use super::*;
pub(super) fn render(atlas: &Atlas) -> Result<String, String> {
    let data = serde_json::to_string(atlas)
        .map_err(|e| e.to_string())?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    Ok(include_str!("explore.html")
        .replace("/*CHARTJS*/", include_str!("vendor/chart.umd.min.js"))
        .replace("/*ATLAS_DATA*/", &data))
}
