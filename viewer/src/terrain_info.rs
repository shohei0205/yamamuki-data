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
}
fn read(path: &Path) -> Result<Data> {
    let conn = db::open(path)?;
    let dataset = db::datasets(&conn)?
        .into_iter()
        .find(|d| d.kind == "terrain")
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
    let mut q=conn.prepare("SELECT level_key,length(payload),substr(payload,1,16) FROM terrain_tiles WHERE dataset_key=?1")?;
    let mut rows = q.query([dataset.key])?;
    while let Some(row) = rows.next()? {
        let level = levels
            .get_mut(&row.get::<_, i64>(0)?)
            .context("収録レベルがありません")?;
        let header: Vec<u8> = row.get(2)?;
        anyhow::ensure!(
            header.len() == 16 && &header[..4] == b"NTS1",
            "未対応の地形タイルです"
        );
        level.tiles += 1;
        level.bytes += row.get::<_, u64>(1)?;
        level.points += u32::from_le_bytes(header[12..16].try_into()?) as u64;
    }
    let mut levels = levels.into_values().collect::<Vec<_>>();
    levels.sort_by_key(|l| l.z);
    Ok(Data {
        dataset,
        levels,
        bytes: std::fs::metadata(path)?.len(),
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
            .default_size([650., 480.])
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
                            ui.label(format!(
                                "SQLite コンテナ版1 / 地形形式 版{}",
                                set.format_version
                            ));
                            ui.label(format!("ファイルサイズ {} B", data.bytes));
                            ui.label(
                                "自然近傍補間 / XY 1/16 m・標高 1/8 m / Rice併用の独立・差分格納",
                            );
                            ui.separator();
                            ui.strong("レベルごとの収録量");
                            egui::Grid::new("terrain-level-stats")
                                .striped(true)
                                .show(ui, |ui| {
                                    for title in [
                                        "レベル",
                                        "タイル数",
                                        "制御点数",
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
                                        ui.label(format!(
                                            "{} / {} m",
                                            l.settings["errorUpMetres"],
                                            l.settings["errorDownMetres"]
                                        ));
                                        ui.end_row();
                                    }
                                });
                            ui.small(
                                "制御点数は各タイルの格納点数の合計です。共有点の重複を含みます。",
                            );
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
