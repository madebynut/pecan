//! Reproduce resident diagram growth with distinct sources and a bounded cache.
use pecan_mermaid::{Appearance, Renderer};
use std::{hint::black_box, time::Instant};

fn snapshot(stage: &str) {
    if let Some(probe) = std::env::var_os("PECAN_MEMORY_PROBE") {
        let output = std::process::Command::new(probe)
            .arg(std::process::id().to_string())
            .output()
            .expect("memory probe runs");
        assert!(output.status.success(), "memory probe failed: {output:?}");
        println!(
            "{stage}: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        );
    }
}

fn main() {
    snapshot("empty");
    let renderer = Renderer::new();
    let start = Instant::now();
    for index in 0..64 {
        let source = format!(
            "flowchart LR\n A[打开文档 {index}] --> B[后台解析]\n B --> C[可见渲染]\n C --> D[原生图表]"
        );
        let diagram = renderer
            .render(&source, Appearance::Light)
            .expect("valid diagram");
        black_box(diagram.png(1600.).expect("bounded preview"));
        if index == 0 {
            snapshot("one diagram");
        }
    }
    println!(
        "render_64_ms={:.3} stats={:?}",
        start.elapsed().as_secs_f64() * 1000.,
        renderer.stats()
    );
    snapshot("64 cached diagrams");
    drop(renderer);
    snapshot("renderer dropped");
}
