//! 地形SQLiteの詳細と収録量を背景で集計する。
use anyhow::{Context, Result};
use eframe::egui;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::mpsc,
};
use yamamuki_data_sqlite as db;
struct Level {
    z: u32,
    tiles: u64,
    points: u64,
    bytes: u64,
    settings: serde_json::Value,
}
struct Data {
    dataset: db::Dataset,
    levels: Vec<Level>,
    bytes: u64,
    preview: bool,
}
fn read(path: &Path) -> Result<Data> {
    if let Some(source) = yamamuki_terrain::dem_cache::Source::try_open(path)? {
        let h = source.header;
        let tiles = h.tiles.len() as u64;
        let bytes = h.tiles.values().map(|&n| n as u64).sum();
        let metadata = serde_json::json!({"source":"国土地理院の標高タイル（DEM10B）","attribution":"地理院タイル（標高タイル）を加工して作成","identity":h.identity,"view":"制御点への変換前の平滑化済み標高格子"});
        return Ok(Data {
            dataset: db::Dataset {
                key: 0,
                id: "smoothed-dem-cache".into(),
                kind: "terrain".into(),
                title: yamamuki_terrain_view::dem_preview::TITLE.into(),
                data_version: h.stamp,
                format_version: 1,
                metadata,
            },
            levels: vec![Level {
                z: 14,
                tiles,
                points: tiles * 65536,
                bytes,
                settings: serde_json::json!({"pixels":256,"heightUnit":"centimetres","missing":"i32-min","encoding":"zstd","protectionMask":true}),
            }],
            bytes: h.bytes,
            preview: true,
        });
    }

    let terrain = yamamuki_terrain::sqlite::Database::open(path)?;
    let conn = &terrain.connection;
    let dataset = db::datasets(conn)?
        .into_iter()
        .find(|d| d.key == terrain.dataset)
        .context("地形がありません")?;
    let mut levels = BTreeMap::new();
    let mut q = conn.prepare(
        "SELECT level_key,z,settings_json FROM terrain_levels WHERE dataset_key=?1 ORDER BY z",
    )?;
    let mut rows = q.query([dataset.key])?;
    while let Some(row) = rows.next()? {
        levels.insert(
            row.get::<_, i64>(0)?,
            Level {
                z: row.get(1)?,
                tiles: 0,
                points: 0,
                bytes: 0,
                settings: serde_json::from_str(&row.get::<_, String>(2)?)?,
            },
        );
    }
    let keys_by_zoom = levels
        .iter()
        .map(|(&key, level)| (level.z, key))
        .collect::<BTreeMap<_, _>>();
    // 形式ごとのヘッダー判定と格納点数は描画と同じ共通リーダーを使う。
    for (&(z, _, _), &count) in &terrain.catalog {
        levels
            .get_mut(&keys_by_zoom[&z])
            .context("収録レベルがありません")?
            .points += count as u64;
    }
    let mut q=conn.prepare("SELECT level_key,COUNT(*),SUM(length(payload)) FROM terrain_tiles WHERE dataset_key=?1 GROUP BY level_key")?;
    let mut rows = q.query([dataset.key])?;
    while let Some(row) = rows.next()? {
        let level = levels
            .get_mut(&row.get::<_, i64>(0)?)
            .context("収録レベルがありません")?;
        level.tiles = row.get(1)?;
        level.bytes = row.get(2)?;
    }
    let mut levels = levels.into_values().collect::<Vec<_>>();
    levels.sort_by_key(|l| l.z);
    Ok(Data {
        dataset,
        levels,
        bytes: std::fs::metadata(path)?.len(),
        preview: false,
    })
}
pub struct Info {
    path: PathBuf,
    pub open: bool,
    incoming: Option<mpsc::Receiver<Result<Data>>>,
    data: Option<Result<Data>>,
}
impl Info {
    pub fn new(_ctx: &egui::Context, path: PathBuf) -> Self {
        Self {
            path,
            open: false,
            incoming: None,
            data: None,
        }
    }
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        if self.data.is_none() && self.incoming.is_none() {
            let (sender, incoming) = mpsc::channel();
            let path = self.path.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = sender.send(read(&path));
                ctx.request_repaint();
            });
            self.incoming = Some(incoming);
        }
        if self.data.is_none() {
            match self.incoming.as_ref().unwrap().try_recv() {
                Ok(data) => self.data = Some(data),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.data = Some(Err(anyhow::anyhow!("集計が中断されました")))
                }
                _ => {}
            }
        }
        egui::Window::new("地形ファイルの情報")
            .open(&mut self.open)
            .default_size([820., 480.])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.strong(self.path.file_name().unwrap_or_default().to_string_lossy());
                    ui.label(self.path.display().to_string());
                    match &self.data {
                        None => {
                            ui.spinner();
                            ui.label("収録情報を集計中");
                        }
                        Some(Err(error)) => {
                            ui.colored_label(ui.visuals().error_fg_color, format!("{error:#}"));
                        }
                        Some(Ok(data)) => {
                            let set = &data.dataset;
                            ui.strong(&set.title);
                            ui.label(format!("データセットID {}", set.id));
                            ui.label(format!("データ版 {}", set.data_version));
                            if data.preview {
                                ui.label("生成用の平滑化DEMキャッシュ（配布用地形SQLiteとは異なる形式）");
                                ui.label("表示z13は元のz14タイル4枚を512×512画素で表示します。z6〜z12は閲覧用の縮小表示です。");
                            }else{ui.label(format!("SQLite コンテナ版1 / 地形形式 版{}",set.format_version));}
                            ui.label(format!("ファイルサイズ {} B", data.bytes));
                            ui.separator();
                            ui.strong("レベルごとの収録量");
                            egui::Grid::new("terrain-level-stats")
                                .striped(true)
                                .show(ui, |ui| {
                                    for title in [
                                        "レベル",
                                        "タイル数",
                                        if data.preview{"標高画素数"}else{"制御点数"},
                                        "圧縮データ",
                                        "上方 / 下方誤差",
                                    ] {
                                        ui.strong(title);
                                    }
                                    ui.end_row();
                                    for l in &data.levels {
                                        ui.label(format!("z{}", l.z));
                                        ui.label(l.tiles.to_string());
                                        ui.label(l.points.to_string());
                                        ui.label(format!("{:.2} MiB", l.bytes as f64 / 1048576.))
                                            .on_hover_text(format!("{} B", l.bytes));
                                        if data.preview{ui.label("変換前の格子");}else{ui.label(format!("{} / {} m",l.settings["errorUpMetres"],l.settings["errorDownMetres"]));}
                                        ui.end_row();
                                    }
                                });
                            ui.small(if data.preview{"元の標高格子の収録数です。閲覧用縮小キャッシュは別ファイルに保存し、このSQLiteは変更しません。"}else{"制御点数は各タイルの格納点数の合計です。共有点はタイルごとに数えます。"});
                            ui.separator();
                            ui.strong("Metadata JSON（出典・利用条件・生成設定）");
                            crate::appearance::json_view(
                                ui,
                                &serde_json::to_string_pretty(&set.metadata).unwrap_or_default(),
                            );
                            for level in &data.levels {
                                ui.collapsing(format!("z{} の設定", level.z), |ui| {
                                    crate::appearance::json_view(
                                        ui,
                                        &serde_json::to_string_pretty(&level.settings)
                                            .unwrap_or_default(),
                                    )
                                });
                            }
                        }
                    }
                });
            });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "指定された実際の全国SQLiteで情報表示を検証する"]
    fn terrain_info_reads_external_sqlite() -> Result<()> {
        let path = PathBuf::from(std::env::var("YAMAMUKI_TERRAIN_INFO_TEST_DATABASE")?);
        let data = read(&path)?;
        anyhow::ensure!(!data.levels.is_empty(), "収録レベルが空です");
        for l in &data.levels {
            eprintln!(
                "z{}: {}タイル / {}制御点 / {} B",
                l.z, l.tiles, l.points, l.bytes
            );
        }
        Ok(())
    }
    #[test]
    fn terrain_info_counts_saved_tiles_and_points() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "terrain-info-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut conn = db::rusqlite::Connection::open(&path)?;
        db::initialize(&conn)?;
        yamamuki_terrain::fixture::create(&mut conn)?;
        drop(conn);
        let data = read(&path)?;
        assert_eq!(data.levels.len(), 8);
        assert!(
            data.levels
                .iter()
                .all(|l| l.tiles == 1 && l.points > 0 && l.bytes > 0)
        );
        std::fs::remove_file(path)?;
        Ok(())
    }
}
