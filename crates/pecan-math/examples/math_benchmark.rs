use pecan_math::{MathFont, MathStyle, Presentation, Renderer};
use std::{hint::black_box, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let renderer = Renderer::new();
    let presentation = Presentation {
        style: MathStyle::Display,
        ..Default::default()
    };
    let source = r"\frac{-b\pm\sqrt{b^2-4ac}}{2a}";
    let start = Instant::now();
    let formula = renderer.render(black_box(source), presentation)?;
    println!(
        "math first incl font init + PNG: {:.3} ms; raster: {} bytes",
        start.elapsed().as_secs_f64() * 1000.,
        formula.raster_bytes()
    );
    let start = Instant::now();
    for _ in 0..10_000 {
        black_box(renderer.render(black_box(source), presentation)?);
    }
    println!(
        "math cached lookup: {:.3} us",
        start.elapsed().as_secs_f64() * 100.
    );
    for (source, font) in [
        (r"\ce{N2 + 3H2 <=> 2NH3}", MathFont::StixTwo),
        (r"\chemfig{H_3C-*6(=-=-=-)}", MathFont::StixTwo),
        (r"\sum_{n=1}^{\infty}\frac{1}{n^2}", MathFont::LatinModern),
    ] {
        let start = Instant::now();
        black_box(renderer.render(
            black_box(source),
            Presentation {
                font,
                ..presentation
            },
        )?);
        println!(
            "{source} ({font:?}): {:.3} ms",
            start.elapsed().as_secs_f64() * 1000.
        );
    }
    Ok(())
}
