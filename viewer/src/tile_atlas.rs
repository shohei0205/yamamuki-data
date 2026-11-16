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
    edges: Arc<Edges>,
}
// 隣接更新には外周だけを残し、全タイルの展開画像は保持しない。
struct Edges {
    size: [usize; 2],
    top: Vec<egui::Color32>,
    bottom: Vec<egui::Color32>,
    left: Vec<egui::Color32>,
    right: Vec<egui::Color32>,
}
impl Edges {
    fn new(image: &ColorImage) -> Self {
        let [w, h] = image.size;
        Self {
            size: image.size,
            top: image.pixels[..w].to_vec(),
            bottom: image.pixels[(h - 1) * w..].to_vec(),
            left: (0..h).map(|y| image.pixels[y * w]).collect(),
            right: (0..h).map(|y| image.pixels[y * w + w - 1]).collect(),
        }
    }
    fn patch(&self, dx: i32, dy: i32) -> Option<([usize; 2], ColorImage)> {
        let [w, h] = self.size;
        let (at, size, pixels) = match (dx, dy) {
            (-1, 0) => ([0, 1], [1, h], self.right.clone()),
            (1, 0) => ([w + 1, 1], [1, h], self.left.clone()),
            (0, -1) => ([1, 0], [w, 1], self.bottom.clone()),
            (0, 1) => ([1, h + 1], [w, 1], self.top.clone()),
            (-1 | 1, -1 | 1) => {
                let row = if dy < 0 { &self.bottom } else { &self.top };
                (
                    [
                        if dx < 0 { 0 } else { w + 1 },
                        if dy < 0 { 0 } else { h + 1 },
                    ],
                    [1, 1],
                    vec![row[if dx < 0 { w - 1 } else { 0 }]],
                )
            }
            _ => return None,
        };
        let mut image = ColorImage::new(size, egui::Color32::TRANSPARENT);
        image.pixels = pixels;
        Some((at, image))
    }
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
        let edges = Arc::new(Edges::new(&image));
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
            edges,
        }
    }
    /// 同じズームの隣接タイルの端を余白へ転送する。UVと画像IDは維持する。
    pub fn set_neighbor(&self, neighbor: &Self, dx: i32, dy: i32) {
        if self.edges.size != neighbor.edges.size {
            return;
        }
        let Some(([px, py], image)) = neighbor.edges.patch(dx, dy) else {
            return;
        };
        let [w, h] = self.edges.size;
        let slot = self.lease.slot;
        let at = [slot % GRID * (w + 2) + px, slot / GRID * (h + 2) + py];
        self.lease
            .page
            .texture
            .lock()
            .unwrap()
            .set_partial(at, image, self.lease.page.options);
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
    #[test]
    fn all_neighbor_edges_and_corners_use_the_neighbor_pixels() {
        let mut source = ColorImage::new([3, 3], egui::Color32::BLACK);
        for (i, pixel) in source.pixels.iter_mut().enumerate() {
            *pixel = egui::Color32::from_gray(i as u8 * 20);
        }
        let edges = Edges::new(&source);
        for (direction, at, indices) in [
            ((-1, 0), [0, 1], vec![2, 5, 8]),
            ((1, 0), [4, 1], vec![0, 3, 6]),
            ((0, -1), [1, 0], vec![6, 7, 8]),
            ((0, 1), [1, 4], vec![0, 1, 2]),
            ((-1, -1), [0, 0], vec![8]),
            ((1, -1), [4, 0], vec![6]),
            ((-1, 1), [0, 4], vec![2]),
            ((1, 1), [4, 4], vec![0]),
        ] {
            let (position, patch) = edges.patch(direction.0, direction.1).unwrap();
            assert_eq!(position, at);
            assert_eq!(
                patch.pixels,
                indices
                    .into_iter()
                    .map(|i| source.pixels[i])
                    .collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn late_neighbors_update_both_gutters_without_replacing_shared_texture_or_uv() {
        let ctx = egui::Context::default();
        let mut images = None;
        let _ = ctx.run(Default::default(), |ctx| {
            images = Some([
                Image::load(
                    ctx,
                    "left",
                    ColorImage::new([2, 2], egui::Color32::RED),
                    egui::TextureOptions::LINEAR,
                ),
                Image::load(
                    ctx,
                    "right",
                    ColorImage::new([2, 2], egui::Color32::BLUE),
                    egui::TextureOptions::LINEAR,
                ),
            ]);
        });
        let [left, right] = images.unwrap();
        let original = (left.id(), left.uv(crate::lod_fade::FULL_UV));
        let output = ctx.run(Default::default(), |_| {
            left.set_neighbor(&right, 1, 0);
            right.set_neighbor(&left, -1, 0);
        });
        assert_eq!(original, (left.id(), left.uv(crate::lod_fade::FULL_UV)));
        assert_eq!(left.id(), right.id());
        let patches = output
            .textures_delta
            .set
            .iter()
            .filter_map(|(id, delta)| {
                assert_eq!(*id, left.id());
                delta.pos.map(|pos| (pos, &delta.image))
            })
            .collect::<Vec<_>>();
        assert_eq!(patches.len(), 2);
        for (position, data) in patches {
            let egui::ImageData::Color(image) = data else {
                panic!("カラー画像が必要");
            };
            assert_eq!(image.size, [1, 2]);
            let expected = match position {
                [3, 1] => egui::Color32::BLUE,
                [4, 1] => egui::Color32::RED,
                other => panic!("余白以外へ転送: {other:?}"),
            };
            assert!(image.pixels.iter().all(|p| *p == expected));
        }
    }
}
