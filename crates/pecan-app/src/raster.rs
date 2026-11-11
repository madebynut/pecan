//! Tie generated image decoding and GPU resources to visible element state.
use gpui_kit::{App, Image, ImageCacheError, RenderImage, Window};
use std::sync::Arc;

struct Raster {
    image: Arc<Image>,
    pixels: Option<Result<Arc<RenderImage>, ImageCacheError>>,
}

pub(crate) fn visible(
    image: &Arc<Image>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<RenderImage>> {
    visible_result(image, window, cx).and_then(Result::ok)
}

pub(crate) fn visible_result(
    image: &Arc<Image>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
    let raster = window.use_keyed_state(("generated-raster", image.id()), cx, |_, cx| {
        cx.on_release(|raster: &mut Raster, cx| {
            raster.image.clone().remove_asset(cx);
            if let Some(Ok(pixels)) = raster.pixels.take() {
                cx.drop_image(pixels, None);
            }
        })
        .detach();
        Raster {
            image: image.clone(),
            pixels: None,
        }
    });
    raster.update(cx, |raster, cx| {
        if raster.pixels.is_none() {
            raster.pixels = raster.image.clone().use_render_image_result(window, cx);
        }
        raster.pixels.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        AppContext, Context, Entity, IntoElement, Render, TestAppContext, VisualTestContext,
        WindowOptions, div, img, prelude::*, px,
    };

    struct Pictures {
        image: Option<Arc<Image>>,
        legacy: bool,
    }
    impl Render for Pictures {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(400.))
                .h(px(300.))
                .when_some(self.image.as_ref(), |this, image| {
                    if self.legacy {
                        this.child(img(image.clone()).size_full())
                    } else {
                        this.when_some(visible(image, window, cx), |this, pixels| {
                            this.child(img(pixels).size_full())
                        })
                    }
                })
        }
    }

    fn picture(index: u8, width: u32, height: u32) -> Arc<Image> {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let pixels = [index, 100, 160, 255].repeat((width * height) as usize);
            writer.write_image_data(&pixels).unwrap();
        }
        Arc::new(Image::from_bytes(gpui_kit::ImageFormat::Png, png))
    }

    fn draw(visual: &mut VisualTestContext) {
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
        }
    }

    fn cycle(cx: &mut TestAppContext, count: u8, dimensions: (u32, u32), legacy: bool) {
        let (handle, view): (_, Entity<Pictures>) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Pictures {
                    image: None,
                    legacy,
                })
            })
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        let mut images = Vec::new();
        for index in 0..count {
            let image = picture(index, dimensions.0, dimensions.1);
            visual.update(|_, cx| {
                view.update(cx, |view, cx| {
                    view.image = Some(image.clone());
                    cx.notify();
                })
            });
            draw(&mut visual);
            let pixels =
                visual.update(|window, cx| image.clone().get_render_image(window, cx).unwrap());
            visual.update(|_, cx| {
                view.update(cx, |view, cx| {
                    view.image = None;
                    cx.notify();
                })
            });
            draw(&mut visual);
            visual.update(|window, cx| {
                if !legacy {
                    assert!(
                        !image.is_asset_cached(cx),
                        "leaving image {index} removes decoded data"
                    );
                    assert!(
                        !window.has_image_atlas_entry(&pixels),
                        "leaving image {index} removes its GPU atlas entry"
                    );
                }
            });
            images.push(image);
        }
        let cached = visual.update(|_, cx| {
            images
                .iter()
                .filter(|image| image.is_asset_cached(cx))
                .count()
        });
        assert_eq!(cached, if legacy { usize::from(count) } else { 0 });
        if let Some(probe) = std::env::var_os("PECAN_MEMORY_PROBE") {
            let output = std::process::Command::new(probe)
                .arg(std::process::id().to_string())
                .output()
                .unwrap();
            assert!(output.status.success());
            println!(
                "legacy={legacy} images={count} dimensions={dimensions:?} cached={cached}: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            );
        }
    }

    #[gpui_kit::test]
    fn leaving_generated_images_releases_decoded_pixels_and_atlas(cx: &mut TestAppContext) {
        cycle(cx, 3, (32, 32), false);
    }

    #[gpui_kit::test]
    #[ignore = "explicit resident-image benchmark; optional PECAN_MEMORY_PROBE and PECAN_BENCHMARK_LEGACY"]
    fn generated_image_memory_benchmark(cx: &mut TestAppContext) {
        cycle(
            cx,
            24,
            (1600, 800),
            std::env::var_os("PECAN_BENCHMARK_LEGACY").is_some(),
        );
    }
}
