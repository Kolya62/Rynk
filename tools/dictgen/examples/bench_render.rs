//! Frame cost, single-threaded vs banded on several cores:
//! `cargo run --release -p dictgen --example bench_render`
use rynk_core::keyboard::KeyboardEngine;
use rynk_core::render::canvas::{rasterize_parallel, raster_threads, Canvas};
use rynk_core::render::theme::{RynkTheme, ThemeId};
use rynk_core::render::KeyboardRenderer;
use std::time::Instant;

fn main() {
    for (w, h) in [(1080usize, 770usize), (1440, 1030)] {
        let mut engine = KeyboardEngine::new(w as f32, h as f32, w as f32 / 392.0);
        engine.update_suggestions();
        let mut renderer = KeyboardRenderer::new(RynkTheme::from_id(ThemeId::Dark));
        let suggestions = engine.cached_suggestions.clone();
        let mut pixels = vec![0u32; w * h];
        let threads = raster_threads(w * h);
        for (name, n) in [("1 thread", 1), ("banded", threads)] {
            let mut times = Vec::new();
            for frame in 0..300u64 {
                let t = Instant::now();
                let mut rec = Canvas::recording(w, h);
                renderer.render(&mut rec, &engine, 1000 + frame * 16, &suggestions);
                rasterize_parallel(&rec.take_commands(), &mut pixels, w, h, w, n);
                times.push(t.elapsed().as_micros() as u64);
            }
            times.sort_unstable();
            println!("{w}x{h} {name:<8} ({n} threads): p50 {}µs p95 {}µs", times[150], times[285]);
        }
    }
}
