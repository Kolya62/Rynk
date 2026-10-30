// SPDX-License-Identifier: GPL-3.0-or-later
//! Renders keyboard states offscreen with Vulkan and writes PNGs (visual check without a phone).
//!
//! cargo run --release -p rynk_core --example snapshot -- <out dir>

use rynk_core::engine::Model;
use rynk_core::gfx::{self, Canvas, Gpu};
use rynk_core::keyboard::{Cmd, Editor, Keyboard};
use rynk_core::prefs::Prefs;
use rynk_core::ui::{kbview, settings::SettingsApp, theme};
use std::sync::Arc;

const W: u32 = 1080;
const DP: f32 = 2.75;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let gpu = Gpu::new(gfx::instance(), None).expect("Vulkan");
    let fmt = wgpu::TextureFormat::Rgba8Unorm;
    let mut canvas = Canvas::new(&gpu, fmt);

    let shots: Vec<(&str, &[&str], bool, &str)> = vec![
        ("ru_light", &["ru", "en"], false, "пр"),
        ("ru_dark", &["ru", "en"], true, ""),
        ("en_light", &["en"], false, "helo"),
        ("en_dark", &["en"], true, "how are "),
        ("en_popup_light", &["en"], false, "caf"),
        ("ru_landscape", &["ru", "en"], false, "пр"),
        ("en_tablet", &["en"], false, "helo"),
        ("hi_light", &["hi"], false, ""),
        ("zh_light", &["zh-Hans"], false, "zhongguoren"),
        ("zhuyin_dark", &["zh-TW"], true, "ㄋㄧˇㄏㄠˇ"),
        ("ja_light", &["ja"], false, "nihon"),
        ("ar_light", &["ar"], false, "سلا"),
    ];
    for (name, langs, dark, typed) in shots {
        let mut p = Prefs::defaults(&[]);
        p.languages = langs.iter().map(|s| s.to_string()).collect();
        let mut kb = Keyboard::new(p);
        // phone portrait by default; a landscape phone and a landscape 10" tablet
        let (w, dp, screen_h) = match name {
            "ru_landscape" => (2340u32, 2.8125f32, 1080.0f32),
            "en_tablet" => (2560, 2.0, 1600.0),
            _ => (W, DP, 2340.0),
        };
        kb.set_screen_height(screen_h);
        let h = kb.measure(w as f32, dp, 0.0) as u32;
        kb.start_input(Editor::from_android(0x4001, 3), false);
        for c in kb.take_commands() {
            if let Cmd::LoadModel(path) = c {
                let file = format!("{}/../app/src/main/assets/{path}", env!("CARGO_MANIFEST_DIR"));
                let Ok(b) = std::fs::read(file) else { continue };
                if path.ends_with(".rcd") {
                    kb.conv_loaded(&path, Arc::new(rynk_core::engine::cjk::ConvDict::decode(&b).unwrap()));
                } else {
                    kb.model_loaded(&path, Arc::new(Model::from_bytes(&b).unwrap()));
                }
            }
        }
        for ch in typed.chars() {
            let Some(key) = kb.keys.iter().find(|k| k.label.to_lowercase() == ch.to_string()).cloned() else {
                kb.type_text(&ch.to_string(), None);
                continue;
            };
            kb.touch_down(1, key.rect.cx(), key.rect.cy(), 0.0);
            kb.touch_up(1, key.rect.cx(), key.rect.cy(), 30.0);
        }
        let popup = match name { "ru_dark" => Some("е"), "en_popup_light" => Some("e"), _ => None };
        if let Some(p) = popup {
            // show the long-press popup
            let key = kb.keys.iter().find(|k| k.label.to_lowercase() == p).unwrap().clone();
            kb.touch_down(1, key.rect.cx(), key.rect.cy(), 0.0);
            kb.tick(1000.0);
        }
        let scripts: Vec<&str> = langs.iter().filter_map(|c| rynk_core::layout::LangDef::by_code(c)).map(|l| l.script).collect();
        canvas.ctx.set_fonts(rynk_core::ui::fonts::definitions(&scripts));
        let pal = if dark { theme::DARK } else { theme::LIGHT };
        // two passes: the first one loads the fonts
        let mut first = canvas.run([w, h], dp, Vec::new(), 0.0, |ui| kbview::paint(&kb, ui.painter(), pal));
        let mut full = canvas.run([w, h], dp, Vec::new(), 0.0, |ui| kbview::paint(&kb, ui.painter(), pal));
        first.textures_delta.append(std::mem::take(&mut full.textures_delta));
        full.textures_delta = first.textures_delta;
        save(&gpu, &mut canvas, full, [w, h], pal.bg, &format!("{out}/{name}.png"));
    }

    // settings screen
    canvas.ctx.set_fonts(rynk_core::ui::fonts::definitions(&[]));
    let mut app = SettingsApp::new(Prefs::defaults(&["ru-RU".into()]), "ru-RU");
    SettingsApp::style(&canvas.ctx, false);
    let size = [W, 2200];
    let mut deltas = egui::TexturesDelta::default();
    let mut full = None;
    for _ in 0..3 {
        let mut f = canvas.run(size, DP, Vec::new(), 0.0, |ui| app.ui(ui, 24.0, 24.0));
        // font atlas updates of every pass must reach the GPU
        deltas.append(std::mem::take(&mut f.textures_delta));
        full = Some(f);
    }
    let mut full = full.unwrap();
    full.textures_delta = deltas;
    save(&gpu, &mut canvas, full, size, egui::Color32::WHITE, &format!("{out}/settings.png"));
}

fn save(gpu: &Gpu, canvas: &mut Canvas, full: egui::FullOutput, size: [u32; 2], clear: egui::Color32, path: &str) {
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    canvas.draw(gpu, &view, size, full, clear);
    let row = (size[0] * 4).div_ceil(256) * 256;
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * size[1]) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None } },
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    gpu.queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = buf.slice(..).get_mapped_range().expect("mapped").to_vec();
    let mut rgba = Vec::with_capacity((size[0] * size[1] * 4) as usize);
    for y in 0..size[1] {
        let s = (y * row) as usize;
        rgba.extend_from_slice(&data[s..s + (size[0] * 4) as usize]);
    }
    std::fs::write(path, png(size[0], size[1], &rgba)).unwrap();
    println!("{path}");
}

/// Minimal PNG writer (uncompressed deflate blocks)
fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc(&c).to_be_bytes());
    }
    let mut raw = Vec::with_capacity(rgba.len() + h as usize);
    for y in 0..h as usize {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * w as usize * 4..(y + 1) * w as usize * 4]);
    }
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&rynk_core::engine::lm::adler32(&raw).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}
