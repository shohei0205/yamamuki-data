//! 深度テスト済みの描画位置を1ピクセルだけ読み、実際に見えている地形を選択する。
use crate::{selection::Camera, shared_data::WORLD};
use eframe::{egui, wgpu};
use std::sync::{Mutex, mpsc};
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub world: [f64; 2],
    pub height: f64,
}
type Reply = mpsc::Sender<anyhow::Result<Option<Hit>>>;
struct Request {
    uv: [f32; 2],
    reply: Reply,
}
struct Pending {
    buffer: wgpu::Buffer,
    center: [f64; 2],
    reply: Reply,
    receive: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
}
#[derive(Default)]
pub struct State {
    request: Option<Request>,
    pending: Option<Pending>,
}
pub fn request(
    state: &Mutex<State>,
    ctx: &egui::Context,
    uv: [f32; 2],
) -> mpsc::Receiver<anyhow::Result<Option<Hit>>> {
    let (reply, receive) = mpsc::channel();
    state.lock().unwrap().request = Some(Request { uv, reply });
    ctx.request_repaint();
    receive
}
fn decode(value: [f32; 4], center: [f64; 2]) -> Option<Hit> {
    if value[3] == 0. || !value.iter().all(|v| v.is_finite()) {
        return None;
    }
    let world = [
        (center[0] + f64::from(value[0])) / WORLD,
        (center[1] + f64::from(value[1])) / WORLD,
    ];
    if !world.iter().all(|v| (0. ..=1.).contains(v)) {
        return None;
    }
    Some(Hit {
        world,
        height: f64::from(value[3].abs()) - 10000.,
    })
}
pub fn poll(state: &Mutex<State>, device: &wgpu::Device, ctx: &egui::Context) {
    let mut state = state.lock().unwrap();
    if let Some(pending) = &mut state.pending {
        if pending.receive.is_none() {
            let (send, receive) = mpsc::channel();
            let wake = ctx.clone();
            pending
                .buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |v| {
                    let _ = send.send(v);
                    wake.request_repaint();
                });
            pending.receive = Some(receive);
        }
    }
    let _ = device.poll(wgpu::Maintain::Poll);
    let done = state
        .pending
        .as_ref()
        .and_then(|p| p.receive.as_ref()?.try_recv().ok());
    if let Some(done) = done {
        let pending = state.pending.take().unwrap();
        let result = match done {
            Ok(()) => {
                let range = pending.buffer.slice(..).get_mapped_range();
                let v = bytemuck::cast_slice::<u8, f32>(&range);
                Ok(decode([v[0], v[1], v[2], v[3]], pending.center))
            }
            Err(e) => Err(anyhow::anyhow!("3Dヒットテストの読み戻しに失敗: {e}")),
        };
        pending.buffer.unmap();
        let _ = pending.reply.send(result);
        ctx.request_repaint();
    }
    if state.pending.is_some() || state.request.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}
pub fn miss(state: &Mutex<State>) {
    if let Some(request) = state.lock().unwrap().request.take() {
        let _ = request.reply.send(Ok(None));
    }
}
pub fn encode(
    state: &Mutex<State>,
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    camera: &Camera,
) {
    let mut state = state.lock().unwrap();
    if state.pending.is_some() {
        return;
    }
    let Some(request) = state.request.take() else {
        return;
    };
    if request
        .uv
        .iter()
        .any(|v| !v.is_finite() || !(0. ..1.).contains(v))
    {
        let _ = request.reply.send(Ok(None));
        return;
    }
    let size = texture.size();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("クリック位置の3D座標"),
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: (request.uv[0] * size.width as f32) as u32,
                y: (request.uv[1] * size.height as f32) as u32,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    state.pending = Some(Pending {
        buffer,
        center: camera.center,
        reply: request.reply,
        receive: None,
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_reads_only_the_selected_pixel_and_ignores_sky() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            eprintln!("GPUがないためGPU試験を省略します");
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let data = [[0f32; 4], [100., 200., 1., 11234.5], [0.; 4], [0.; 4]];
        queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(32),
                rows_per_image: None,
            },
            texture.size(),
        );
        let state = Mutex::new(State::default());
        let ctx = egui::Context::default();
        let camera = Camera::default();
        for (uv, expected) in [([0.75, 0.25], true), ([0.25, 0.75], false)] {
            let receive = request(&state, &ctx, uv);
            let mut encoder = device.create_command_encoder(&Default::default());
            encode(&state, &device, &mut encoder, &texture, &camera);
            queue.submit([encoder.finish()]);
            poll(&state, &device, &ctx);
            let _ = device.poll(wgpu::Maintain::Wait);
            poll(&state, &device, &ctx);
            let hit = receive.recv().unwrap().unwrap();
            assert_eq!(hit.is_some(), expected);
            if let Some(hit) = hit {
                assert_eq!(hit.height, 1234.5);
                assert!((hit.world[0] - (camera.center[0] + 100.) / WORLD).abs() < 1e-12);
            }
        }
    }
    #[test]
    fn rendered_hit_keeps_world_position_and_height() {
        let center = [WORLD * 0.8, WORLD * 0.4];
        for sign in [1., -1.] {
            let hit = decode([100., -50., 1., sign * 11234.5], center).unwrap();
            assert!((hit.height - 1234.5).abs() < 1e-6);
            assert!((hit.world[0] - (center[0] + 100.) / WORLD).abs() < 1e-12);
            assert!((hit.world[1] - (center[1] - 50.) / WORLD).abs() < 1e-12);
        }
        assert!(decode([0.; 4], center).is_none());
        assert!(decode([f32::NAN, 0., 1., 10000.], center).is_none());
    }
}
