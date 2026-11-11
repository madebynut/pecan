//! Reproduce the memory bound for very dense, otherwise small Markdown input.
use pecan_document::Document;
fn main() {
    let source = "# tiny\n".repeat(500_001);
    match Document::parse(source) {
        Ok(document) => println!("headings={}", document.headings().len()),
        Err(error) => println!("rejected={error}"),
    }
}
