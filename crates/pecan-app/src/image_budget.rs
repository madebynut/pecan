//! Regression coverage for the image decoder consumed by the actual GPUI loader.
use gpui_kit::{App, Image, ImageFormat, TestAppContext};
use std::sync::Arc;

fn gif(frame_count: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    let frame = image::Frame::new(image::RgbaImage::from_pixel(
        1,
        1,
        image::Rgba([17, 29, 43, 255]),
    ));
    image::codecs::gif::GifEncoder::new(&mut bytes)
        .encode_frames((0..frame_count).map(|_| frame.clone()))
        .unwrap();
    bytes
}

#[gpui_kit::test]
fn animation_and_encoded_image_budgets_fail_without_truncation(cx: &mut TestAppContext) {
    let renderer = cx.update(|cx: &mut App| cx.svg_renderer());
    let bytes = gif(4097);
    assert!(
        Image::from_bytes(ImageFormat::Gif, bytes)
            .to_image_data(renderer.clone())
            .is_err(),
        "an excessive frame count must fail rather than allocate every frame"
    );
    let mut bytes = gif(1);
    bytes.resize(16 * 1024 * 1024 + 1, 0);
    assert!(
        Image::from_bytes(ImageFormat::Gif, bytes)
            .to_image_data(renderer)
            .is_err(),
        "encoded bytes share the same limit for file and in-memory sources"
    );
}

#[gpui_kit::test]
fn ordinary_animation_keeps_all_frames_and_colors(cx: &mut TestAppContext) {
    let renderer = cx.update(|cx: &mut App| cx.svg_renderer());
    let image = Image::from_bytes(ImageFormat::Gif, gif(2))
        .to_image_data(renderer)
        .unwrap();
    assert_eq!(image.frame_count(), 2);
    for frame in 0..2 {
        assert_eq!(image.as_bytes(frame).unwrap(), &[43, 29, 17, 255]);
    }
}

#[gpui_kit::test]
async fn resource_loader_enforces_the_same_budget(cx: &mut TestAppContext) {
    use gpui_kit::{Asset, ImageAssetLoader, Resource};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("long.gif");
    std::fs::write(&path, gif(4097)).unwrap();
    let pending = cx.update(|cx| {
        let load: std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<Arc<gpui_kit::RenderImage>, gpui_kit::ImageCacheError>,
                    > + Send,
            >,
        > = Box::pin(ImageAssetLoader::load(Resource::Path(Arc::from(path)), cx));
        load
    });
    assert!(
        pending.await.is_err(),
        "ordinary document images must use the bounded decoder too"
    );
}

#[gpui_kit::test]
fn declared_pixels_are_rejected_before_allocation(cx: &mut TestAppContext) {
    let renderer = cx.update(|cx| cx.svg_renderer());
    let mut bytes = gif(1);
    // GIF canvas dimensions can be huge even when the only encoded frame is 1x1.
    bytes[6..10].copy_from_slice(&[255, 255, 255, 255]);
    let error = Image::from_bytes(ImageFormat::Gif, bytes)
        .to_image_data(renderer)
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("128 MiB"), "{error:#}");
}

#[gpui_kit::test]
fn jpeg_orientation_and_svg_embedded_images_survive(cx: &mut TestAppContext) {
    let renderer = cx.update(|cx| cx.svg_renderer());
    let jpeg = Image::from_bytes(
        ImageFormat::Jpeg,
        include_bytes!("../../../vendor/gpui-pre/examples/image/exif-orientation-rotate-180.jpg")
            .to_vec(),
    );
    let pixels = jpeg.to_image_data(renderer.clone()).unwrap();
    assert_eq!(pixels.size(0), gpui_kit::size(16.into(), 32.into()));
    assert_eq!(&pixels.as_bytes(0).unwrap()[..4], &[255, 255, 255, 255]);
    assert_eq!(
        &pixels.as_bytes(0).unwrap()[(16 * 32 - 1) * 4..],
        &[0, 0, 0, 255]
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("picture.gif");
    std::fs::write(&path, gif(1)).unwrap();
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="{}" width="10" height="10"/></svg>"#,
        path.display()
    );
    let pixels = Image::from_bytes(ImageFormat::Svg, svg.as_bytes().to_vec())
        .to_image_data(renderer.clone())
        .unwrap();
    assert!(
        pixels
            .as_bytes(0)
            .unwrap()
            .as_chunks::<4>()
            .0
            .contains(&[43, 29, 17, 255])
    );
    let mut oversized = gif(1);
    oversized[6..10].copy_from_slice(&[255, 255, 255, 255]);
    std::fs::write(&path, oversized).unwrap();
    assert!(
        Image::from_bytes(ImageFormat::Svg, svg.into_bytes())
            .to_image_data(renderer)
            .is_err()
    );
}

#[gpui_kit::test]
fn compressed_svg_has_a_decompressed_budget(cx: &mut TestAppContext) {
    let renderer = cx.update(|cx| cx.svg_renderer());
    let error = Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../tests/fixtures/oversized.svgz").to_vec(),
    )
    .to_image_data(renderer)
    .err()
    .unwrap();
    assert!(
        error.to_string().contains("maximum number of SVG elements"),
        "{error:#}"
    );
}
