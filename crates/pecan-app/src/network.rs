//! Application ownership of the independent bounded transport.
use gpui_kit::http_client::HttpClient;
use std::sync::Arc;
pub(crate) fn client() -> Arc<dyn HttpClient> {
    Arc::new(
        pecan_network::Client::new().expect("network executor and bundled TLS must be available"),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[gpui_kit::test]
    #[ignore = "requires the explicitly supplied network fixture URL"]
    fn supplied_network_picture_decodes_through_gpui(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{Asset as _, ImgResourceLoader, Resource};
        let url = std::env::var("PECAN_NETWORK_PICTURE").expect("provide the fixture URL");
        let load = cx.update(|cx| {
            cx.set_http_client(client());
            let load: std::pin::Pin<
                Box<
                    dyn Future<
                            Output = Result<Arc<gpui_kit::RenderImage>, gpui_kit::ImageCacheError>,
                        > + Send,
                >,
            > = Box::pin(ImgResourceLoader::load(Resource::Uri(url.into()), cx));
            load
        });
        // This ignored integration test uses real network I/O rather than the
        // deterministic scheduler used by the normal UI regression suite.
        let image = futures_lite::future::block_on(load).unwrap();
        let size = image.size(0);
        assert!(size.width.0 > 100 && size.height.0 > 100);
        println!(
            "GPUI decoded network image: {} x {}",
            size.width.0, size.height.0
        );
    }
}
