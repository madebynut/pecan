//! Share system font discovery across renderer handles without changing selection.
use resvg::usvg;
use std::sync::{Arc, OnceLock};

/// Portable preference name for the platform UI font and native CJK fallback.
pub const SYSTEM_UI_FONT: &str = ".SystemUIFont";
/// Portable preference name for the installed platform monospace font.
pub const SYSTEM_MONO_FONT: &str = ".SystemMonospace";

fn installed_family(candidates: &[&str], monospace: bool) -> String {
    let db = database();
    for name in candidates {
        if let Some(face) = db.faces().find(|face| {
            (!monospace || face.monospaced)
                && face.families.iter().any(|(family, _)| family == name)
        }) {
            return face.families[0].0.clone();
        }
    }
    db.faces()
        .find(|face| !monospace || face.monospaced)
        .map_or_else(|| "sans-serif".into(), |face| face.families[0].0.clone())
}

fn system_mono() -> &'static str {
    static FAMILY: OnceLock<String> = OnceLock::new();
    FAMILY.get_or_init(|| {
        installed_family(
            if cfg!(target_os = "macos") {
                &["Menlo", "Monaco", "Courier New"]
            } else if cfg!(target_os = "windows") {
                &["Consolas", "Cascadia Mono", "Courier New"]
            } else {
                &["DejaVu Sans Mono", "Noto Sans Mono", "Liberation Mono"]
            },
            true,
        )
    })
}

fn system_ui(chinese: bool) -> &'static str {
    static LATIN: OnceLock<String> = OnceLock::new();
    static CJK: OnceLock<String> = OnceLock::new();
    let (cache, names): (&OnceLock<String>, &[&str]) = if chinese {
        (
            &CJK,
            if cfg!(target_os = "macos") {
                &["PingFang SC", "Hiragino Sans GB"]
            } else if cfg!(target_os = "windows") {
                &["Microsoft YaHei UI", "Microsoft YaHei", "Segoe UI"]
            } else {
                &["Noto Sans CJK SC", "Noto Sans SC", "DejaVu Sans"]
            },
        )
    } else {
        (
            &LATIN,
            if cfg!(target_os = "macos") {
                &[".AppleSystemUIFont", "System Font", "Helvetica Neue"]
            } else if cfg!(target_os = "windows") {
                &["Segoe UI Variable", "Segoe UI"]
            } else {
                &["Noto Sans", "DejaVu Sans"]
            },
        )
    };
    cache.get_or_init(|| installed_family(names, false))
}

/// Independent system font families for Latin and CJK diagram text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramFonts {
    english: String,
    chinese: String,
}

impl Default for DiagramFonts {
    fn default() -> Self {
        Self::new(SYSTEM_MONO_FONT, SYSTEM_UI_FONT)
    }
}

impl DiagramFonts {
    /// Select family names; unavailable glyphs keep the rasterizer's system fallback.
    pub fn new(english: impl Into<String>, chinese: impl Into<String>) -> Self {
        Self {
            english: english.into(),
            chinese: chinese.into(),
        }
    }
    /// Family used for Latin text and step numbers.
    pub fn english(&self) -> &str {
        &self.english
    }
    /// Family used for CJK text and full-width punctuation.
    pub fn chinese(&self) -> &str {
        &self.chinese
    }
    /// Resolve the Latin preference to an installed platform family when it is a
    /// system font alias. Explicit family names are returned unchanged.
    pub fn english_family(&self) -> &str {
        if self.english == SYSTEM_MONO_FONT {
            system_mono()
        } else if self.english == SYSTEM_UI_FONT {
            system_ui(false)
        } else {
            self.english()
        }
    }
    /// Resolve the CJK preference to an installed platform family when it is a
    /// system font alias. Explicit family names are returned unchanged.
    pub fn chinese_family(&self) -> &str {
        if self.chinese == SYSTEM_MONO_FONT {
            system_mono()
        } else if self.chinese == SYSTEM_UI_FONT {
            system_ui(true)
        } else {
            self.chinese()
        }
    }
}

fn cjk(ch: char) -> bool {
    matches!(ch as u32, 0x2e80..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0xfe30..=0xfe4f | 0xff00..=0xffef | 0x20000..=0x323af)
}

/// Assign actual SVG spans rather than relying on the Latin font lacking CJK glyphs.
pub(crate) fn apply_to_svg(svg: &str, fonts: &DiagramFonts) -> Result<String, roxmltree::Error> {
    let document = roxmltree::Document::parse(svg)?;
    let mut output = String::with_capacity(svg.len());
    let mut cursor = 0;
    for node in document
        .descendants()
        .filter(|node| node.is_text() && node.ancestors().any(|parent| parent.has_tag_name("text")))
    {
        let range = node.range();
        output.push_str(&svg[cursor..range.start]);
        let text = node.text().unwrap_or_default();
        let mut start = 0;
        let mut chinese = text.chars().next().is_some_and(cjk);
        for (index, ch) in text.char_indices() {
            // Combining marks and spaces stay with their surrounding script.
            let next =
                if ch.is_whitespace() || matches!(ch as u32, 0x0300..=0x036f | 0xfe00..=0xfe0f) {
                    chinese
                } else {
                    cjk(ch)
                };
            if next != chinese {
                write_span(&mut output, &text[start..index], chinese, fonts);
                start = index;
                chinese = next;
            }
        }
        write_span(&mut output, &text[start..], chinese, fonts);
        cursor = range.end;
    }
    output.push_str(&svg[cursor..]);
    Ok(output)
}

fn write_span(output: &mut String, text: &str, chinese: bool, fonts: &DiagramFonts) {
    if text.is_empty() {
        return;
    }
    output.push_str("<tspan font-family=\"");
    output.push_str(&quick_xml::escape::escape(if chinese {
        fonts.chinese_family()
    } else {
        fonts.english_family()
    }));
    output.push_str("\">");
    output.push_str(&quick_xml::escape::escape(text));
    output.push_str("</tspan>");
}

/// System fonts, loaded once per process and shared by diagrams and exports.
pub fn database() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut fonts = usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_labels_keep_the_selected_family_for_each_actual_glyph() {
        let mut families = vec![DiagramFonts::default()];
        for english in ["Helvetica Neue", "Consolas", "IIInsomnia"] {
            if database()
                .faces()
                .any(|face| face.families.iter().any(|(family, _)| family == english))
            {
                families.push(DiagramFonts::new(english, SYSTEM_UI_FONT));
            }
        }
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" width="600" height="120"><text x="20" y="30" font-size="18"><tspan>API 中文 version</tspan><tspan x="20" dy="30">配置 sn + targetVersion</tspan><tspan x="20" dy="30" font-weight="700">步骤 12 MQTT</tspan></text></svg>"#;
        let renderer = crate::Renderer::new();
        for fonts in families {
            check_glyph_families(&apply_to_svg(source, &fonts).unwrap(), &fonts);
            for appearance in [crate::Appearance::Light, crate::Appearance::Dark] {
                for source in [
                    "flowchart LR\n A[Reader 阅读器] -->|Request 请求 API| B[Worker 任务]",
                    "sequenceDiagram\n autonumber\n participant A as Reader 阅读器\n participant B as Worker 任务\n A->>B: API 中文 version\n Note over A,B: 配置 sn + targetVersion\n B-->>A: Result 结果",
                ] {
                    let diagram = renderer
                        .render_with_fonts(source, appearance, &fonts)
                        .unwrap();
                    check_glyph_families(diagram.svg(), &fonts);
                }
            }
        }
    }

    fn check_glyph_families(svg: &str, fonts: &DiagramFonts) {
        let tree = usvg::Tree::from_str(
            svg,
            &usvg::Options {
                fontdb: database(),
                ..Default::default()
            },
        )
        .unwrap();
        fn visit(
            group: &usvg::Group,
            tree: &usvg::Tree,
            fonts: &DiagramFonts,
            checked: &mut usize,
        ) {
            for node in group.children() {
                match node {
                    usvg::Node::Group(group) => visit(group, tree, fonts, checked),
                    usvg::Node::Text(text) => {
                        for glyph in text
                            .layouted()
                            .iter()
                            .flat_map(|span| &span.positioned_glyphs)
                        {
                            let expected = if glyph.text.chars().any(cjk) {
                                fonts.chinese_family()
                            } else if glyph.text.chars().any(|ch| ch.is_ascii_alphanumeric()) {
                                fonts.english_family()
                            } else {
                                continue;
                            };
                            let face = tree.fontdb().face(glyph.font).unwrap();
                            assert!(
                                face.families.iter().any(|(name, _)| name == expected),
                                "mixed label glyph {:?} uses {:?}, expected {expected}",
                                glyph.text,
                                face.families
                            );
                            *checked += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut checked = 0;
        visit(tree.root(), &tree, fonts, &mut checked);
        assert!(checked > 10, "actual mixed-script glyphs must be checked");
    }
    #[test]
    fn mixed_script_spans_preserve_entities_positions_and_font_weights() {
        let input = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="12" y="25" font-weight="700">Reader &#x4e2d;文（API） &amp; code &lt;x&gt; é</text></svg>"#;
        let result = apply_to_svg(input, &DiagramFonts::new("Latin & Serif", "CJK Font")).unwrap();
        let tree = roxmltree::Document::parse(&result).unwrap();
        let text = tree
            .descendants()
            .find(|node| node.has_tag_name("text"))
            .unwrap();
        assert_eq!(text.attribute("x"), Some("12"));
        assert_eq!(text.attribute("font-weight"), Some("700"));
        let spans: Vec<_> = text.children().filter(|node| node.is_element()).collect();
        assert!(spans.iter().any(|span| span.text() == Some("中文（")
            && span.attribute("font-family") == Some("CJK Font")));
        assert!(spans.iter().any(|span| span.text() == Some("API")
            && span.attribute("font-family") == Some("Latin & Serif")));
        let preserved: String = text
            .descendants()
            .filter(|node| node.is_text())
            .map(|node| node.text().unwrap())
            .collect();
        assert_eq!(preserved, "Reader 中文（API） & code <x> é");
        let system = DiagramFonts::new(".SystemUIFont", ".SystemUIFont");
        let aliases = apply_to_svg(input, &system).unwrap();
        assert!(aliases.contains(&format!("font-family=\"{}\"", system_ui(false))));
        assert!(aliases.contains(&format!("font-family=\"{}\"", system_ui(true))));
        assert_eq!(
            system.english(),
            ".SystemUIFont",
            "stored identity stays stable for redraw comparisons"
        );
    }
    #[test]
    fn renderer_windows_share_font_discovery() {
        assert!(Arc::ptr_eq(&database(), &database()));
    }

    #[test]
    fn portable_defaults_resolve_to_installed_monospace_and_cjk_faces() {
        let fonts = DiagramFonts::default();
        let db = database();
        assert!(
            db.faces().any(|face| face.monospaced
                && face
                    .families
                    .iter()
                    .any(|(name, _)| name == fonts.english_family())),
            "English default must resolve to a real installed monospace face"
        );
        assert!(
            db.faces().any(|face| face
                .families
                .iter()
                .any(|(name, _)| name == fonts.chinese_family())),
            "Chinese default must resolve to an installed face"
        );
        assert_eq!(fonts.english(), SYSTEM_MONO_FONT);
        assert_eq!(fonts.chinese(), SYSTEM_UI_FONT);
    }
}
