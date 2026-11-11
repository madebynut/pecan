//! Measure indexing and presentation budgets for long, continuously displayed code.
use std::{hint::black_box, time::Instant};

use pecan_document::{Block, Document};

fn main() {
    for target in [1_048_576, 10_485_760] {
        let line = "let value = \"中文\";\n";
        let source = format!("```rust\n{}```\n", line.repeat(target / line.len()));
        let mut timings = Vec::new();
        let mut pages = 0;
        for _ in 0..7 {
            let start = Instant::now();
            let document =
                black_box(Document::parse(source.clone()).expect("fixture fits source cap"));
            timings.push(start.elapsed());
            pages = document.blocks().len();
            assert!(document.blocks().iter().all(|block| matches!(block,
                Block::CodePage { content, .. } if content.len() <= 16 * 1024
                    && document.source()[content.clone()].matches('\n').count() <= 128)));
        }
        timings.sort_unstable();
        println!(
            "code_bytes={} rows={pages} index_median_ms={:.3}",
            source.len(),
            timings[3].as_secs_f64() * 1000.
        );
    }
}
