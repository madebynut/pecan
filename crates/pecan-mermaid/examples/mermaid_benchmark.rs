//! Measures uncached native diagrams and cached vector reuse separately.
use pecan_mermaid::{Appearance, Renderer};
use std::{hint::black_box, time::Instant};

fn main() {
    let renderer = Renderer::new();
    let source = std::env::args_os()
        .nth(1)
        .map(std::fs::read_to_string)
        .transpose()
        .expect("benchmark source must be readable")
        .unwrap_or_else(|| {
            "flowchart LR\n A[打开文档] --> B[后台解析]\n B --> C[可见渲染]\n C --> D[原生图表]"
                .into()
        });
    let start = Instant::now();
    let diagram = renderer
        .render(&source, Appearance::Light)
        .expect("benchmark source is valid Mermaid");
    println!(
        "first_render_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.
    );
    let start = Instant::now();
    black_box(diagram.png(1600.).expect("benchmark preview is bounded"));
    println!("png_1600_ms={:.3}", start.elapsed().as_secs_f64() * 1000.);
    let start = Instant::now();
    for _ in 0..10_000 {
        black_box(
            renderer
                .render(&source, Appearance::Light)
                .expect("cached diagram remains valid"),
        );
    }
    println!(
        "cached_lookup_us={:.3} stats={:?}",
        start.elapsed().as_secs_f64() * 100.,
        renderer.stats()
    );
}
