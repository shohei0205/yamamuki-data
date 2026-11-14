//! 元解像度を保ったまま16タイルを一枚に格納し、eguiのサンプラー確保数を抑える。
use eframe::egui::{self, ColorImage, Rect, TextureHandle};
use std::sync::{Arc, Mutex, Weak};
const GRID: usize = 4;
#[derive(Default)]
struct Pool {
    pages: Vec<Weak<Page>>,
}
struct Page {
    texture: Mutex<TextureHandle>,
    slots: Mutex<[Option<u64>; GRID * GRID]>,
    size: [usize; 2],
    options: egui::TextureOptions,
    ctx: egui::Context,
}
struct Lease {
    page: Arc<Page>,
    slot: usize,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.page.slots.lock().unwrap()[self.slot] = Some(self.page.ctx.cumulative_pass_nr() + 3);
    }
}
#[derive(Clone)]
pub struct Image {
    lease: Arc<Lease>,
    area: Rect,
}
fn bordered(image: &ColorImage) -> ColorImage {
    let [w, h] = image.size;
    let mut out = ColorImage::new([w + 2, h + 2], egui::Color32::TRANSPARENT);
    for y in 0..h + 2 {
        for x in 0..w + 2 {
            out.pixels[y * (w + 2) + x] =
                image.pixels[y.saturating_sub(1).min(h - 1) * w + x.saturating_sub(1).min(w - 1)];
        }
    }
    out
}
impl Image {
    pub fn load(
        ctx: &egui::Context,
        _name: impl Into<String>,
        image: ColorImage,
        options: egui::TextureOptions,
    ) -> Self {
        let pool = ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Arc<Mutex<Pool>>>(egui::Id::new("native-tile-atlas"))
                .clone()
        });
        let mut pool = pool.lock().unwrap();
        let frame = ctx.cumulative_pass_nr();
        pool.pages.retain(|p| p.strong_count() > 0);
        let mut chosen = None;
        for page in pool.pages.iter().filter_map(Weak::upgrade) {
            if page.size != image.size || page.options != options {
                continue;
            }
            let mut slots = page.slots.lock().unwrap();
            if let Some(i) = slots.iter().position(|v| v.is_some_and(|t| t <= frame)) {
                slots[i] = None;
                chosen = Some((page.clone(), i));
                break;
            }
        }
        let (page, slot) = chosen.unwrap_or_else(|| {
            let size = image.size;
            let full = [(size[0] + 2) * GRID, (size[1] + 2) * GRID];
            let texture = ctx.load_texture(
                "元解像度のタイル集約画像",
                ColorImage::new(full, egui::Color32::TRANSPARENT),
                options,
            );
            let mut slots = [Some(0); GRID * GRID];
            slots[0] = None;
            let page = Arc::new(Page {
                texture: Mutex::new(texture),
                slots: Mutex::new(slots),
                size,
                options,
                ctx: ctx.clone(),
            });
            pool.pages.push(Arc::downgrade(&page));
            (page, 0)
        });
        let [w, h] = image.size;
        let x = slot % GRID * (w + 2);
        let y = slot / GRID * (h + 2);
        page.texture
            .lock()
            .unwrap()
            .set_partial([x, y], bordered(&image), options);
        let full = [(w + 2) * GRID, (h + 2) * GRID];
        let area = Rect::from_min_max(
            egui::pos2(
                (x + 1) as f32 / full[0] as f32,
                (y + 1) as f32 / full[1] as f32,
            ),
            egui::pos2(
                (x + 1 + w) as f32 / full[0] as f32,
                (y + 1 + h) as f32 / full[1] as f32,
            ),
        );
        Self {
            lease: Arc::new(Lease { page, slot }),
            area,
        }
    }
    pub fn id(&self) -> egui::TextureId {
        self.lease.page.texture.lock().unwrap().id()
    }
    pub fn uv(&self, uv: Rect) -> Rect {
        let at = |p: egui::Pos2| self.area.min + self.area.size() * p.to_vec2();
        Rect::from_min_max(at(uv.min), at(uv.max))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thousands_of_tiles_upload_without_exhausting_egui_samplers() {
        use eframe::wgpu;
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: if cfg!(windows) {
                wgpu::Backends::DX12
            } else {
                wgpu::Backends::PRIMARY
            },
            ..Default::default()
        });
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            eprintln!("GPUがないためGPU試験を省略します");
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let mut renderer = eframe::egui_wgpu::Renderer::new(
            &device,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            None,
            1,
            false,
        );
        let ctx = egui::Context::default();
        let mut images = Vec::new();
        let output = ctx.run(Default::default(), |_| {
            for i in 0..2304 {
                images.push(Image::load(
                    &ctx,
                    format!("tile{i}"),
                    ColorImage::new([4, 4], egui::Color32::from_rgb(i as u8, 30, 80)),
                    egui::TextureOptions::LINEAR,
                ));
            }
        });
        for (id, delta) in &output.textures_delta.set {
            renderer.update_texture(&device, &queue, *id, delta);
        }
        let _ = device.poll(wgpu::Maintain::Wait);
        assert_eq!(
            images
                .iter()
                .map(Image::id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            144
        );
        assert!(images.iter().all(|i| renderer.texture(&i.id()).is_some()));
    }
    #[test]
    fn many_tiles_share_pages_and_keep_native_uv_gutters() {
        let ctx = egui::Context::default();
        let full = Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.));
        let images = (0..2304)
            .map(|i| {
                Image::load(
                    &ctx,
                    format!("tile{i}"),
                    ColorImage::new([4, 4], egui::Color32::WHITE),
                    egui::TextureOptions::LINEAR,
                )
            })
            .collect::<Vec<_>>();
        let ids = images
            .iter()
            .map(Image::id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), 144);
        assert_eq!(images[0].id(), images[15].id());
        assert_ne!(images[0].id(), images[16].id());
        let a = images[0].uv(full);
        let b = images[1].uv(full);
        assert!(a.max.x < b.min.x);
        assert!((a.width() * 24. - 4.).abs() < 1e-5);
        let subset = images[0].uv(Rect::from_min_max(
            egui::pos2(0.25, 0.25),
            egui::pos2(0.75, 0.75),
        ));
        assert!((subset.width() * 24. - 2.).abs() < 1e-5);
    }
    #[test]
    fn border_repeats_the_outermost_pixels() {
        let image = ColorImage::from_rgba_unmultiplied([2, 1], &[255, 0, 0, 255, 0, 0, 255, 255]);
        let b = bordered(&image);
        assert_eq!(b.size, [4, 3]);
        for row in b.pixels.chunks_exact(4) {
            assert_eq!(
                row,
                &[
                    image.pixels[0],
                    image.pixels[0],
                    image.pixels[1],
                    image.pixels[1]
                ]
            );
        }
    }
}
