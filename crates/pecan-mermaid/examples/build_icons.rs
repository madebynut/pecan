//! Rebuild the monochrome application icon from its editable SVG source.
use std::path::PathBuf;

use resvg::{tiny_skia, usvg};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let source = PathBuf::from(args.next().ok_or("provide an SVG path")?);
    let output = PathBuf::from(args.next().ok_or("provide an iconset directory")?);
    let tree = usvg::Tree::from_data(&std::fs::read(source)?, &usvg::Options::default())?;
    std::fs::create_dir_all(&output)?;
    for logical in [16, 32, 48, 128, 256, 512] {
        for scale in [1, 2] {
            let width = logical * scale;
            let mut pixels = tiny_skia::Pixmap::new(width, width).ok_or("allocate icon raster")?;
            resvg::render(
                &tree,
                tiny_skia::Transform::from_scale(
                    width as f32 / tree.size().width(),
                    width as f32 / tree.size().height(),
                ),
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
