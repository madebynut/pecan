//! Exercise portable exports without creating a desktop window.
use anyhow::{Context as _, Result};
use pecan_export::{DocumentFormat, Snapshot};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(args.next().context("provide an input Markdown path")?);
    let output = PathBuf::from(args.next().context("provide an output directory")?);
    let input = std::fs::canonicalize(input)?;
    let document = Arc::new(pecan_document::Document::parse(std::fs::read_to_string(
        &input,
    )?)?);
    let preferences = Arc::new(pecan_config::PreferencesData::default());
    let codes = pecan_format::CodeSnapshot::prepare(
        &document,
        preferences.code_tab_size.clone(),
        &AtomicBool::new(false),
    );
    let snapshot = Snapshot {
        document,
        directory: input
            .parent()
            .context("input has no parent directory")?
            .into(),
        title: input
            .file_name()
            .context("input has no file name")?
            .to_string_lossy()
            .into(),
        preferences,
        codes,
        diagrams: pecan_mermaid::RenderQueue::new(),
        math: pecan_math::RenderQueue::new(),
        resources: None,
    };
    std::fs::create_dir_all(&output)?;
    let client = Arc::new(pecan_network::Client::new()?);
    futures_lite::future::block_on(async {
        for format in [
            DocumentFormat::Html,
            DocumentFormat::Markdown,
            DocumentFormat::Word,
            DocumentFormat::Pdf,
        ] {
            let path = output.join(format!("export.{}", format.extension()));
            let started = Instant::now();
            pecan_export::write_document(snapshot.clone(), format, &path, client.clone()).await?;
            println!(
                "{format:?}: {:?}, {} bytes",
                started.elapsed(),
                std::fs::metadata(path)?.len()
            );
        }
        Ok::<(), anyhow::Error>(())
    })
}
