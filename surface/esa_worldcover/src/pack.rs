use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::Path;
use yamamuki_data_sqlite::rusqlite::{Connection, OpenFlags, params};

const ESA: &str = "ESA WorldCover 2021";
const GSI: &str = "国土地理院の標高タイル（DEM10B）";

pub fn import(db: &mut Connection, source: &Path, version: &str) -> Result<()> {
    let source = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let read_metadata = |key| -> Result<Value> {
        let value: String =
            source.query_row("SELECT value FROM metadata WHERE key=?1", [key], |row| {
                row.get(0)
            })?;
        Ok(serde_json::from_str(&value)?)
    };
    let manifest = read_metadata("manifest")?;
    let shade = read_metadata("hillshade")?;
    ensure!(
        manifest["version"] == 1
            && manifest["crs"] == "EPSG:3857"
            && manifest["landEncoding"] == "forest-bare-unified-gray4"
            && manifest["waterEncoding"] == "gray4-coverage"
            && manifest["bareOmitted"] == true
            && manifest["maskWebp"] == true
            && manifest["sourceNoDataClassification"]["landEvidence"] == "positive-elevation"
            && shade["encoding"] == "webp-or-png-gray8"
            && shade["version"] == 1
            && shade["scheme"] == "xyz",
        "ESA地表の中間SQLiteの形式が不正です",
    );
    let attribution = manifest["attribution"]
        .as_str()
        .context("ESAの帰属表示がありません")?;
    let palette = manifest["palette"]
        .as_array()
        .filter(|palette| palette.len() == 5)
        .context("地表のパレットがありません")?;
    let mut metadata = json!({
        "coverage": {"regions": ["JP"]},
        "authors": [{
            "name": "The yamamuki Authors",
            "role": "配布データの編集・加工",
            "url": "https://github.com/shohei0205/yamamuki-data/blob/main/AUTHORS.md"
        }],
        "sources": [
            {"name": ESA, "source": "https://esa-worldcover.org/en/data-access",
             "license": "CC BY 4.0 https://creativecommons.org/licenses/by/4.0/",
             "attribution": attribution,
             "changes": "分類・補完・ノイズ除去・縮小を行い、森林と水域の画像タイルに加工"},
            {"name": GSI, "source": "https://maps.gsi.go.jp/development/ichiran.html",
             "license": "国土地理院コンテンツ利用規約 https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html",
             "attribution": "地理院タイル（標高タイル）を加工して作成",
             "changes": "標高タイルを地表分類の補助判定と陰影の生成に使用"}
        ]
    });
    // 国・地域と言語を引き継ぐ。図形は収録タイルの読み込み後に再計算する。
    if let Some(coverage) = manifest.get("coverage") {
        metadata["coverage"] = coverage.clone();
    }
    if let Some(language) = manifest.get("language") {
        metadata["language"] = language.clone();
    }
    yamamuki_data_sqlite::validate_coverage_metadata(&metadata, true)?;
    let complete_tile_extent = manifest["completeCoarseTiles"] == true;
    let tx = db.transaction()?;
    tx.execute(
        "INSERT INTO datasets(id,kind,title,data_version,format_version,metadata_json)
         VALUES('esa-worldcover-japan','surface','日本全国の地表（ESA WorldCover 2021）',?1,1,?2)",
        params![version, metadata.to_string()],
    )?;
    let dataset = tx.last_insert_rowid();
    for (key, id, role, draw_order, names, settings, table, spec, levels) in [
        (
            1,
            "forest",
            "mask",
            0,
            json!([ESA, GSI]),
            json!({"color": palette[1], "background": palette[0], "sampleBits": 4, "completeTileExtent": complete_tile_extent}),
            "tiles",
            &manifest,
            &manifest["levels"],
        ),
        (
            2,
            "water",
            "mask",
            20,
            json!([ESA, GSI]),
            json!({"color": palette[3], "sampleBits": 4, "completeTileExtent": complete_tile_extent}),
            "water_tiles",
            &manifest,
            &manifest["waterLevels"],
        ),
        (
            3,
            "hillshade",
            "hillshade",
            10,
            json!([GSI]),
            json!({"neutral": shade["neutral"], "strength": 0.65, "azimuths": shade["azimuths"], "lightAltitude": shade["lightAltitude"], "quantization": shade["quantization"], "webpQuality": shade["webpQuality"]}),
            "hillshade_tiles",
            &shade,
            &shade["levels"],
        ),
    ] {
        tx.execute(
            "INSERT INTO surface_layers VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                dataset,
                key,
                id,
                role,
                draw_order,
                names.to_string(),
                settings.to_string()
            ],
        )?;
        let offset = spec["tileKeyZoomOffset"]
            .as_i64()
            .context("XYZズームの補正値がありません")?;
        let default_size = spec["tilePixels"]
            .as_i64()
            .context("タイル寸法がありません")?;
        let levels = levels
            .as_array()
            .filter(|levels| !levels.is_empty())
            .context("レベル定義がありません")?;
        for level in levels {
            let nominal_z = level["z"].as_i64().context("ズームがありません")?;
            let z = nominal_z
                .checked_add(offset)
                .filter(|z| (0..=30).contains(z))
                .context("XYZズームが範囲外です")?;
            let pixels = level["tilePixels"].as_i64().unwrap_or(default_size);
            tx.execute(
                "INSERT INTO surface_levels VALUES(?1,?2,?3,?4,?5)",
                params![dataset, key, z, format!("z{z}"), pixels],
            )?;
        }
        let mut read =
            source.prepare(&format!("SELECT z,x,y,payload FROM {table} ORDER BY z,x,y"))?;
        let mut write = tx.prepare("INSERT INTO surface_tiles VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
        let mut rows = read.query([])?;
        while let Some(row) = rows.next()? {
            let z = row
                .get::<_, i64>(0)?
                .checked_add(offset)
                .context("XYZズームが不正です")?;
            let x: i64 = row.get(1)?;
            let y: i64 = row.get(2)?;
            let payload = row.get_ref(3)?.as_blob()?;
            let encoding = if payload.starts_with(b"\x89PNG\r\n\x1a\n") {
                1
            } else if payload.starts_with(b"RIFF") && payload.get(8..12) == Some(b"WEBP") {
                2
            } else {
                anyhow::bail!("{table}: 未対応の画像形式 ({z}/{x}/{y})");
            };
            write.execute(params![dataset, key, z, x, y, encoding, payload])?;
        }
    }
    yamamuki_data_sqlite::recompute_surface_coverage(&tx, dataset)?;
    tx.commit()?;
    Ok(())
}
