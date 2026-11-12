//! Rebuild the monochrome application icon from its editable SVG source.
//!
//! `build_icons <svg> <iconset> [--macos]`. With `--macos`, the artwork is scaled onto
//! Apple's icon grid so the Dock shows it at the same size as other applications.
use std::path::PathBuf;

use resvg::{tiny_skia, usvg};

/// The source draws its rounded square 896 units wide on a 1024 canvas.
const SOURCE_BODY: f32 = 896.;
/// macOS 11+ icons draw an 824-unit body on the 1024 canvas, leaving 100 units each side.
const MACOS_BODY: f32 = 824.;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let source = PathBuf::from(args.next().ok_or("provide an SVG path")?);
    let output = PathBuf::from(args.next().ok_or("provide an iconset directory")?);
    let body = match args.next() {
        None => 1.,
        Some(flag) if flag == "--macos" => MACOS_BODY / SOURCE_BODY,
        Some(flag) => return Err(format!("unknown option {flag:?}").into()),
    };
    let tree = usvg::Tree::from_data(&std::fs::read(source)?, &usvg::Options::default())?;
    std::fs::create_dir_all(&output)?;
    for logical in [16, 32, 48, 128, 256, 512] {
        for scale in [1, 2] {
            let width = logical * scale;
            let mut pixels = tiny_skia::Pixmap::new(width, width).ok_or("allocate icon raster")?;
            // Scale about the canvas centre, keeping the transparent margin even on all sides.
            let fit = width as f32 / tree.size().width();
            let inset = width as f32 * (1. - body) / 2.;
            resvg::render(
                &tree,
                tiny_skia::Transform::from_row(fit * body, 0., 0., fit * body, inset, inset),
                &mut pixels.as_mut(),
            );
            let suffix = if scale == 2 { "@2x" } else { "" };
            pixels.save_png(output.join(format!("icon_{logical}x{logical}{suffix}.png")))?;
            if scale == 1 && logical <= 48 {
                let rgba: Vec<u8> = pixels
                    .pixels()
                    .iter()
                    .flat_map(|pixel| {
                        let pixel = pixel.demultiply();
                        [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
                    })
                    .collect();
                std::fs::write(output.join(format!("icon_{logical}x{logical}.rgba")), rgba)?;
            }
        }
    }
    Ok(())
}
