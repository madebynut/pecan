//! Render a Mermaid file for visual checks; an optional --dark selects the dark palette.
use pecan_mermaid::{Appearance, Renderer};
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(args.next().ok_or("expected a Mermaid source path")?);
    let output = PathBuf::from(args.next().ok_or("expected an output PNG path")?);
    let appearance = if args.any(|arg| arg == "--dark") {
        Appearance::Dark
    } else {
        Appearance::Light
    };
    let source = fs::read_to_string(input)?;
    let renderer = Renderer::new();
    let diagram = renderer.render(&source, appearance)?;
    fs::write(output.with_extension("svg"), diagram.svg())?;
    fs::write(output, diagram.png(2800.)?)?;
    println!(
        "dimensions={:?}, layout_ms={:.3}",
        diagram.dimensions(),
        diagram.render_time().as_secs_f64() * 1000.
    );
    Ok(())
}
