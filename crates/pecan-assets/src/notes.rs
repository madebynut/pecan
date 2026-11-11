//! Shared light callout styles and their flat Lucide SVG sources.
use pecan_document::AlertKind;
/// Export uses the same symbol color as a light reading surface.
pub fn export_mark(kind: Option<AlertKind>) -> [u8; 3] {
    let mark: u32 = kind.map_or(0xc4841a, |kind| light_colors(kind).2);
    let [_, red, green, blue] = mark.to_be_bytes();
    [red, green, blue]
}

/// Print containers use the same light palette as reading, regardless of application theme.
pub fn export_colors(kind: Option<AlertKind>) -> ([u8; 3], [u8; 3]) {
    let (fill, edge, _) = light_colors(kind.unwrap_or(AlertKind::Warning));
    let rgb = |color: u32| {
        let [_, r, g, b] = color.to_be_bytes();
        [r, g, b]
    };
    (rgb(fill), rgb(edge))
}

pub fn light_colors(kind: AlertKind) -> (u32, u32, u32) {
    match kind {
        AlertKind::Note | AlertKind::Info => (0xe8f2ff, 0xc5daf6, 0x1d4f91),
        AlertKind::Tip => (0xe7f6ee, 0xc6e6d2, 0x1d7a45),
        AlertKind::Important => (0xf4eaff, 0xe0c8f6, 0x6b3fa0),
        AlertKind::Warning => (0xfff6e8, 0xf0e0c0, 0xc4841a),
        AlertKind::Caution | AlertKind::Error => (0xfdecec, 0xf5c8c8, 0xa33b3b),
    }
}

/// One static Lucide asset, with the same name used by native controls.
pub fn svg(kind: Option<AlertKind>) -> &'static [u8] {
    match kind {
        None | Some(AlertKind::Tip) => include_bytes!("../../../assets/icons/lightbulb.svg"),
        Some(AlertKind::Note | AlertKind::Info) => include_bytes!("../../../assets/icons/info.svg"),
        Some(AlertKind::Important) => include_bytes!("../../../assets/icons/circle-alert.svg"),
        Some(AlertKind::Warning) => include_bytes!("../../../assets/icons/triangle-alert.svg"),
        Some(AlertKind::Caution) => include_bytes!("../../../assets/icons/ban.svg"),
        Some(AlertKind::Error) => include_bytes!("../../../assets/icons/circle-x.svg"),
    }
}
