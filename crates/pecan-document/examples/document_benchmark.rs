//! Reproducible indexing and Unicode search measurements without desktop dependencies.
use pecan_document::{Document, SearchQuery};
use std::{hint::black_box, time::Instant};

fn main() {
    for target in [1_048_576, 10_485_760] {
        let paragraph =
            "## 标题 Rust 阅读器\n\n这是性能测试段落，包含中文、**强调**、`code` 与常见文字。\n\n";
        let source = paragraph.repeat(target / paragraph.len());
        let mut timings = Vec::new();
        let mut document = None;
        for _ in 0..7 {
            let start = Instant::now();
            document = Some(black_box(
                Document::parse(source.clone())
                    .expect("benchmark source is below the document cap"),
            ));
            timings.push(start.elapsed());
        }
        timings.sort_unstable();
        let document = document.expect("seven benchmark iterations produced a document");
        let query = SearchQuery::new("性能测试").expect("benchmark search query is nonempty");
        let start = Instant::now();
        let matches = black_box(query.find(&document));
        println!(
            "bytes={} chunks={} headings={} index_median_ms={:.3} search_ms={:.3} matches={}",
            document.source().len(),
            document.blocks().len(),
            document.headings().len(),
            timings[3].as_secs_f64() * 1000.,
            start.elapsed().as_secs_f64() * 1000.,
            matches.len()
        );
        let query = SearchQuery::new("不存在的全文检索目标").expect("full scan query is nonempty");
        let start = Instant::now();
        let missing = black_box(query.find(&document));
        println!(
            "full_scan_no_match_ms={:.3} matches={}",
            start.elapsed().as_secs_f64() * 1000.,
            missing.len()
        );
    }
}
