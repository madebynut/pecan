//! Bounded, deduplicated image collection for self-contained exports.
mod webp;

use anyhow::{Context as _, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_lite::AsyncReadExt as _;
use http_client::HttpClient;
use image::ImageDecoder as _;
use pecan_mermaid::{Diagram, Renderer};
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const MAX_IMAGE: usize = 16 * 1024 * 1024;
const MAX_RESOURCES: usize = 64 * 1024 * 1024;
// Keep full resolution while bounding buffers retained by PDF and temporary Word conversion.
const MAX_DECODED_IMAGE: u64 = 128 * 1024 * 1024;
const MAX_DECODED_RESOURCES: u64 = 256 * 1024 * 1024;
const MAX_SVG: usize = 4 * 1024 * 1024;
const MAX_SVG_DEPTH: usize = 32;

pub(super) struct Images {
    directory: PathBuf,
    client: Arc<dyn HttpClient>,
    bytes: usize,
    loaded: usize,
    entries: HashMap<String, String>,
    decoded: HashMap<String, (u32, u32)>,
    decoded_bytes: u64,
    svg_resources: HashSet<[u8; 32]>,
    decoded_resources: HashSet<[u8; 32]>,
    svg_rasters: HashMap<[u8; 32], usvg::ImageKind>,
    svg_parse_bytes: usize,
}
impl Images {
    pub(super) fn new(directory: &Path, client: Arc<dyn HttpClient>) -> Self {
        Self {
            directory: pecan_document::files::canonicalize(directory)
                .unwrap_or_else(|_| directory.to_path_buf()),
            client,
            bytes: 0,
            loaded: 0,
            entries: HashMap::new(),
            decoded: HashMap::new(),
            decoded_bytes: 0,
            svg_resources: HashSet::new(),
            decoded_resources: HashSet::new(),
            svg_rasters: HashMap::new(),
            svg_parse_bytes: 0,
        }
    }
    pub(super) async fn embed(&mut self, uri: &str) -> Result<String> {
        self.resolve(uri, None).await
    }
    pub(super) async fn copy(&mut self, uri: &str, assets: &Path) -> Result<String> {
        self.resolve(uri, Some(assets)).await
    }
    /// Raw bytes and file extension, under the same count and size budget as embedding.
    /// Callers deduplicate by URI.
    pub(super) async fn fetch(&mut self, uri: &str) -> Result<(Vec<u8>, &'static str)> {
        let (bytes, _, extension) = self.load_counted(uri).await?;
        Ok((bytes, extension))
    }
    /// Check metadata before raster decoding; each immutable URI spends the budget once.
    /// Embedding or copying original image bytes does not require this decode budget.
    pub(super) fn check_decode(&mut self, uri: &str, bytes: &[u8]) -> Result<(u32, u32)> {
        if let Some(&dimensions) = self.decoded.get(uri) {
            return Ok(dimensions);
        }
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .context("识别图片像素格式")?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(MAX_DECODED_IMAGE);
        reader.limits(limits);
        let decoder = reader
            .into_decoder()
            .context("读取图片尺寸与像素格式（单图解码预算 128 MiB）")?;
        let (width, height) = decoder.dimensions();
        if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            webp::check(bytes, (width, height))?;
        }
        // Reserve RGBA capacity at the original depth, including later alpha/channel conversion.
        let bytes_per_pixel = match decoder.color_type() {
            image::ColorType::L16
            | image::ColorType::La16
            | image::ColorType::Rgb16
            | image::ColorType::Rgba16 => 8,
            color => color.bytes_per_pixel().max(4),
        };
        let mut decoded = decoded_size(width, height, bytes_per_pixel)?;
        if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            // GIF's first frame can exceed its logical canvas; krilla decodes that frame directly.
            let mut gif = gif::DecodeOptions::new()
                .read_info(bytes)
                .context("读取 GIF 图片信息")?;
            let frame = gif
                .next_frame_info()
                .context("读取 GIF 首帧尺寸")?
                .context("GIF 图片没有帧")?;
            decoded = decoded.max(decoded_size(
                u32::from(frame.width),
                u32::from(frame.height),
                4,
            )?);
        }
        ensure!(
            decoded <= MAX_DECODED_IMAGE,
            "图片解码超过 128 MiB 单图预算"
        );
        let total = self
            .decoded_bytes
            .checked_add(decoded)
            .context("图片解码总量溢出")?;
        ensure!(
            total <= MAX_DECODED_RESOURCES,
            "图片解码总量超过 256 MiB 导出预算"
        );
        self.decoded_bytes = total;
        self.decoded.insert(uri.to_owned(), (width, height));
        self.decoded_resources
            .insert(*blake3::hash(bytes).as_bytes());
        Ok((width, height))
    }
    /// Parse SVG with bounded, deduplicated raster resources and explicit resource failures.
    pub(super) fn svg_image(
        &mut self,
        renderer: &Renderer,
        uri: &str,
        bytes: &[u8],
    ) -> Result<Diagram> {
        let svg = svg_text(bytes)?;
        self.count_svg_resource(bytes)?;
        self.count_svg_parse(bytes.len())?;
        let directory = if uri.starts_with("data:")
            || uri.starts_with("http://")
            || uri.starts_with("https://")
        {
            self.directory.clone()
        } else {
            image_path(&self.directory, uri)?
                .parent()
                .context("SVG 文件没有目录")?
                .to_path_buf()
        };
        let state = Mutex::new(SvgState {
            images: self,
            directories: vec![directory],
            files: HashMap::new(),
            file_data: HashMap::new(),
            error: None,
        });
        let resolver = usvg::ImageHrefResolver {
            resolve_data: Box::new(|mime, data, options| {
                svg_resource(&state, mime, data, options, None)
            }),
            resolve_string: Box::new(|href, options| {
                let (data, directory) = {
                    let mut state = state.lock().expect("SVG resource lock poisoned");
                    if state.error.is_some() {
                        return None;
                    }
                    match state.file(href) {
                        Ok(file) => file,
                        Err(error) => {
                            state.error = Some(error);
                            return None;
                        }
                    }
                };
                svg_resource(&state, "", data, options, Some(directory))
            }),
        };
        let diagram = renderer.svg_image_with_resolver(svg, resolver);
        if let Some(error) = state
            .into_inner()
            .expect("SVG resource lock poisoned")
            .error
        {
            return Err(error.context("导出 SVG 内部图片"));
        }
        diagram.context("解析 SVG 图片")
    }
    fn count_svg_resource(&mut self, bytes: &[u8]) -> Result<[u8; 32]> {
        let key = *blake3::hash(bytes).as_bytes();
        if self.svg_resources.contains(&key) {
            return Ok(key);
        }
        ensure!(bytes.len() <= MAX_IMAGE, "SVG 图片资源超过 16 MiB");
        ensure!(self.loaded < 2048, "文档图片超过 2048 张导出预算");
        let total = self
            .bytes
            .checked_add(bytes.len())
            .context("图片资源总量溢出")?;
        ensure!(total <= MAX_RESOURCES, "图片资源总量超过 64 MiB 导出预算");
        self.bytes = total;
        self.loaded += 1;
        self.svg_resources.insert(key);
        Ok(key)
    }
    fn count_svg_parse(&mut self, bytes: usize) -> Result<()> {
        // Nested SVG trees are owned per occurrence, even when their source bytes are identical.
        let total = self
            .svg_parse_bytes
            .checked_add(bytes)
            .context("SVG 解析总量溢出")?;
        ensure!(
            total <= MAX_RESOURCES,
            "SVG 解析总量超过 64 MiB 导出预算（重复嵌套按出现次数计入）"
        );
        self.svg_parse_bytes = total;
        Ok(())
    }
    async fn load_counted(&mut self, uri: &str) -> Result<(Vec<u8>, &'static str, &'static str)> {
        ensure!(self.loaded < 2048, "文档图片超过 2048 张导出预算");
        ensure!(
            uri.starts_with("data:") || uri.len() <= 8192,
            "图片 URL 超过 8192 字节"
        );
        let label = if uri.starts_with("data:") {
            "内嵌图片".into()
        } else {
            uri.chars().take(160).collect::<String>()
        };
        let bytes = self
            .load(uri)
            .await
            .with_context(|| format!("导出图片 {label}"))?;
        ensure!(
            !bytes.starts_with(&[0x1f, 0x8b]),
            "SVGZ 压缩图片暂不支持导出"
        );
        self.loaded += 1;
        self.bytes += bytes.len();
        ensure!(
            self.bytes <= MAX_RESOURCES,
            "图片资源总量超过 64 MiB 导出预算"
        );
        let (mime, extension) = image_type(&bytes).context("图片格式不支持导出")?;
        self.svg_resources.insert(*blake3::hash(&bytes).as_bytes());
        Ok((bytes, mime, extension))
    }
    async fn resolve(&mut self, uri: &str, assets: Option<&Path>) -> Result<String> {
        if let Some(existing) = self.entries.get(uri) {
            return Ok(existing.clone());
        }
        let (bytes, mime, extension) = self.load_counted(uri).await?;
        let result = if let Some(assets) = assets {
            let name = format!("image-{}.{}", self.entries.len() + 1, extension);
            std::fs::write(assets.join(&name), &bytes)?;
            let directory = assets
                .file_name()
                .context("图片目录没有名称")?
                .to_string_lossy();
            format!("{directory}/{name}")
        } else {
            format!("data:{mime};base64,{}", STANDARD.encode(bytes))
        };
        self.entries.insert(uri.into(), result.clone());
        Ok(result)
    }
    async fn load(&self, uri: &str) -> Result<Vec<u8>> {
        if uri.starts_with("data:") {
            let data =
                data_url::DataUrl::process(uri).map_err(|_| anyhow::anyhow!("无效 data URL"))?;
            let (bytes, _) = data
                .decode_to_vec()
                .map_err(|_| anyhow::anyhow!("无效图片数据"))?;
            ensure!(bytes.len() <= MAX_IMAGE, "图片超过 16 MiB");
            return Ok(bytes);
        }
        if uri.starts_with("https://") || uri.starts_with("http://") {
            let mut response = self.client.get(uri, ().into(), true).await?;
            ensure!(
                response.status().is_success(),
                "图片下载失败：HTTP {}",
                response.status()
            );
            let mut bytes = Vec::new();
            response
                .body_mut()
                .take(MAX_IMAGE as u64 + 1)
                .read_to_end(&mut bytes)
                .await?;
            ensure!(bytes.len() <= MAX_IMAGE, "图片超过 16 MiB");
            return Ok(bytes);
        }
        read_image(&image_path(&self.directory, uri)?)
    }
}

struct SvgState<'a> {
    images: &'a mut Images,
    directories: Vec<PathBuf>,
    files: HashMap<PathBuf, Arc<Vec<u8>>>,
    file_data: HashMap<[u8; 32], Arc<Vec<u8>>>,
    error: Option<anyhow::Error>,
}
impl SvgState<'_> {
    fn file(&mut self, href: &str) -> Result<(Arc<Vec<u8>>, PathBuf)> {
        ensure!(href.len() <= 8192, "SVG 图片路径超过 8192 字节");
        let directory = self.directories.last().context("SVG 文件没有目录")?;
        let path = image_path(directory, href).context("SVG 内部图片仅支持本地文件或 data URL")?;
        let data = match self.files.get(&path) {
            Some(data) => data.clone(),
            None => {
                ensure!(
                    self.files.len() < 2048,
                    "SVG 本地图片路径超过 2048 个导出预算"
                );
                let data = Arc::new(read_image(&path)?);
                let key = *blake3::hash(&data).as_bytes();
                let data = self.file_data.entry(key).or_insert(data).clone();
                self.files.insert(path.clone(), data.clone());
                data
            }
        };
        Ok((
            data,
            path.parent().context("SVG 图片没有目录")?.to_path_buf(),
        ))
    }
    fn admit(
        &mut self,
        mime: &str,
        data: &[u8],
        directory: Option<PathBuf>,
    ) -> Result<([u8; 32], &'static str, bool)> {
        ensure!(
            !data.starts_with(&[0x1f, 0x8b]),
            "SVGZ 压缩图片暂不支持导出"
        );
        let (mime, extension) = image_type(data)
            .or_else(|| (mime == "image/svg+xml").then_some(("image/svg+xml", "svg")))
            .context("SVG 内部图片格式不支持导出")?;
        ensure!(extension != "bmp", "SVG 内部 BMP 图片暂不支持导出");
        let key = self.images.count_svg_resource(data)?;
        let svg = extension == "svg";
        if svg {
            svg_text(data)?;
            self.images.count_svg_parse(data.len())?;
            ensure!(
                self.directories.len() < MAX_SVG_DEPTH,
                "SVG 图片嵌套超过 32 层"
            );
            let directory = directory
                .or_else(|| self.directories.last().cloned())
                .context("SVG 文件没有目录")?;
            self.directories.push(directory);
        } else if !self.images.decoded_resources.contains(&key) {
            self.images
                .check_decode(&format!("svg:{}", blake3::Hash::from(key).to_hex()), data)?;
        }
        Ok((key, mime, svg))
    }
}

fn svg_resource(
    state: &Mutex<SvgState<'_>>,
    mime: &str,
    data: Arc<Vec<u8>>,
    options: &usvg::Options<'_>,
    directory: Option<PathBuf>,
) -> Option<usvg::ImageKind> {
    let (key, mime, svg) = {
        let mut state = state.lock().expect("SVG resource lock poisoned");
        if state.error.is_some() {
            return None;
        }
        let admitted = match state.admit(mime, &data, directory) {
            Ok(admitted) => admitted,
            Err(error) => {
                state.error = Some(error);
                return None;
            }
        };
        if let Some(kind) = state.images.svg_rasters.get(&admitted.0) {
            return Some(kind.clone());
        }
        admitted
    };
    // Parsing nested SVG can re-enter either resolver; no resource lock is held here.
    let kind = if svg {
        // The default nested resolver silently removes local images. Keep our controlled resolver.
        std::str::from_utf8(&data)
            .map_err(anyhow::Error::from)
            .and_then(|svg| usvg::Tree::from_str(svg, options).map_err(anyhow::Error::from))
            .map(usvg::ImageKind::SVG)
    } else {
        usvg::ImageHrefResolver::default_data_resolver()(mime, data, options)
            .context("SVG 内部图片无法解析")
    };
    let mut state = state.lock().expect("SVG resource lock poisoned");
    if svg {
        state.directories.pop();
    }
    match kind {
        Ok(kind) => {
            if !svg {
                state.images.svg_rasters.insert(key, kind.clone());
            }
            Some(kind)
        }
        Err(error) => {
            if state.error.is_none() {
                state.error = Some(error);
            }
            None
        }
    }
}

fn svg_text(bytes: &[u8]) -> Result<&str> {
    ensure!(
        !bytes.starts_with(&[0x1f, 0x8b]),
        "SVGZ 压缩图片暂不支持导出"
    );
    ensure!(bytes.len() <= MAX_SVG, "SVG 图片超过 4 MiB");
    std::str::from_utf8(bytes).context("SVG 不是 UTF-8 文本")
}

fn image_path(directory: &Path, uri: &str) -> Result<PathBuf> {
    let path = resolve_image_path(directory, uri)?;
    // Opening a UNC path makes Windows authenticate to that host.
    ensure!(
        pecan_document::files::resource_allowed(directory, &path),
        "已阻止读取其他网络共享上的图片"
    );
    Ok(path)
}

fn resolve_image_path(directory: &Path, uri: &str) -> Result<PathBuf> {
    // A Windows drive letter is a path, not a URL scheme.
    #[cfg(windows)]
    if Path::new(uri).is_absolute() {
        return Ok(PathBuf::from(uri));
    }
    if let Ok(url) = url::Url::parse(uri) {
        ensure!(url.scheme() == "file", "不支持的图片 URL 协议");
        url.to_file_path()
            .map_err(|_| anyhow::anyhow!("无效文件 URL"))
    } else {
        // URL parsing decodes spaces/non-ASCII in relative image paths.
        let base = url::Url::from_directory_path(directory)
            .map_err(|_| anyhow::anyhow!("无效图片目录"))?;
        base.join(uri)?
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("无效相对图片路径"))
    }
}

fn read_image(path: &Path) -> Result<Vec<u8>> {
    // Check before opening: opening a FIFO or device blocks until another process writes.
    let metadata = std::fs::metadata(path).with_context(|| format!("读取 {}", path.display()))?;
    ensure!(metadata.is_file(), "图片不是普通文件");
    let mut file = std::fs::File::open(path).with_context(|| format!("读取 {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "图片不是普通文件");
    ensure!(metadata.len() <= MAX_IMAGE as u64, "图片超过 16 MiB");
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_IMAGE as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_IMAGE, "图片超过 16 MiB");
    if bytes.is_empty() {
        bail!("图片为空");
    }
    Ok(bytes)
}

fn decoded_size(width: u32, height: u32, bytes_per_pixel: u8) -> Result<u64> {
    ensure!(width > 0 && height > 0, "图片尺寸为空");
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(u64::from(bytes_per_pixel)))
        .context("图片解码尺寸溢出")
}

fn image_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(("image/jpeg", "jpg"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("image/gif", "gif"))
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some(("image/webp", "webp"))
    } else if bytes.starts_with(b"BM") {
        Some(("image/bmp", "bmp"))
    } else if std::str::from_utf8(bytes)
        .ok()
        .is_some_and(|s| s.contains("<svg"))
    {
        Some(("image/svg+xml", "svg"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pecan_mermaid::Region;

    #[cfg(windows)]
    #[test]
    fn windows_drive_unc_and_file_urls_resolve_to_local_images() {
        let directory = Path::new(r"C:\文档");
        for path in [r"C:\图片\hello world.png", "C:/图片/hello world.png"] {
            assert_eq!(image_path(directory, path).unwrap(), PathBuf::from(path));
        }
        // A share is readable from documents on that share, never from elsewhere.
        let shared = Path::new(r"\\server\share\文档");
        assert_eq!(
            image_path(shared, r"\\server\share\图.png").unwrap(),
            PathBuf::from(r"\\server\share\图.png")
        );
        for path in [
            r"\\server\share\图.png",
            "file://server/share/a.png",
            "//server/share/a.png",
        ] {
            assert!(image_path(directory, path).is_err(), "{path}");
        }
        assert_eq!(
            image_path(directory, "file:///C:/images/hello%20world.png").unwrap(),
            PathBuf::from(r"C:\images\hello world.png")
        );
        assert_eq!(
            image_path(directory, "images/hello%20world.png").unwrap(),
            directory.join(r"images\hello world.png")
        );
    }

    #[test]
    fn local_svg_resource_aliases_share_the_same_byte_allocation() {
        let directory = tempfile::tempdir().unwrap();
        let bytes = colored_png();
        std::fs::write(directory.path().join("first.png"), &bytes).unwrap();
        std::fs::write(directory.path().join("alias.png"), &bytes).unwrap();
        let mut images = Images::new(directory.path(), crate::network::client());
        let mut state = SvgState {
            images: &mut images,
            directories: vec![directory.path().to_path_buf()],
            files: HashMap::new(),
            file_data: HashMap::new(),
            error: None,
        };
        let (first, _) = state.file("first.png").unwrap();
        let (alias, _) = state.file("alias.png").unwrap();
        assert!(
            Arc::ptr_eq(&first, &alias),
            "local aliases must not retain duplicate image buffers"
        );
        assert_eq!(state.file_data.len(), 1);
    }

    #[test]
    fn repeated_nested_svg_parsing_has_a_cumulative_budget() {
        let directory = tempfile::tempdir().unwrap();
        let nested = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><!--{}--><rect width="16" height="16"/></svg>"#,
            "a".repeat(MAX_SVG - 1024)
        );
        std::fs::write(directory.path().join("nested.svg"), &nested).unwrap();
        let repeated = (0..18)
            .map(|_| r#"<image href="nested.svg" width="16" height="16"/>"#)
            .collect::<String>();
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16">{repeated}</svg>"#
        );
        let mut images = Images::new(directory.path(), crate::network::client());
        let error = images
            .svg_image(&Renderer::new(), "outer.svg", source.as_bytes())
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("SVG 解析总量超过 64 MiB"),
            "{error:#}"
        );
        assert_eq!(
            images.loaded, 2,
            "unique source resources stay deduplicated"
        );
        assert!(
            images.svg_parse_bytes <= MAX_RESOURCES,
            "oversized repeated trees must be rejected before parsing"
        );
    }

    fn svg_with_image(href: &str) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><image href="{href}" width="16" height="16"/></svg>"#
        )
    }
    fn colored_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([200, 30, 60, 255]),
        ))
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
        bytes
    }
    fn assert_svg_image_visible(diagram: &Diagram) {
        let png = diagram
            .png_region(Region::whole(diagram, 1.), None)
            .unwrap();
        let pixels = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(
            pixels.get_pixel(8, 8).0,
            [200, 30, 60, 255],
            "the embedded raster must render, not silently disappear"
        );
    }

    #[test]
    fn svg_data_images_preserve_pixels_and_spend_the_budget_once() {
        let directory = tempfile::tempdir().unwrap();
        let mut images = Images::new(directory.path(), crate::network::client());
        let href = format!("data:image/png;base64,{}", STANDARD.encode(colored_png()));
        let repeated = svg_with_image(&href).replace(
            "</svg>",
            &format!(
                "{}</svg>",
                (0..2050)
                    .map(|_| format!(r#"<image href="{href}" width="16" height="16"/>"#))
                    .collect::<String>()
            ),
        );
        let diagram = images
            .svg_image(
                &Renderer::new(),
                "data:image/svg+xml,fixture",
                repeated.as_bytes(),
            )
            .unwrap();
        assert_svg_image_visible(&diagram);
        assert_eq!(
            images.loaded, 2,
            "SVG and one unique PNG are the only resources"
        );
        assert_eq!(
            images.decoded_bytes, 16,
            "repeated references must share the decoded budget"
        );
        assert_eq!(images.svg_rasters.len(), 1);
    }

    #[test]
    fn nested_local_svg_images_resolve_from_each_svg_directory() {
        let directory = tempfile::tempdir().unwrap();
        let assets = directory.path().join("assets/child");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("图 片.png"), colored_png()).unwrap();
        std::fs::write(assets.join("inner.svg"), svg_with_image("图%20片.png")).unwrap();
        let source = svg_with_image("child/inner.svg");
        let mut images = Images::new(directory.path(), crate::network::client());
        let diagram = images
            .svg_image(&Renderer::new(), "assets/outer.svg", source.as_bytes())
            .unwrap();
        assert_svg_image_visible(&diagram);
    }

    #[test]
    fn svg_data_and_local_images_cannot_bypass_the_decode_budget() {
        let directory = tempfile::tempdir().unwrap();
        let huge = png_header(4097, 4096, png::BitDepth::Sixteen);
        std::fs::write(directory.path().join("large.png"), &huge).unwrap();
        for href in [
            "large.png".to_owned(),
            format!("data:image/png;base64,{}", STANDARD.encode(&huge)),
        ] {
            let mut images = Images::new(directory.path(), crate::network::client());
            let error = images
                .svg_image(
                    &Renderer::new(),
                    "outer.svg",
                    svg_with_image(&href).as_bytes(),
                )
                .unwrap_err();
            assert!(format!("{error:#}").contains("128 MiB"), "{error:#}");
            assert_eq!(images.decoded_bytes, 0);
        }
    }

    #[test]
    fn nested_svg_errors_are_reported_instead_of_missing_images() {
        let directory = tempfile::tempdir().unwrap();
        let inner = svg_with_image("missing.png");
        let href = format!(
            "data:image/svg+xml;base64,{}",
            STANDARD.encode(inner.as_bytes())
        );
        let mut images = Images::new(directory.path(), crate::network::client());
        let error = images
            .svg_image(
                &Renderer::new(),
                "outer.svg",
                svg_with_image(&href).as_bytes(),
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("missing.png"), "{error:#}");
    }

    #[test]
    fn svgz_and_recursive_local_svg_fail_explicitly() {
        let directory = tempfile::tempdir().unwrap();
        let mut images = Images::new(directory.path(), crate::network::client());
        let href = format!(
            "data:image/svg+xml;base64,{}",
            STANDARD.encode([0x1f, 0x8b, 0])
        );
        let error = images
            .svg_image(
                &Renderer::new(),
                "outer.svg",
                svg_with_image(&href).as_bytes(),
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("SVGZ"), "{error:#}");
        let source = svg_with_image("recursive.svg");
        std::fs::write(directory.path().join("recursive.svg"), &source).unwrap();
        let mut images = Images::new(directory.path(), crate::network::client());
        let error = images
            .svg_image(&Renderer::new(), "recursive.svg", source.as_bytes())
            .unwrap_err();
        assert!(format!("{error:#}").contains("32 层"), "{error:#}");
    }

    // Metadata only: these files describe large images without allocating pixel buffers.
    fn png_header(width: u32, height: u32, depth: png::BitDepth) -> Vec<u8> {
        png_header_color(width, height, depth, png::ColorType::Rgba)
    }

    fn png_header_color(
        width: u32,
        height: u32,
        depth: png::BitDepth,
        color: png::ColorType,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        let mut writer = encoder.write_header().unwrap();
        writer.write_chunk(png::chunk::IDAT, &[0]).unwrap();
        drop(writer);
        bytes
    }

    #[test]
    fn metadata_budget_counts_bit_depth_and_unique_images_before_decoding() {
        let directory = tempfile::tempdir().unwrap();
        let mut images = Images::new(directory.path(), crate::network::client());
        let large = png_header(4096, 4096, png::BitDepth::Sixteen);
        assert!(
            large.len() < 256,
            "the test must not allocate a large image"
        );
        assert_eq!(images.check_decode("first", &large).unwrap(), (4096, 4096));
        for _ in 0..2050 {
            images.check_decode("first", &large).unwrap();
        }
        images.check_decode("second", &large).unwrap();
        let tiny = png_header(1, 1, png::BitDepth::Eight);
        let error = images.check_decode("third", &tiny).unwrap_err();
        assert!(error.to_string().contains("256 MiB"), "{error:#}");
        assert_eq!(
            images.decoded.len(),
            2,
            "a rejection must not consume the budget"
        );
    }

    #[test]
    fn compressed_image_cannot_exceed_the_single_image_decode_budget() {
        let directory = tempfile::tempdir().unwrap();
        let mut images = Images::new(directory.path(), crate::network::client());
        let bytes = png_header(4097, 4096, png::BitDepth::Sixteen);
        let error = images.check_decode("oversized", &bytes).unwrap_err();
        assert!(error.to_string().contains("128 MiB"), "{error:#}");
        assert_eq!(images.decoded_bytes, 0, "rejection happens before decoding");
        assert!(decoded_size(u32::MAX, u32::MAX, 8).is_err());
    }

    #[test]
    fn sixteen_bit_images_reserve_rgba_capacity_at_the_original_depth() {
        let directory = tempfile::tempdir().unwrap();
        let mut images = Images::new(directory.path(), crate::network::client());
        for color in [
            png::ColorType::Grayscale,
            png::ColorType::GrayscaleAlpha,
            png::ColorType::Rgb,
            png::ColorType::Rgba,
        ] {
            let bytes = png_header_color(4097, 4096, png::BitDepth::Sixteen, color);
            let error = images
                .check_decode(&format!("{color:?}"), &bytes)
                .unwrap_err();
            assert!(
                error.to_string().contains("128 MiB"),
                "{color:?} must reserve eight bytes per pixel: {error:#}"
            );
        }
        assert_eq!(images.decoded_bytes, 0);
    }

    #[test]
    fn gif_budget_includes_a_first_frame_larger_than_its_canvas() {
        let mut bytes = Vec::new();
        {
            let mut encoder = gif::Encoder::new(&mut bytes, 1, 1, &[0, 0, 0]).unwrap();
            let frame = gif::Frame::from_indexed_pixels(1, 1, [0], None);
            encoder.write_frame(&frame).unwrap();
        }
        let descriptor = bytes.iter().position(|&byte| byte == b',').unwrap();
        bytes[descriptor + 5..descriptor + 7].copy_from_slice(&60_000_u16.to_le_bytes());
        bytes[descriptor + 7..descriptor + 9].copy_from_slice(&60_000_u16.to_le_bytes());
        let directory = tempfile::tempdir().unwrap();
        let mut images = Images::new(directory.path(), crate::network::client());
        let error = images.check_decode("oversized.gif", &bytes).unwrap_err();
        assert!(error.to_string().contains("128 MiB"), "{error:#}");
    }

    #[test]
    fn ordinary_raster_formats_keep_their_full_dimensions() {
        let directory = tempfile::tempdir().unwrap();
        let mut images = Images::new(directory.path(), crate::network::client());
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::Gif,
            image::ImageFormat::Bmp,
            image::ImageFormat::WebP,
        ] {
            let mut bytes = Vec::new();
            image::DynamicImage::new_rgb8(3, 2)
                .write_to(&mut Cursor::new(&mut bytes), format)
                .unwrap();
            assert_eq!(
                images.check_decode(&format!("{format:?}"), &bytes).unwrap(),
                (3, 2),
                "{format:?} must retain its native dimensions"
            );
        }
    }

    #[test]
    fn embedding_original_bytes_does_not_spend_the_decode_budget() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let mut images = Images::new(directory.path(), crate::network::client());
            let bytes = png_header(4097, 4096, png::BitDepth::Sixteen);
            std::fs::write(directory.path().join("large.png"), &bytes).unwrap();
            let embedded = images.embed("large.png").await.unwrap();
            assert_eq!(
                embedded,
                format!("data:image/png;base64,{}", STANDARD.encode(&bytes))
            );
            assert_eq!(images.decoded_bytes, 0);
        });
    }
}
