//! Validate container and bitstream dimensions before a WebP decoder allocates pixels.
//! Extended WebP decoders report the canvas before checking the inner VP8 frame size.

use anyhow::{Context as _, Result, ensure};

pub(super) fn check(bytes: &[u8], canvas: (u32, u32)) -> Result<()> {
    ensure!(
        bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        "无效 WebP RIFF 头"
    );
    ensure!(canvas.0 > 0 && canvas.1 > 0, "WebP 画布尺寸为空");
    let riff_size = usize::try_from(le_u32(bytes.get(4..8).context("WebP RIFF 头被截断")?))?;
    let end = riff_size.checked_add(8).context("WebP RIFF 长度溢出")?;
    let chunks = bytes.get(12..end).context("WebP RIFF 数据被截断")?;
    ensure!(check_chunks(chunks, canvas, true)? > 0, "WebP 没有图像帧");
    Ok(())
}

fn check_chunks(mut bytes: &[u8], expected: (u32, u32), allow_frames: bool) -> Result<usize> {
    let mut images = 0_usize;
    while !bytes.is_empty() {
        let header = bytes.get(..8).context("WebP chunk 头被截断")?;
        let size = usize::try_from(le_u32(&header[4..8]))?;
        let end = size.checked_add(8).context("WebP chunk 长度溢出")?;
        let data = bytes.get(8..end).context("WebP chunk 数据被截断")?;
        match &header[..4] {
            b"VP8 " => {
                ensure!(
                    vp8_dimensions(data)? == expected,
                    "WebP VP8 尺寸与画布或动画帧不一致"
                );
                images = images.checked_add(1).context("WebP 帧数量溢出")?;
            }
            b"VP8L" => {
                ensure!(
                    vp8l_dimensions(data)? == expected,
                    "WebP VP8L 尺寸与画布或动画帧不一致"
                );
                images = images.checked_add(1).context("WebP 帧数量溢出")?;
            }
            b"VP8X" => {
                ensure!(allow_frames, "WebP 动画帧包含无效 VP8X chunk");
                let header = data.get(..10).context("WebP VP8X 头被截断")?;
                let canvas = (le_u24(&header[4..7]) + 1, le_u24(&header[7..10]) + 1);
                ensure!(canvas == expected, "WebP VP8X 画布尺寸不一致");
            }
            b"ANMF" => {
                ensure!(allow_frames, "WebP 动画帧不能嵌套");
                let frame = data.get(..16).context("WebP ANMF 头被截断")?;
                let x = le_u24(&frame[..3])
                    .checked_mul(2)
                    .context("WebP 动画偏移溢出")?;
                let y = le_u24(&frame[3..6])
                    .checked_mul(2)
                    .context("WebP 动画偏移溢出")?;
                let width = le_u24(&frame[6..9]) + 1;
                let height = le_u24(&frame[9..12]) + 1;
                ensure!(
                    x.checked_add(width).context("WebP 动画宽度溢出")? <= expected.0
                        && y.checked_add(height).context("WebP 动画高度溢出")? <= expected.1,
                    "WebP 动画帧超出画布"
                );
                ensure!(
                    check_chunks(&data[16..], (width, height), false)? > 0,
                    "WebP 动画帧缺少图像数据"
                );
                images = images.checked_add(1).context("WebP 帧数量溢出")?;
            }
            _ => {}
        }
        let padded = end
            .checked_add(size & 1)
            .context("WebP chunk 填充长度溢出")?;
        bytes = bytes.get(padded..).context("WebP chunk 填充被截断")?;
    }
    Ok(images)
}

fn vp8_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    let header = bytes.get(..10).context("WebP VP8 头被截断")?;
    ensure!(
        header[0] & 1 == 0 && header[3..6] == [0x9d, 0x01, 0x2a],
        "WebP VP8 不是有效关键帧"
    );
    let width = u16::from_le_bytes([header[6], header[7]]) & 0x3fff;
    let height = u16::from_le_bytes([header[8], header[9]]) & 0x3fff;
    ensure!(width > 0 && height > 0, "WebP VP8 尺寸为空");
    Ok((u32::from(width), u32::from(height)))
}

fn vp8l_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    let header = bytes.get(..5).context("WebP VP8L 头被截断")?;
    ensure!(header[0] == 0x2f, "WebP VP8L 签名无效");
    let dimensions = le_u32(&header[1..5]);
    ensure!(dimensions >> 29 == 0, "WebP VP8L 版本不支持");
    Ok(((dimensions & 0x3fff) + 1, ((dimensions >> 14) & 0x3fff) + 1))
}

// Callers obtain fixed-size, checked slices before reading these little-endian fields.
fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn le_u24(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageDecoder as _;
    use std::io::Cursor;

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = kind.to_vec();
        bytes.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(data);
        if !data.len().is_multiple_of(2) {
            bytes.push(0);
        }
        bytes
    }

    fn riff(chunks: &[u8]) -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&u32::try_from(chunks.len() + 4).unwrap().to_le_bytes());
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(chunks);
        bytes
    }

    fn lossless((width, height): (u32, u32)) -> Vec<u8> {
        let mut data = vec![0x2f];
        data.extend_from_slice(&((width - 1) | ((height - 1) << 14)).to_le_bytes());
        chunk(b"VP8L", &data)
    }

    fn extended((width, height): (u32, u32), flags: u8) -> Vec<u8> {
        let mut data = vec![flags, 0, 0, 0];
        data.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        data.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        chunk(b"VP8X", &data)
    }

    fn lossy((width, height): (u16, u16)) -> Vec<u8> {
        let mut data = vec![0, 0, 0, 0x9d, 0x01, 0x2a];
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        chunk(b"VP8 ", &data)
    }

    #[test]
    fn ordinary_lossless_images_and_lossy_headers_keep_their_dimensions() {
        for size in [(1, 1), (3, 2), (24, 16)] {
            let mut bytes = Vec::new();
            image::DynamicImage::new_rgba8(size.0, size.1)
                .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::WebP)
                .unwrap();
            check(&bytes, size).unwrap();
        }
        check(&riff(&lossy((3, 2))), (3, 2)).unwrap();
    }

    #[test]
    fn tiny_extended_canvas_cannot_hide_a_large_vp8_bitstream() {
        let mut chunks = extended((1, 1), 0);
        chunks.extend(lossy((8192, 8192)));
        let bytes = riff(&chunks);
        // The production decoder exposes only the tiny outer canvas at this stage.
        let decoder = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();
        assert_eq!(decoder.dimensions(), (1, 1));
        let error = check(&bytes, decoder.dimensions()).unwrap_err();
        assert!(error.to_string().contains("VP8 尺寸"), "{error:#}");

        let mut chunks = extended((1, 1), 0);
        chunks.extend(lossless((8192, 8192)));
        assert!(check(&riff(&chunks), (1, 1)).is_err());
    }

    #[test]
    fn animated_frames_check_their_rectangles_and_bitstream_dimensions() {
        let mut chunks = extended((8, 6), 2);
        chunks.extend(chunk(b"ANIM", &[0; 6]));
        let mut frame = vec![0; 16];
        frame[0] = 1; // x = 2
        frame[3] = 1; // y = 2
        frame[6] = 3; // width = 4
        frame[9] = 2; // height = 3
        frame.extend(lossless((4, 3)));
        chunks.extend(chunk(b"ANMF", &frame));
        check(&riff(&chunks), (8, 6)).unwrap();

        let mut outside = frame.clone();
        outside[0] = 3; // x = 6; x + width > canvas width
        let mut chunks = extended((8, 6), 2);
        chunks.extend(chunk(b"ANMF", &outside));
        assert!(
            check(&riff(&chunks), (8, 6))
                .unwrap_err()
                .to_string()
                .contains("超出画布")
        );

        frame.truncate(16);
        frame.extend(lossy((8192, 8192)));
        let mut chunks = extended((8, 6), 2);
        chunks.extend(chunk(b"ANMF", &frame));
        assert!(
            check(&riff(&chunks), (8, 6))
                .unwrap_err()
                .to_string()
                .contains("VP8 尺寸")
        );
    }

    #[test]
    fn truncated_and_overflowing_lengths_are_rejected_without_panicking() {
        let bytes = riff(&lossless((3, 2)));
        for end in 0..bytes.len() {
            assert!(check(&bytes[..end], (3, 2)).is_err());
        }
        let mut too_long = bytes.clone();
        too_long[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(check(&too_long, (3, 2)).is_err());
        let mut too_long = bytes;
        too_long[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(check(&too_long, (3, 2)).is_err());
    }
}
