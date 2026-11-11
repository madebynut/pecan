//! Shared limits for resource images, clipboard images and generated raster bytes.
use anyhow::{Context as _, Result, ensure};
use image::{
    AnimationDecoder as _, DynamicImage, Frame, ImageDecoder, ImageFormat, Rgba,
    codecs::{gif::GifDecoder, webp::WebPDecoder},
};
use smallvec::SmallVec;
use std::{
    io::{Cursor, Read as _},
    path::Path,
};

pub(crate) const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_DECODED_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FRAMES: usize = 4096;

pub(crate) fn check_source(bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() <= MAX_IMAGE_BYTES,
        "image source exceeds 16 MiB"
    );
    Ok(())
}

pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    ensure!(
        file.metadata()?.len() <= MAX_IMAGE_BYTES as u64,
        "image source exceeds 16 MiB"
    );
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    check_source(&bytes)?;
    Ok(bytes)
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    limits
}

fn check_decoder(decoder: &mut impl ImageDecoder) -> Result<u64> {
    decoder.set_limits(limits())?;
    let (width, height) = decoder.dimensions();
    let rgba_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .context("image dimensions overflow")?;
    ensure!(
        rgba_bytes <= MAX_DECODED_BYTES && decoder.total_bytes() <= MAX_DECODED_BYTES,
        "decoded image exceeds 128 MiB"
    );
    Ok(rgba_bytes)
}

pub(crate) fn decode(bytes: &[u8], format: ImageFormat) -> Result<SmallVec<[Frame; 1]>> {
    check_source(bytes)?;
    match format {
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(Cursor::new(bytes))?;
            let frame_bytes = check_decoder(&mut decoder)?;
            animation(decoder.into_frames(), frame_bytes)
        }
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes))?;
            let frame_bytes = check_decoder(&mut decoder)?;
            if decoder.has_animation() {
                let _ = decoder.set_background_color(Rgba([0, 0, 0, 0]));
                animation(decoder.into_frames(), frame_bytes)
            } else {
                decode_static(decoder)
            }
        }
        _ => {
            let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
            reader.limits(limits());
            decode_static(reader.into_decoder().context("creating image decoder")?)
        }
    }
}

fn decode_static(mut decoder: impl ImageDecoder) -> Result<SmallVec<[Frame; 1]>> {
    check_decoder(&mut decoder)?;
    let orientation = decoder.orientation().context("reading image orientation")?;
    let mut image = DynamicImage::from_decoder(decoder).context("decoding image")?;
    image.apply_orientation(orientation);
    let mut frame = Frame::new(image.into_rgba8());
    bgra(&mut frame);
    Ok(SmallVec::from_elem(frame, 1))
}

fn animation(frames: image::Frames<'_>, frame_bytes: u64) -> Result<SmallVec<[Frame; 1]>> {
    let mut output = SmallVec::new();
    let mut decoded = 0u64;
    for (index, frame) in frames.enumerate() {
        // The decoder holds at most one additional frame when the total is exceeded.
        // Never silently truncate an animation or suppress a decoder's limit error.
        ensure!(index < MAX_FRAMES, "animation exceeds 4096 frames");
        let mut frame = match frame {
            Ok(frame) => frame,
            Err(error @ image::ImageError::Limits(_)) => return Err(error.into()),
            Err(error) => {
                log::debug!("Skipping damaged animation frame: {error}");
                continue;
            }
        };
        decoded += frame_bytes.max(frame.buffer().len() as u64);
        ensure!(
            decoded <= MAX_DECODED_BYTES,
            "decoded animation exceeds 128 MiB"
        );
        bgra(&mut frame);
        output.push(frame);
    }
    ensure!(!output.is_empty(), "animation contains no frames");
    Ok(output)
}

fn bgra(frame: &mut Frame) {
    for pixel in frame.buffer_mut().chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

// Embedded SVG rasters are decoded by resvg later. Validate their advertised
// allocation without decoding a second copy on the loading thread.
pub(crate) fn raster_size(bytes: &[u8]) -> Result<u64> {
    check_source(bytes)?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(limits());
    check_decoder(&mut reader.into_decoder()?)
}
