//! 地点・地表に共通する対象範囲。日付変更線で分割済みの WGS 84 を扱う。

use anyhow::{Context, Result, ensure};
use geo::{Area, Covers, LineString, MultiPolygon, Polygon, Validation};
use serde_json::Value;
use std::collections::HashSet;

pub fn validate_metadata(metadata: &Value, surface: bool) -> Result<()> {
    ensure!(metadata.is_object(), "metadata は object にしてください");
    if let Some(language) = metadata.get("language") {
        validate_language(
            language
                .as_str()
                .context("language は文字列にしてください")?,
        )?;
    }
    let Some(coverage) = metadata.get("coverage") else {
        return Ok(());
    };
    ensure!(coverage.is_object(), "coverage は object にしてください");
    if let Some(regions) = coverage.get("regions") {
        let regions = regions
            .as_array()
            .filter(|v| !v.is_empty())
            .context("regions は1件以上の配列にしてください")?;
        let mut seen = HashSet::new();
        for region in regions {
            let region = region
                .as_str()
                .context("国・地域コードは文字列にしてください")?;
            ensure!(
                region.len() == 2
                    && region.bytes().all(|c| c.is_ascii_uppercase())
                    && seen.insert(region),
                "国・地域コードが不正または重複しています"
            );
        }
    }
    let outer = coverage.get("geometry").map(polygons).transpose()?;
    if let Some(finest) = coverage.get("finest") {
        ensure!(
            surface && finest.is_object(),
            "finest は地表の object にしてください"
        );
        let outer = outer.context("finest には全体の geometry が必要です")?;
        let inner = polygons(
            finest
                .get("geometry")
                .context("finest.geometry が必要です")?,
        )?;
        ensure!(
            outer.covers(&inner),
            "finest が全体の範囲からはみ出しています"
        );
    }
    Ok(())
}

fn validate_language(tag: &str) -> Result<()> {
    let lower = tag.to_ascii_lowercase();
    if "en-gb-oed i-ami i-bnn i-default i-enochian i-hak i-klingon i-lux i-mingo i-navajo i-pwn i-tao i-tay i-tsu sgn-be-fr sgn-be-nl sgn-ch-de art-lojban cel-gaulish no-bok no-nyn zh-guoyu zh-hakka zh-min zh-min-nan zh-xiang".split_whitespace().any(|s| s == lower) {
        return Ok(());
    }
    let parts: Vec<_> = lower.split('-').collect();
    ensure!(
        parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 8 && p.bytes().all(|c| c.is_ascii_alphanumeric())),
        "language が不正です"
    );
    if parts[0] == "x" {
        ensure!(parts.len() > 1, "私用タグが空です");
        return Ok(());
    }
    let alpha = |s: &str| s.bytes().all(|c| c.is_ascii_alphabetic());
    ensure!(
        (2..=8).contains(&parts[0].len()) && alpha(parts[0]),
        "言語コードが不正です"
    );
    let mut i = 1;
    if parts[0].len() <= 3 {
        for _ in 0..3 {
            if i < parts.len() && parts[i].len() == 3 && alpha(parts[i]) {
                i += 1;
            } else {
                break;
            }
        }
    }
    if i < parts.len() && parts[i].len() == 4 && alpha(parts[i]) {
        i += 1;
    }
    if i < parts.len()
        && ((parts[i].len() == 2 && alpha(parts[i]))
            || (parts[i].len() == 3 && parts[i].bytes().all(|c| c.is_ascii_digit())))
    {
        i += 1;
    }
    let mut seen = HashSet::new();
    while i < parts.len()
        && (parts[i].len() >= 5 || (parts[i].len() == 4 && parts[i].as_bytes()[0].is_ascii_digit()))
    {
        ensure!(
            seen.insert(parts[i]),
            "language の variant が重複しています"
        );
        i += 1;
    }
    seen.clear();
    while i < parts.len() && parts[i].len() == 1 && parts[i] != "x" {
        ensure!(seen.insert(parts[i]), "language の拡張が重複しています");
        i += 1;
        let start = i;
        while i < parts.len() && parts[i].len() >= 2 {
            i += 1;
        }
        ensure!(i > start, "language の拡張が空です");
    }
    if i < parts.len() && parts[i] == "x" {
        i += 1;
        ensure!(i < parts.len(), "私用タグが空です");
        i = parts.len();
    }
    ensure!(
        i == parts.len(),
        "language は BCP 47 の言語タグにしてください"
    );
    Ok(())
}

fn polygons(value: &Value) -> Result<MultiPolygon<f64>> {
    let coordinates = value
        .get("coordinates")
        .and_then(Value::as_array)
        .context("geometry.coordinates は配列にしてください")?;
    let inputs: Vec<&Value> = match value.get("type").and_then(Value::as_str) {
        Some("Polygon") => vec![&value["coordinates"]],
        Some("MultiPolygon") => coordinates.iter().collect(),
        _ => anyhow::bail!("geometry は Polygon または MultiPolygon にしてください"),
    };
    ensure!(!inputs.is_empty(), "geometry が空です");
    let mut result = Vec::new();
    for input in inputs {
        let rings = input
            .as_array()
            .filter(|v| !v.is_empty())
            .context("外周がありません")?;
        let mut polygon = Vec::new();
        for input in rings {
            let points = input
                .as_array()
                .filter(|v| v.len() >= 4)
                .context("リングは4座標以上にしてください")?;
            let mut ring: Vec<(f64, f64)> = Vec::new();
            let mut seen = HashSet::new();
            for (i, point) in points.iter().enumerate() {
                let p = point
                    .as_array()
                    .filter(|v| v.len() == 2)
                    .context("座標は [経度, 緯度] にしてください")?;
                let x = p[0].as_f64().context("経度が不正です")?;
                let y = p[1].as_f64().context("緯度が不正です")?;
                ensure!(
                    x.is_finite()
                        && y.is_finite()
                        && (-180.0..=180.0).contains(&x)
                        && (-90.0..=90.0).contains(&y),
                    "座標が範囲外です"
                );
                if i + 1 < points.len() {
                    ensure!(
                        seen.insert((
                            if x == 0. { 0 } else { x.to_bits() },
                            if y == 0. { 0 } else { y.to_bits() }
                        )),
                        "リングの座標が重複しています"
                    );
                }
                ring.push((x, y));
            }
            ensure!(ring.first() == ring.last(), "リングが閉じていません");
            ensure!(
                ring.windows(2).all(|e| (e[0].0 - e[1].0).abs() <= 180.0),
                "日付変更線で面を分割してください"
            );
            polygon.push(LineString::from(ring));
        }
        let outer = polygon.remove(0);
        let polygon = Polygon::new(outer, polygon);
        ensure!(polygon.unsigned_area() > 0., "リングの面積がありません");
        result.push(polygon);
    }
    let result = MultiPolygon(result);
    result
        .check_validation()
        .context("提供範囲の面が不正です")?;
    Ok(result)
}
