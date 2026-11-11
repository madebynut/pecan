//! Small, local Lucide symbols shared by all document exporters.
use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

use anyhow::{Context as _, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use pecan_document::AlertKind;
use pecan_mermaid::Renderer;

/// Compact logical size for the exported document typography.
pub(super) const SIZE: f32 = 16.;

#[derive(Default)]
pub(super) struct Icons {
    symbols: HashMap<&'static str, Symbol>,
}

struct Symbol {
    svg: String,
    html: Option<String>,
    tree: Option<Arc<usvg::Tree>>,
    png: Option<Vec<u8>>,
}

impl Icons {
    fn symbol(&mut self, kind: Option<AlertKind>) -> Result<&mut Symbol> {
        let key = kind.map_or("generic", AlertKind::class);
        if let Entry::Vacant(entry) = self.symbols.entry(key) {
            let bytes = pecan_assets::notes::svg(kind);
            let [red, green, blue] = pecan_assets::notes::export_mark(kind);
            let color = format!("#{red:02x}{green:02x}{blue:02x}");
            let svg = std::str::from_utf8(bytes)
                .context("提示图标 SVG 编码无效")?
                .replace("currentColor", &color);
            entry.insert(Symbol {
                svg,
                html: None,
                tree: None,
                png: None,
            });
        }
        Ok(self.symbols.get_mut(key).expect("symbol was inserted"))
    }

    pub(super) fn html(&mut self, kind: Option<AlertKind>) -> Result<String> {
        let symbol = self.symbol(kind)?;
        Ok(symbol.html.get_or_insert_with(|| {
            let data = STANDARD.encode(&symbol.svg);
            format!("<img class=\"note-icon\" alt=\"\" width=\"16\" height=\"16\" src=\"data:image/svg+xml;base64,{data}\">")
        }).clone())
    }

    /// Icons contain paths only, so parsing does not need a system font database.
    pub(super) fn tree(&mut self, kind: Option<AlertKind>) -> Result<Arc<usvg::Tree>> {
        let symbol = self.symbol(kind)?;
        if symbol.tree.is_none() {
            symbol.tree = Some(Arc::new(
                usvg::Tree::from_str(&symbol.svg, &usvg::Options::default())
                    .context("解析提示图标 SVG")?,
            ));
        }
        Ok(symbol.tree.as_ref().expect("tree was parsed").clone())
    }

    /// Word needs a tiny raster; repeated notes share this 2× PNG and its media part.
    pub(super) fn png(&mut self, kind: Option<AlertKind>, renderer: &Renderer) -> Result<&[u8]> {
        let symbol = self.symbol(kind)?;
        if symbol.png.is_none() {
            let diagram = renderer
                .svg_image(&symbol.svg)
                .context("解析提示图标 SVG")?;
            symbol.png = Some(diagram.png(SIZE * 2.).context("渲染提示图标")?);
        }
        Ok(symbol.png.as_deref().expect("PNG was rendered"))
    }
}
