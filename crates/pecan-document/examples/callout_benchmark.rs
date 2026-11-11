//! Indexing scaling for documents containing many independent callouts.
use pecan_document::Document;
use std::{hint::black_box, time::Instant};
fn main() {
    for count in [1_000, 4_000, 16_000] {
        let source =
            "# Section\n\n::: Note\n\n## Nested\n\n**Body** `code`\n\n:::\n\n".repeat(count);
        let mut samples = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let document = black_box(Document::parse(source.clone()).unwrap());
            samples.push(start.elapsed());
            assert_eq!(document.headings().len(), count * 2);
        }
        samples.sort_unstable();
        println!(
            "callouts={count} bytes={} median_ms={:.3}",
            source.len(),
            samples[2].as_secs_f64() * 1000.
        );
    }
}
