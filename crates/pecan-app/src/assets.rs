//! Default component assets plus reading symbols and native code toolbar icons.
use std::borrow::Cow;

use gpui_kit::{AssetSource, SharedString, assets::Assets};

gpui_kit::assets::icon_assets!(
    DocumentIcons,
    [
        CodeXml,
        Save,
        SaveCheck,
        Sigma,
        Network,
        Download,
        Lightbulb,
        ListIndentIncrease,
        Braces,
        FoldVertical,
        UnfoldVertical,
        Image,
        ImageOff
    ]
);

#[derive(Debug)]
pub(crate) struct ReaderAssets;

impl AssetSource for ReaderAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = crate::language_icons::ICONS
            .iter()
            .chain(std::iter::once(&crate::language_icons::GENERIC))
            .find(|icon| icon.path == path)
        {
            return Ok(Some(Cow::Borrowed(icon.svg)));
        }
        match DocumentIcons.load(path)? {
            Some(icon) => Ok(Some(icon)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(DocumentIcons.list(path)?);
        paths.extend(
            crate::language_icons::ICONS
                .iter()
                .chain(std::iter::once(&crate::language_icons::GENERIC))
                .filter(|icon| icon.path.starts_with(path))
                .map(|icon| SharedString::from(icon.path)),
        );
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::assets::IconName;

    #[test]
    fn document_source_icon_and_default_controls_are_embedded() {
        for icon in [
            IconName::Download,
            IconName::CodeXml,
            IconName::Save,
            IconName::SaveCheck,
            IconName::Sigma,
            IconName::Network,
            IconName::Copy,
            IconName::ListIndentIncrease,
            IconName::Braces,
            IconName::FoldVertical,
            IconName::UnfoldVertical,
            IconName::Maximize,
            IconName::Lightbulb,
            IconName::Info,
            IconName::CircleAlert,
            IconName::TriangleAlert,
            IconName::Ban,
            IconName::CircleX,
        ] {
            let path = icon.path();
            let bytes = ReaderAssets.load(&path).unwrap().unwrap();
            assert!(bytes.starts_with(b"<svg"), "{path} has embedded SVG data");
            assert!(ReaderAssets.list("icons/").unwrap().contains(&path));
        }
        assert!(matches!(
            ReaderAssets.load(&IconName::CodeXml.path()).unwrap(),
            Some(Cow::Borrowed(_))
        ));
    }
    #[test]
    fn code_controls_rasterize_at_their_actual_icon_size() {
        use gpui_kit::{DevicePixels, SvgRenderer, SvgSize, size};
        let renderer = SvgRenderer::new(std::sync::Arc::new(ReaderAssets));
        for icon in [
            IconName::ListIndentIncrease,
            IconName::Save,
            IconName::SaveCheck,
            IconName::Braces,
            IconName::FoldVertical,
            IconName::UnfoldVertical,
            IconName::Copy,
        ] {
            let path = icon.path();
            let bytes = ReaderAssets
                .load(&path)
                .unwrap()
                .expect("embedded code icon");
            let parsed = renderer.parse_svg(&bytes).expect("valid code SVG");
            let image = renderer
                .render_parsed(
                    &parsed,
                    SvgSize::ExactSize(size(DevicePixels(16), DevicePixels(16))),
                )
                .unwrap();
            let visible = image
                .as_bytes(0)
                .unwrap()
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[3] > 32)
                .count();
            assert!(
                visible > 16,
                "code control {path} renders an empty or invisible icon"
            );
        }
    }

    #[test]
    fn devicon_marks_are_local_and_visible_at_normal_and_retina_sizes() {
        use gpui_kit::{DevicePixels, SvgRenderer, SvgSize, size};
        let source = ReaderAssets;
        let renderer = SvgRenderer::new(std::sync::Arc::new(ReaderAssets));
        let paths = source.list("language-icons/").unwrap();
        for icon in crate::language_icons::ICONS
            .iter()
            .chain(std::iter::once(&crate::language_icons::GENERIC))
        {
            assert!(
                paths.iter().any(|path| path.as_ref() == icon.path),
                "{} is listed",
                icon.path
            );
            let bytes = source.load(icon.path).unwrap().unwrap();
            assert!(
                matches!(bytes, Cow::Borrowed(_)),
                "assets have no runtime copy"
            );
            let parsed = renderer.parse_svg(&bytes).unwrap();
            for pixels in [16, 32] {
                let image = renderer
                    .render_parsed(
                        &parsed,
                        SvgSize::ExactSize(size(DevicePixels(pixels), DevicePixels(pixels))),
                    )
                    .unwrap();
                let visible = image
                    .as_bytes(0)
                    .unwrap()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|pixel| pixel[3] > 32)
                    .count();
                assert!(visible > 16, "{} is invisible at {pixels}px", icon.path);
            }
        }
    }
}
