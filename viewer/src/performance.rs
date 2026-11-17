//! 描画を増やさず、実際の更新間隔とGPUアロケーターの確保量を表示する。
use crate::appearance;
use eframe::{
    egui::{self, Rect},
    egui_wgpu::RenderState,
};
use std::collections::VecDeque;

#[derive(Default, Clone, Copy)]
pub struct CacheStats {
    pub maps: usize,
    pub map_pending: usize,
    pub surfaces: usize,
    pub surface_bytes: usize,
    pub surface_pending: usize,
}

#[derive(Default)]
pub struct Overlay {
    samples: VecDeque<f64>,
    last_gpu_sample: Option<f64>,
    gpu_memory: Option<(u64, u64)>,
    adapter: Option<String>,
}

impl Overlay {
    fn record(&mut self, now: f64) {
        if self.samples.back().is_some_and(|last| *last == now) {
            return;
        }
        if self.samples.back().is_some_and(|last| now < *last) {
            self.samples.clear();
        }
        self.samples.push_back(now);
        while self.samples.len() > 2 && self.samples.front().is_some_and(|first| now - *first > 1.)
        {
            self.samples.pop_front();
        }
    }
    fn frame_ms(&self) -> Option<f64> {
        let span = self.samples.back()? - self.samples.front()?;
        (self.samples.len() > 1 && span > 0.)
            .then(|| 1000. * span / (self.samples.len() - 1) as f64)
    }
    pub fn paint(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        bottom: f32,
        state: Option<&RenderState>,
        cpu_seconds: Option<f32>,
        cache: CacheStats,
    ) {
        let now = ui.input(|input| input.time);
        self.record(now);
        if self
            .last_gpu_sample
            .is_none_or(|last| now - last >= 1. || now < last)
        {
            self.last_gpu_sample = Some(now);
            if let Some(state) = state {
                if self.adapter.is_none() {
                    let info = state.adapter.get_info();
                    self.adapter = Some(format!("{} / {:?}", info.name, info.backend));
                }
                self.gpu_memory = state
                    .device
                    .generate_allocator_report()
                    .map(|report| (report.total_allocated_bytes, report.total_reserved_bytes));
            }
        }
        let frame = self.frame_ms().map_or_else(
            || "FPS — / フレーム —".into(),
            |ms| format!("FPS {:.1} / フレーム {ms:.1} ms", 1000. / ms),
        );
        let cpu = cpu_seconds.map_or_else(|| "—".into(), |s| format!("{:.1} ms", s * 1000.));
        let gpu = self.gpu_memory.map_or_else(
            || {
                if state.is_some() {
                    "GPUメモリ 取得非対応".into()
                } else {
                    "GPUメモリ 準備中".into()
                }
            },
            |(used, reserved)| {
                format!(
                    "GPU 確保 {:.1} MiB / 予約 {:.1} MiB",
                    mib(used),
                    mib(reserved)
                )
            },
        );
        let text = format!(
            "{frame} / CPU {cpu}\n{gpu}\n地図 {}枚 (待ち{}) / 地表 {}画像 {:.1} MiB (待ち{})",
            cache.maps,
            cache.map_pending,
            cache.surfaces,
            mib(cache.surface_bytes as u64),
            cache.surface_pending
        );
        let painter = ui.painter();
        let label = painter.layout(
            text,
            egui::FontId::monospace(11.),
            appearance::INK,
            (rect.width() - 34.).max(100.),
        );
        let position = rect.right_bottom() - label.size() - egui::vec2(17., 14. + bottom);
        let badge = Rect::from_min_size(position, label.size()).expand2(egui::vec2(7., 4.));
        painter.rect_filled(badge, 7, appearance::PAPER);
        painter.rect_stroke(
            badge,
            7,
            egui::Stroke::new(1_f32, appearance::LINE),
            egui::StrokeKind::Inside,
        );
        painter.galley(position, label, appearance::INK);
        ui.interact(badge,ui.id().with("performance-information"),egui::Sense::hover()).on_hover_text(format!(
            "{}\nFPSは実際の描画更新頻度です。無操作時は連続描画しません。\nCPUは前フレームのアプリ処理時間です。GPUの実行時間ではありません。\nGPU確保・予約はこのアプリのwgpuアロケーターの報告値です。GPU全体の使用量・総VRAM容量ではありません。\n地表画像のMiBは画像サイズから数えた容量で、GPUメモリの内訳とは一致しません。\nGPU統計は描画時に最大毎秒1回取得します。",
            self.adapter.as_deref().unwrap_or("GPU準備中")));
    }
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024. * 1024.)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_rate_ignores_duplicate_passes_and_recovers_after_idle() {
        let mut stats = Overlay::default();
        for frame in 0..120 {
            let time = frame as f64 / 60.;
            stats.record(time);
            stats.record(time);
        }
        assert!((stats.frame_ms().unwrap() - 1000. / 60.).abs() < 1e-8);
        assert!(stats.samples.len() <= 62);
        stats.record(10.);
        assert!(stats.frame_ms().unwrap() > 1000.);
        for frame in 1..120 {
            stats.record(10. + frame as f64 / 60.);
        }
        assert!((stats.frame_ms().unwrap() - 1000. / 60.).abs() < 1e-8);
        stats.record(0.);
        assert!(stats.frame_ms().is_none());
    }
    #[test]
    fn overlay_is_inside_the_map_and_does_not_keep_redrawing() {
        let ctx = egui::Context::default();
        let mut overlay = Overlay::default();
        let rect = Rect::from_min_max(egui::pos2(100., 80.), egui::pos2(980., 680.));
        let mut last = None;
        for frame in 0..5 {
            last = Some(ctx.run(
                egui::RawInput {
                    time: Some(frame as f64 / 60.),
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000., 700.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        overlay.paint(
                            ui,
                            rect,
                            60.,
                            None,
                            Some(0.002),
                            CacheStats {
                                maps: 12,
                                map_pending: 2,
                                surfaces: 30,
                                surface_bytes: 1024 * 1024,
                                surface_pending: 3,
                            },
                        );
                    });
                },
            ));
        }
        let output = last.unwrap();
        let label = output
            .shapes
            .iter()
            .find_map(|s| {
                let egui::Shape::Text(text) = &s.shape else {
                    return None;
                };
                text.galley.text().contains("GPUメモリ").then_some(text)
            })
            .expect("右下の統計表示");
        let bounds = Rect::from_min_size(label.pos, label.galley.size());
        assert!(rect.contains_rect(bounds));
        assert!(bounds.bottom() < rect.bottom() - 60.);
        assert!(label.galley.text().contains("待ち3"));
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                > std::time::Duration::from_secs(1),
            "統計表示だけで再描画を要求した"
        );
    }

    #[test]
    #[ignore = "実GPUのアロケーター統計を確認する"]
    fn gpu_memory_report_has_meaningful_allocation_sizes() {
        let instance = eframe::wgpu::Instance::new(&eframe::wgpu::InstanceDescriptor {
            backends: if cfg!(windows) {
                eframe::wgpu::Backends::DX12
            } else {
                eframe::wgpu::Backends::all()
            },
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let _buffer = device.create_buffer(&eframe::wgpu::BufferDescriptor {
            label: Some("統計確認用"),
            size: 1024 * 1024,
            usage: eframe::wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        if let Some(report) = device.generate_allocator_report() {
            eprintln!(
                "GPU確保 {} bytes / 予約 {} bytes / {:?}",
                report.total_allocated_bytes,
                report.total_reserved_bytes,
                adapter.get_info()
            );
            assert!(report.total_allocated_bytes >= 1024 * 1024);
            assert!(report.total_reserved_bytes >= report.total_allocated_bytes);
        } else {
            eprintln!(
                "このバックエンドはメモリ報告に非対応: {:?}",
                adapter.get_info()
            );
        }
    }
}
