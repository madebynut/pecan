//! Per-export derived language mark resources; repeated labels share vector geometry and media.
use std::{collections::HashMap, sync::Arc};

use anyhow::{Context as _, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use pecan_mermaid::Renderer;

pub(super) const SIZE: f32 = 16.;

#[derive(Default)]
pub(super) struct Icons {
    symbols: HashMap<&'static str, Symbol>,
}

#[derive(Default)]
struct Symbol {
    html: Option<String>,
    tree: Option<Arc<usvg::Tree>>,
    png: Option<Vec<u8>>,
}

impl Icons {
    pub(super) fn html(&mut self, language: &str) -> String {
        let icon = pecan_assets::language::for_language(language);
        let symbol = self.symbols.entry(icon.path).or_default();
        symbol.html.get_or_insert_with(|| {
            let data = STANDARD.encode(icon.svg);
            format!("<img class=\"language-logo\" alt=\"\" width=\"16\" height=\"16\" src=\"data:image/svg+xml;base64,{data}\">")
        }).clone()
    }

    pub(super) fn tree(&mut self, language: &str) -> Result<Arc<usvg::Tree>> {
        let icon = pecan_assets::language::for_language(language);
        let symbol = self.symbols.entry(icon.path).or_default();
        if symbol.tree.is_none() {
            symbol.tree = Some(Arc::new(
                usvg::Tree::from_data(icon.svg, &usvg::Options::default())
                    .context("解析语言图标 SVG")?,
            ));
        }
        Ok(symbol.tree.as_ref().expect("tree was parsed").clone())
    }

    /// Word embeds a 2× raster and reuses its native media part.
    pub(super) fn png(&mut self, language: &str, renderer: &Renderer) -> Result<&[u8]> {
        let icon = pecan_assets::language::for_language(language);
        let symbol = self.symbols.entry(icon.path).or_default();
        if symbol.png.is_none() {
            let svg = std::str::from_utf8(icon.svg).context("语言图标 SVG 编码无效")?;
            symbol.png = Some(
                renderer
                    .svg_image(svg)
                    .context("解析语言图标")?
                    .png(SIZE * 2.)
                    .context("渲染语言图标")?,
            );
        }
        Ok(symbol.png.as_deref().expect("PNG was rendered"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_and_unknown_names_share_their_derived_resources() {
        let mut icons = Icons::default();
        let first_vector = icons.tree("json").unwrap();
        let second = icons.tree("JSONC").unwrap();
        assert!(
            Arc::ptr_eq(&first_vector, &second),
            "aliases share the parsed vector"
        );
        let renderer = Renderer::new();
        let first = icons.png("json", &renderer).unwrap().as_ptr();
        let second = icons.png("json-ld", &renderer).unwrap().as_ptr();
        assert_eq!(first, second, "aliases share the raster bytes");
        assert_eq!(icons.symbols.len(), 1);
        let fallback = icons.tree("text").unwrap();
        let unknown = icons.tree("unknown").unwrap();
        assert!(
            Arc::ptr_eq(&fallback, &unknown),
            "unknown names share a vector"
        );
        assert!(
            !Arc::ptr_eq(&first_vector, &fallback),
            "fallback differs from JSON"
        );
        let html = icons.html("text");
        assert!(html.contains("language-logo"));
        assert_eq!(html, icons.html("unknown"));
        let first = icons.png("text", &renderer).unwrap().as_ptr();
        let second = icons.png("", &renderer).unwrap().as_ptr();
        assert_eq!(first, second, "all missing marks share the PNG");
        assert_eq!(
            icons.symbols.len(),
            2,
            "only JSON and one fallback are cached"
        );
        let sql = icons.tree("SQL").unwrap();
        assert!(!Arc::ptr_eq(&sql, &fallback));
        let pg = icons.tree("pg").unwrap();
        assert!(Arc::ptr_eq(&pg, &icons.tree("PgSql").unwrap()));
        assert_ne!(icons.html("sql"), icons.html("unknown"));
        let pg_png = icons.png("pg", &renderer).unwrap().as_ptr();
        assert_eq!(pg_png, icons.png("postgresql", &renderer).unwrap().as_ptr());
        assert_eq!(
            icons.symbols.len(),
            4,
            "JSON, generic, SQL and PostgreSQL resources"
        );
    }
}
