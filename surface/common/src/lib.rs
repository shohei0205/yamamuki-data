use anyhow::{Context, Result, ensure};
use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, io::Cursor, path::Path};

pub use rusqlite;

mod coverage;
mod tile_coverage;
pub use coverage::validate_metadata as validate_coverage_metadata;
pub use tile_coverage::recompute as recompute_surface_coverage;

const APPLICATION_ID: i64 = 1498235970;
const CONTAINER_VERSION: i64 = 1;

pub fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA page_size=512;")?;
    db.execute_batch(include_str!("schema.sql"))?;
    Ok(())
}

pub fn open(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("SQLiteを開けません: {}", path.display()))?;
    let app_id: i64 = db.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    let version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    ensure!(
        app_id == APPLICATION_ID && version == CONTAINER_VERSION,
        "未対応の山むきSQLite形式です"
    );
    Ok(db)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dataset {
    #[serde(skip)]
    pub key: i64,
    pub id: String,
    pub kind: String,
    pub title: String,
    pub data_version: String,
    pub format_version: i64,
    pub metadata: Value,
}

pub fn datasets(db: &Connection) -> Result<Vec<Dataset>> {
    let mut query = db.prepare(
        "SELECT dataset_key,id,kind,title,data_version,format_version,metadata_json FROM datasets ORDER BY id",
    )?;
    let mut rows = query.query([])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        result.push(Dataset {
            key: row.get(0)?,
            id: row.get(1)?,
            kind: row.get(2)?,
            title: row.get(3)?,
            data_version: row.get(4)?,
            format_version: row.get(5)?,
            metadata: serde_json::from_str(&row.get::<_, String>(6)?)?,
        });
    }
    Ok(result)
}

#[derive(Debug)]
pub struct SurfaceLayer {
    pub key: i64,
    pub id: String,
    pub role: String,
    pub source_names: Vec<String>,
    pub settings: Value,
}

pub fn surface_layers(db: &Connection, dataset: i64) -> Result<Vec<SurfaceLayer>> {
    let mut query = db.prepare(
        "SELECT layer_key,id,role,source_names_json,settings_json FROM surface_layers
         WHERE dataset_key=?1 ORDER BY draw_order,layer_key",
    )?;
    let mut rows = query.query([dataset])?;
    let mut layers = Vec::new();
    while let Some(row) = rows.next()? {
        layers.push(SurfaceLayer {
            key: row.get(0)?,
            id: row.get(1)?,
            role: row.get(2)?,
            source_names: serde_json::from_str(&row.get::<_, String>(3)?)?,
            settings: serde_json::from_str(&row.get::<_, String>(4)?)?,
        });
    }
    Ok(layers)
}

#[derive(Debug, PartialEq, Eq)]
pub struct SurfaceLevel {
    pub z: u32,
    pub lod: String,
    pub pixels: u32,
}

pub fn surface_levels(db: &Connection, dataset: i64, layer: i64) -> Result<Vec<SurfaceLevel>> {
    let mut query = db.prepare(
        "SELECT z,lod,tile_pixels FROM surface_levels WHERE dataset_key=?1 AND layer_key=?2 ORDER BY z",
    )?;
    Ok(query
        .query_map(params![dataset, layer], |row| {
            Ok(SurfaceLevel {
                z: row.get(0)?,
                lod: row.get(1)?,
                pixels: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn surface_tile(
    db: &Connection,
    dataset: i64,
    layer: i64,
    z: u32,
    x: u32,
    y: u32,
) -> Result<Option<DynamicImage>> {
    let row = db
        .query_row(
            "SELECT t.encoding,t.payload,l.tile_pixels FROM surface_tiles AS t
         JOIN surface_levels AS l USING(dataset_key,layer_key,z)
         WHERE t.dataset_key=?1 AND t.layer_key=?2 AND t.z=?3 AND t.x=?4 AND t.y=?5",
            params![dataset, layer, z, x, y],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, u32>(2)?,
                ))
            },
        )
        .optional()?;
    row.map(|(encoding, payload, pixels)| decode_tile(encoding, &payload, pixels))
        .transpose()
}

pub fn decode_tile(encoding: i64, bytes: &[u8], pixels: u32) -> Result<DynamicImage> {
    let format = match encoding {
        1 => ImageFormat::Png,
        2 => ImageFormat::WebP,
        3 => ImageFormat::Jpeg,
        _ => anyhow::bail!("未対応の画像形式: {encoding}"),
    };
    ensure!((1..=4096).contains(&pixels), "タイル寸法が範囲外です");
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(pixels);
    limits.max_image_height = Some(pixels);
    reader.limits(limits);
    let image = reader.decode()?;
    ensure!(
        image.width() == pixels && image.height() == pixels,
        "画像の寸法がレベル定義と一致しません"
    );
    Ok(image)
}

pub fn validate(db: &Connection) -> Result<()> {
    let result: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(result == "ok", "SQLiteの整合性検査に失敗: {result}");
    ensure!(
        !db.prepare("PRAGMA foreign_key_check")?.exists([])?,
        "SQLiteの参照先が見つかりません"
    );
    let datasets = datasets(db)?;
    ensure!(!datasets.is_empty(), "データセットがありません");
    for dataset in datasets {
        validate_coverage_metadata(&dataset.metadata, dataset.kind == "surface")?;
        if let Some(expected) = dataset.metadata.get("pointCount") {
            let actual: u64 = db.query_row(
                "SELECT count(*) FROM point_records WHERE dataset_key=?1",
                [dataset.key],
                |row| row.get(0),
            )?;
            ensure!(
                dataset.kind == "points" && actual > 0 && expected.as_u64() == Some(actual),
                "metadata.pointCountと収録地点数が一致しません: {}",
                dataset.id
            );
        }
        let sources = dataset
            .metadata
            .get("sources")
            .map(|value| {
                value
                    .as_array()
                    .context("metadata.sourcesは配列で指定してください")
            })
            .transpose()?;
        let mut source_names = std::collections::HashSet::new();
        for source in sources.into_iter().flatten() {
            let name = source
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty() && name.trim() == *name)
                .context("出典名が不正です")?;
            ensure!(source_names.insert(name), "出典名が重複しています: {name}");
        }
        let mut query =
            db.prepare("SELECT source_names_json FROM surface_layers WHERE dataset_key=?1")?;
        for row in query.query_map([dataset.key], |row| row.get::<_, String>(0))? {
            for name in serde_json::from_str::<Vec<String>>(&row?)? {
                ensure!(
                    source_names.contains(name.as_str()),
                    "レイヤーの出典が見つかりません: {name}"
                );
            }
        }
    }
    let misplaced: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM surface_layers JOIN datasets USING(dataset_key) WHERE kind<>'surface')
             OR EXISTS(SELECT 1 FROM terrain_levels JOIN datasets USING(dataset_key) WHERE kind<>'terrain')
             OR EXISTS(SELECT 1 FROM point_records JOIN datasets USING(dataset_key) WHERE kind<>'points')
             OR EXISTS(SELECT 1 FROM terrain_tiles JOIN terrain_levels USING(dataset_key,level_key)
                       WHERE x >= (1 << z) OR y >= (1 << z))",
        [], |row| row.get(0),
    )?;
    ensure!(
        !misplaced,
        "データセット種別または地形のタイル座標が不正です"
    );
    Ok(())
}

pub fn validate_surface_images(db: &Connection) -> Result<usize> {
    let mut query = db.prepare(
        "SELECT t.dataset_key,t.layer_key,t.z,t.x,t.y,t.encoding,t.payload,l.tile_pixels
         FROM surface_tiles t JOIN surface_levels l USING(dataset_key,layer_key,z)",
    )?;
    let mut rows = query.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        decode_tile(row.get(5)?, row.get_ref(6)?.as_blob()?, row.get(7)?).with_context(|| {
            format!(
                "画像タイル {}/{}/{}/{}/{}",
                row.get::<_, i64>(0).unwrap_or(-1),
                row.get::<_, i64>(1).unwrap_or(-1),
                row.get::<_, i64>(2).unwrap_or(-1),
                row.get::<_, i64>(3).unwrap_or(-1),
                row.get::<_, i64>(4).unwrap_or(-1)
            )
        })?;
        count += 1;
    }
    Ok(count)
}

pub fn merge_from(db: &mut Connection, source: &Connection) -> Result<()> {
    copy_from(db, source, None)
}

pub fn copy_surface_layers(
    db: &mut Connection,
    source: &Connection,
    layer_ids: &[String],
) -> Result<()> {
    let selection: SurfaceSelection = layer_ids.iter().map(|id| (id.clone(), None)).collect();
    ensure!(
        selection.len() == layer_ids.len(),
        "レイヤーIDを重複なく指定してください"
    );
    copy_surface_selection(db, source, &selection)
}

/// レイヤーIDに対し、収録する実XYZズームを指定する。Noneは全レベル。
pub type SurfaceSelection = BTreeMap<String, Option<Vec<u32>>>;

pub fn copy_surface_selection(
    db: &mut Connection,
    source: &Connection,
    selection: &SurfaceSelection,
) -> Result<()> {
    copy_from(db, source, Some(selection))
}

fn copy_from(
    db: &mut Connection,
    source: &Connection,
    selection: Option<&SurfaceSelection>,
) -> Result<()> {
    validate(source)?;
    let source_datasets = datasets(source)?;
    if selection.is_some() {
        ensure!(
            source_datasets.len() == 1 && source_datasets[0].kind == "surface",
            "レイヤーを抽出するには地表データセットを1件だけ収録したSQLiteを指定してください"
        );
    }
    let tx = db.transaction()?;
    for mut dataset in source_datasets {
        let filters = if let Some(selection) = selection {
            let layers = surface_layers(source, dataset.key)?;
            let selected: Vec<_> = layers
                .iter()
                .filter(|layer| selection.contains_key(&layer.id))
                .collect();
            ensure!(
                !selected.is_empty() && selected.len() == selection.len(),
                "存在するレイヤーIDを重複なく指定してください"
            );
            if let Some(sources) = dataset
                .metadata
                .get_mut("sources")
                .and_then(Value::as_array_mut)
            {
                sources.retain(|entry| {
                    entry["name"].as_str().is_some_and(|name| {
                        selected
                            .iter()
                            .any(|layer| layer.source_names.iter().any(|source| source == name))
                    })
                });
            }
            let keys = selected.iter().map(|layer| layer.key).collect::<Vec<_>>();
            let mut pairs = Vec::new();
            for layer in selected {
                let available = surface_levels(source, dataset.key, layer.key)?;
                let zooms = if let Some(zooms) = &selection[&layer.id] {
                    let unique = zooms
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>();
                    ensure!(
                        !zooms.is_empty()
                            && zooms.len() == unique.len()
                            && zooms
                                .iter()
                                .all(|z| available.iter().any(|level| level.z == *z)),
                        "レイヤー {} のズーム指定が空、重複、または未収録です",
                        layer.id
                    );
                    zooms.clone()
                } else {
                    available.into_iter().map(|level| level.z).collect()
                };
                pairs.extend(zooms.into_iter().map(|z| (layer.key, z)));
            }
            Some((
                serde_json::to_string(&keys)?,
                serde_json::to_string(&pairs)?,
            ))
        } else {
            None
        };
        tx.execute(
            "INSERT INTO datasets(id,kind,title,data_version,format_version,metadata_json)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                dataset.id,
                dataset.kind,
                dataset.title,
                dataset.data_version,
                dataset.format_version,
                dataset.metadata.to_string()
            ],
        )
        .with_context(|| format!("データセットを統合できません: {}", dataset.id))?;
        let key = tx.last_insert_rowid();
        for (select, insert, is_surface) in [
            (
                "SELECT layer_key,id,role,draw_order,source_names_json,settings_json FROM surface_layers WHERE dataset_key=?1",
                "INSERT INTO surface_layers VALUES(?1,?2,?3,?4,?5,?6,?7)",
                true,
            ),
            (
                "SELECT layer_key,z,lod,tile_pixels FROM surface_levels WHERE dataset_key=?1",
                "INSERT INTO surface_levels VALUES(?1,?2,?3,?4,?5)",
                true,
            ),
            (
                "SELECT layer_key,z,x,y,encoding,payload FROM surface_tiles WHERE dataset_key=?1",
                "INSERT INTO surface_tiles VALUES(?1,?2,?3,?4,?5,?6,?7)",
                true,
            ),
            (
                "SELECT level_key,z,settings_json FROM terrain_levels WHERE dataset_key=?1",
                "INSERT INTO terrain_levels VALUES(?1,?2,?3,?4)",
                false,
            ),
            (
                "SELECT level_key,x,y,encoding,payload FROM terrain_tiles WHERE dataset_key=?1",
                "INSERT INTO terrain_tiles VALUES(?1,?2,?3,?4,?5,?6)",
                false,
            ),
            (
                "SELECT ordinal,point_id,payload_json FROM point_records WHERE dataset_key=?1",
                "INSERT INTO point_records VALUES(?1,?2,?3,?4)",
                false,
            ),
        ] {
            let filtered_select;
            let filter = if let Some((keys, pairs)) = &filters {
                if !is_surface {
                    continue;
                }
                let (clause, filter) = if select.contains("FROM surface_layers") {
                    ("layer_key IN (SELECT value FROM json_each(?2))", keys)
                } else {
                    (
                        "(layer_key,z) IN (SELECT json_extract(value,'$[0]'),json_extract(value,'$[1]') FROM json_each(?2))",
                        pairs,
                    )
                };
                filtered_select = format!("{select} AND {clause}");
                Some(filter)
            } else {
                filtered_select = select.to_owned();
                None
            };
            let select = &filtered_select;
            let mut read = source.prepare(select)?;
            let mut write = tx.prepare(insert)?;
            let columns = read.column_count();
            let mut rows = if let Some(filter) = filter {
                read.query(params![dataset.key, filter])?
            } else {
                read.query([dataset.key])?
            };
            while let Some(row) = rows.next()? {
                let mut values = Vec::with_capacity(columns + 1);
                values.push(rusqlite::types::Value::Integer(key));
                for index in 0..columns {
                    values.push(row.get::<_, rusqlite::types::Value>(index)?);
                }
                write.execute(rusqlite::params_from_iter(values))?;
            }
        }
        if dataset.kind == "surface" && selection.is_some() {
            recompute_surface_coverage(&tx, key)?;
        }
    }
    tx.commit()?;
    Ok(())
}
