//! Persisted reading preferences, migrations and bounded configuration I/O.
use pecan_mermaid::{SYSTEM_MONO_FONT, SYSTEM_UI_FONT};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
/// Persisted reading settings; schema migrations preserve user choices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PreferencesData {
    /// Body font for English and Latin text.
    pub english_font: String,
    /// Body font for Chinese text.
    pub chinese_font: String,
    /// Interface font for English and Latin labels.
    pub ui_font: String,
    #[serde(default)]
    /// Interface font for Chinese labels.
    pub ui_chinese_font: String,
    /// Selected bundled OpenType math face.
    pub math_font: String,
    /// Code block font family.
    pub code_font: String,
    /// Automatic detection or a 2, 4 or 8-column Tab width.
    pub code_tab_size: String,
    /// Mermaid font for English text.
    pub mermaid_english_font: String,
    /// Mermaid font for Chinese text.
    pub mermaid_chinese_font: String,
    #[serde(default)]
    /// Version of the untouched-default migration schema.
    pub defaults_version: u8,
    /// System, light or dark appearance preference.
    pub theme: String,
    /// Body font size in logical pixels.
    pub font_size: f64,
    /// Interface font size in logical pixels.
    pub ui_size: f64,
    /// Code font size in logical pixels.
    pub code_size: f64,
    /// Math font size in logical pixels.
    pub math_size: f64,
    /// Body line height relative to its font size.
    pub line_height: f64,
    /// Preferred body content width in logical pixels.
    pub reading_width: f64,
    /// Reload open documents when their source file changes.
    pub auto_reload: bool,
    /// Exit when the reader's last document tab is closed.
    pub quit_on_last_tab: bool,
    /// Ask GitHub Releases once per launch whether a newer version exists.
    pub check_updates: bool,
}

impl Default for PreferencesData {
    fn default() -> Self {
        Self {
            english_font: SYSTEM_MONO_FONT.into(),
            chinese_font: SYSTEM_UI_FONT.into(),
            ui_font: SYSTEM_MONO_FONT.into(),
            ui_chinese_font: SYSTEM_UI_FONT.into(),
            math_font: "stix".into(),
            code_font: SYSTEM_MONO_FONT.into(),
            code_tab_size: "auto".into(),
            mermaid_english_font: SYSTEM_MONO_FONT.into(),
            mermaid_chinese_font: SYSTEM_UI_FONT.into(),
            defaults_version: 7,
            theme: "system".into(),
            font_size: 16.,
            ui_size: 14.,
            code_size: 14.,
            math_size: 18.,
            line_height: 1.6,
            reading_width: 960.,
            auto_reload: true,
            quit_on_last_tab: false,
            check_updates: true,
        }
    }
}

impl PreferencesData {
    /// Migrate untouched historical defaults once, preserving user choices.
    pub fn migrate_defaults(&mut self) -> bool {
        if self.defaults_version >= 7 {
            return false;
        }
        if self.defaults_version == 6 {
            if self.line_height == 1.8 {
                self.line_height = 1.6;
            }
            self.defaults_version = 7;
            return true;
        }
        if self.defaults_version < 4 {
            // Migrate untouched legacy defaults while preserving custom sizes and font names.
            if self.defaults_version == 0 && self.font_size == 17. {
                self.font_size = 14.;
            }
            if self.defaults_version < 2
                && (self.ui_size == 15. || self.defaults_version == 0 && self.ui_size == 13.)
            {
                self.ui_size = 14.;
            }
            if self.defaults_version < 3 && self.reading_width == 800. {
                self.reading_width = 960.;
            }
            if self.font_size == 14. {
                self.font_size = 16.;
            }
            if self.ui_chinese_font.is_empty() {
                self.ui_chinese_font = if self.ui_font == ".SystemUIFont" {
                    "PingFang SC".into()
                } else {
                    self.ui_font.clone()
                };
            }
        }
        if self.defaults_version < 5 && self.math_size == 19. {
            self.math_size = 18.;
        }
        for (font, previous, next) in [
            (&mut self.english_font, "Georgia", SYSTEM_MONO_FONT),
            (&mut self.chinese_font, "PingFang SC", SYSTEM_UI_FONT),
            (&mut self.ui_font, ".SystemUIFont", SYSTEM_MONO_FONT),
            (&mut self.ui_chinese_font, "PingFang SC", SYSTEM_UI_FONT),
            (&mut self.code_font, "Menlo", SYSTEM_MONO_FONT),
            (
                &mut self.mermaid_english_font,
                "Helvetica Neue",
                SYSTEM_MONO_FONT,
            ),
            (
                &mut self.mermaid_chinese_font,
                "PingFang SC",
                SYSTEM_UI_FONT,
            ),
        ] {
            if font == previous {
                *font = next.into();
            }
        }
        if self.line_height == 1.8 {
            self.line_height = 1.6;
        }
        self.defaults_version = 7;
        true
    }
    /// Clamp invalid persisted values and restore safe defaults.
    pub fn validate(&mut self) {
        for (value, default, min, max) in [
            (&mut self.font_size, 16., 12., 32.),
            (&mut self.code_size, 14., 10., 32.),
            (&mut self.ui_size, 14., 11., 20.),
            (&mut self.math_size, 18., 12., 36.),
            (&mut self.line_height, 1.6, 1.2, 2.6),
            (&mut self.reading_width, 960., 480., 1600.),
        ] {
            *value = if value.is_finite() {
                value.clamp(min, max)
            } else {
                default
            };
        }
        for (font, default) in [
            (&mut self.english_font, SYSTEM_MONO_FONT),
            (&mut self.chinese_font, SYSTEM_UI_FONT),
            (&mut self.ui_font, SYSTEM_MONO_FONT),
            (&mut self.ui_chinese_font, SYSTEM_UI_FONT),
            (&mut self.code_font, SYSTEM_MONO_FONT),
            (&mut self.mermaid_english_font, SYSTEM_MONO_FONT),
            (&mut self.mermaid_chinese_font, SYSTEM_UI_FONT),
        ] {
            if font.trim().is_empty() || font.len() > 256 || font.chars().any(char::is_control) {
                *font = default.into();
            }
        }
        if !["auto", "2", "4", "8"].contains(&self.code_tab_size.as_str()) {
            self.code_tab_size = "auto".into();
        }
        if !["stix", "modern"].contains(&self.math_font.as_str()) {
            self.math_font = "stix".into();
        }
        if !["system", "light", "dark"].contains(&self.theme.as_str()) {
            self.theme = "system".into();
        }
    }
    /// Whether built reading rows remain valid. Reload behavior does not affect them, and
    /// the theme choice reaches rows only through the palette, which has its own observer.
    pub fn same_presentation(&self, other: &Self) -> bool {
        self.english_font == other.english_font
            && self.chinese_font == other.chinese_font
            && self.ui_font == other.ui_font
            && self.ui_chinese_font == other.ui_chinese_font
            && self.math_font == other.math_font
            && self.code_font == other.code_font
            && self.code_tab_size == other.code_tab_size
            && self.mermaid_english_font == other.mermaid_english_font
            && self.mermaid_chinese_font == other.mermaid_chinese_font
            && self.defaults_version == other.defaults_version
            && self.font_size == other.font_size
            && self.ui_size == other.ui_size
            && self.code_size == other.code_size
            && self.math_size == other.math_size
            && self.line_height == other.line_height
            && self.reading_width == other.reading_width
    }
    /// Independent font preferences for the shared native Mermaid renderer.
    pub fn diagram_fonts(&self) -> pecan_mermaid::DiagramFonts {
        pecan_mermaid::DiagramFonts::new(
            self.mermaid_english_font.clone(),
            self.mermaid_chinese_font.clone(),
        )
    }
}

/// Store new state in the native per-user location, retaining the existing macOS path.
pub fn config_directory() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/Pecan"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("Pecan"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|root| Path::new(root).is_absolute())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|root| root.join("Pecan"))
    }
}

/// Machine-local storage for crash-recovery copies of unsaved drafts, never roamed or synced.
pub fn recovery_directory() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        config_directory().map(|directory| directory.join("Recovery"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
            .map(|root| root.join("Pecan").join("Recovery"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        log_directory()
            .and_then(|logs| logs.parent().map(Path::to_path_buf))
            .map(|root| root.join("recovery"))
    }
}

/// Keep diagnostics in each platform's per-user log or state location, outside roaming settings.
pub fn log_directory() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Logs/Pecan"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
            .map(|root| root.join("Pecan").join("logs"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("XDG_STATE_HOME")
            .filter(|root| Path::new(root).is_absolute())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .map(|root| root.join("Pecan").join("logs"))
    }
}

/// Read bounded configuration bytes; Pecan wins; missing settings may import Moye, then MyMD without modifying either.
pub fn read_config_with_legacy(path: &Path) -> std::io::Result<(Vec<u8>, bool)> {
    match read_config(path) {
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && path
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|name| name == "Pecan") =>
        {
            let parent = path.parent().and_then(Path::parent).ok_or(error)?;
            for app in ["Moye", "MyMD"] {
                match read_config(&parent.join(app).join("preferences.json")) {
                    Ok(bytes) => return Ok((bytes, true)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no current or legacy settings",
            ))
        }
        result => result.map(|bytes| (bytes, false)),
    }
}

fn read_config(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn save(path: &Path, data: &PreferencesData) -> anyhow::Result<()> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("settings path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec_pretty(data)?)?;
    temporary.as_file().sync_all()?;
    pecan_document::files::persist(temporary, path)?;
    Ok(())
}

/// Save validated preferences atomically, preserving an existing file on failure.
/// # Errors
/// Returns an error when serialization or filesystem I/O fails.
pub fn save_preferences(path: &Path, data: &PreferencesData) -> Result<(), crate::ConfigError> {
    save(path, data).map_err(crate::ConfigError::new)
}
#[cfg(test)]
mod tests {
    #[test]
    fn last_tab_exit_defaults_off_for_existing_preferences_and_persists_when_enabled() {
        let mut data: super::PreferencesData = serde_json::from_str(r#"{"font_size":22}"#).unwrap();
        assert!(!data.quit_on_last_tab);
        assert!(
            data.check_updates,
            "settings saved before the option existed keep checking"
        );
        data.quit_on_last_tab = true;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        super::save_preferences(&path, &data).unwrap();
        let saved: super::PreferencesData =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(saved.quit_on_last_tab);
        assert_eq!(saved.font_size, 22.);
    }
    use super::*;

    #[test]
    fn line_height_migrates_only_the_old_default_and_keeps_version_six_fonts() {
        for version in [0, 5, 6] {
            for (old, expected) in [(1.8, 1.6), (1.5, 1.5), (2.1, 2.1)] {
                let mut data: PreferencesData = serde_json::from_str(&format!(
                    r#"{{"defaults_version":{version},"line_height":{old},"english_font":"Georgia","code_font":"Menlo","font_size":22}}"#
                )).unwrap();
                assert!(data.migrate_defaults());
                assert_eq!(data.line_height, expected, "version {version}, old {old}");
                assert_eq!(data.font_size, 22.);
                if version == 6 {
                    assert_eq!(
                        data.english_font, "Georgia",
                        "do not reapply old font migrations"
                    );
                    assert_eq!(data.code_font, "Menlo");
                }
                let mut restored: PreferencesData =
                    serde_json::from_slice(&serde_json::to_vec(&data).unwrap()).unwrap();
                assert!(!restored.migrate_defaults());
                assert_eq!(restored, data);
            }
        }
        let mut data = PreferencesData {
            line_height: f64::NAN,
            ..Default::default()
        };
        data.validate();
        assert_eq!(data.line_height, 1.6);
        let base = PreferencesData::default();
        data.line_height = 1.7;
        assert!(
            !base.same_presentation(&data),
            "line height rebuilds existing reading rows"
        );
    }

    #[test]
    fn reload_and_theme_choice_keep_reading_rows() {
        let base = PreferencesData::default();
        let mut other = base.clone();
        other.auto_reload = !other.auto_reload;
        other.quit_on_last_tab = true;
        other.theme = "dark".into();
        assert!(base.same_presentation(&other));
        other.font_size += 1.;
        assert!(!base.same_presentation(&other));
    }

    #[test]
    fn platform_font_migration_preserves_custom_choices_and_sizes() {
        let mut old: PreferencesData = serde_json::from_str(r#"{"defaults_version":5,"english_font":"Georgia","chinese_font":"PingFang SC","ui_font":".SystemUIFont","ui_chinese_font":"PingFang SC","code_font":"Menlo","mermaid_english_font":"Helvetica Neue","mermaid_chinese_font":"PingFang SC","font_size":22,"code_size":17,"math_size":19}"#).unwrap();
        assert!(old.migrate_defaults());
        assert_eq!(old.english_font, SYSTEM_MONO_FONT);
        assert_eq!(old.chinese_font, SYSTEM_UI_FONT);
        assert_eq!(old.ui_font, SYSTEM_MONO_FONT);
        assert_eq!(old.ui_chinese_font, SYSTEM_UI_FONT);
        assert_eq!(old.code_font, SYSTEM_MONO_FONT);
        assert_eq!(old.diagram_fonts(), pecan_mermaid::DiagramFonts::default());
        assert_eq!(
            (old.font_size, old.code_size, old.math_size),
            (22., 17., 19.)
        );
        assert!(!old.migrate_defaults());
        let mut custom: PreferencesData = serde_json::from_str(r#"{"defaults_version":5,"english_font":"Palatino","chinese_font":"Songti SC","ui_font":"Avenir","ui_chinese_font":"Heiti SC","code_font":"Courier New","mermaid_english_font":"DIN Alternate","mermaid_chinese_font":"Hiragino Sans GB"}"#).unwrap();
        let before = custom.clone();
        custom.migrate_defaults();
        custom.defaults_version = before.defaults_version;
        assert_eq!(custom, before, "user-selected families remain independent");
    }

    #[test]
    fn diagram_fonts_default_legacy_settings_and_roundtrip_independently() {
        let mut data: PreferencesData =
            serde_json::from_str(r#"{"defaults_version":5,"english_font":"Georgia"}"#).unwrap();
        assert_eq!(data.diagram_fonts(), pecan_mermaid::DiagramFonts::default());
        data.mermaid_english_font = "Menlo".into();
        data.mermaid_chinese_font = "Songti SC".into();
        let restored: PreferencesData =
            serde_json::from_str(&serde_json::to_string(&data).unwrap()).unwrap();
        assert_eq!(
            restored.diagram_fonts(),
            pecan_mermaid::DiagramFonts::new("Menlo", "Songti SC")
        );
        assert_eq!(restored.english_font, "Georgia");
        data.mermaid_english_font = "\n".into();
        data.mermaid_chinese_font = "".into();
        data.validate();
        assert_eq!(data.diagram_fonts(), pecan_mermaid::DiagramFonts::default());
    }
    #[test]
    fn latex_default_migrates_without_reapplying_body_defaults() {
        for (old_size, expected) in [(19., 18.), (24., 24.)] {
            let mut data: PreferencesData = serde_json::from_str(&format!(
                r#"{{"defaults_version":4,"font_size":14,"math_size":{old_size}}}"#
            ))
            .unwrap();
            assert!(data.migrate_defaults());
            assert_eq!(data.math_size, expected);
            assert_eq!(
                data.font_size, 14.,
                "custom body size survives the LaTeX migration"
            );
            assert!(!data.migrate_defaults(), "migration runs only once");
        }
    }
    #[test]
    fn legacy_defaults_migrate_once_and_custom_sizes_and_fonts_survive() {
        let mut defaults: PreferencesData = serde_json::from_str(r#"{"font_size":17,"ui_size":13,"english_font":"DIN Alternate","chinese_font":"Custom CJK","ui_font":"Avenir","code_font":"Courier New"}"#).unwrap();
        assert!(defaults.migrate_defaults());
        defaults.validate();
        assert_eq!((defaults.font_size, defaults.ui_size), (16., 14.));
        assert_eq!(defaults.english_font, "DIN Alternate");
        assert_eq!(defaults.chinese_font, "Custom CJK");
        assert_eq!(defaults.ui_font, "Avenir");
        assert_eq!(defaults.code_font, "Courier New");
        let serialized = serde_json::to_string(&defaults).unwrap();
        let mut reloaded: PreferencesData = serde_json::from_str(&serialized).unwrap();
        assert!(!reloaded.migrate_defaults());
        assert_eq!(reloaded, defaults);
        let mut custom: PreferencesData =
            serde_json::from_str(r#"{"font_size":20,"ui_size":18}"#).unwrap();
        custom.migrate_defaults();
        assert_eq!((custom.font_size, custom.ui_size), (20., 18.));
        assert!(!PreferencesData::default().migrate_defaults());
        let mut previous: PreferencesData = serde_json::from_str(
            r#"{"defaults_version":1,"ui_size":15,"font_size":17,"english_font":"Palatino"}"#,
        )
        .unwrap();
        assert!(previous.migrate_defaults());
        assert_eq!(
            (previous.ui_size, previous.font_size),
            (14., 17.),
            "only the previous default UI size changes"
        );
        assert_eq!(previous.english_font, "Palatino");
        assert!(!previous.migrate_defaults());
    }

    #[test]
    fn typography_migrates_and_independent_settings_roundtrip() {
        let mut legacy: PreferencesData = serde_json::from_str(
            r#"{"defaults_version":3,"font_size":14,"ui_size":14,"ui_font":"Avenir","code_font":"Menlo"}"#,
        ).unwrap();
        assert!(legacy.migrate_defaults());
        assert_eq!(
            (legacy.font_size, legacy.code_size, legacy.ui_size),
            (16., 14., 14.)
        );
        assert_eq!(legacy.ui_chinese_font, "Avenir");
        let mut custom: PreferencesData = serde_json::from_str(
            r#"{"defaults_version":3,"font_size":20,"ui_font":"Menlo","ui_chinese_font":"Songti SC","code_size":18}"#,
        ).unwrap();
        custom.migrate_defaults();
        assert_eq!((custom.font_size, custom.code_size), (20., 18.));
        assert_eq!(custom.ui_chinese_font, "Songti SC");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        save(&path, &custom).unwrap();
        let mut restored: PreferencesData =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(restored, custom);
        assert!(!restored.migrate_defaults());
        restored.code_size = f64::NAN;
        restored.validate();
        assert_eq!(restored.code_size, 14.);
        assert_eq!(restored.font_size, 20.);
    }

    #[test]
    fn preferences_roundtrip_and_invalid_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let data = PreferencesData {
            math_font: "modern".into(),
            chinese_font: "Songti SC".into(),
            english_font: "Palatino".into(),
            ui_font: "Menlo".into(),
            ..Default::default()
        };
        save(&path, &data).unwrap();
        assert_eq!(
            serde_json::from_slice::<PreferencesData>(&std::fs::read(path).unwrap()).unwrap(),
            data
        );
        let mut bad = PreferencesData {
            font_size: f64::NAN,
            line_height: 100.,
            english_font: "PingFang SC".into(),
            ..Default::default()
        };
        bad.validate();
        assert_eq!(bad.font_size, 16.);
        assert_eq!(bad.line_height, 2.6);
        assert_eq!(bad.english_font, "PingFang SC");
    }

    #[test]
    fn reading_width_migrates_and_custom_widths_survive() {
        let mut old: PreferencesData =
            serde_json::from_str(r#"{"defaults_version":2,"reading_width":800,"ui_size":15}"#)
                .unwrap();
        assert!(old.migrate_defaults());
        assert_eq!(old.reading_width, 960.);
        assert_eq!(
            old.ui_size, 15.,
            "existing custom UI sizes remain unchanged"
        );
        let mut custom: PreferencesData =
            serde_json::from_str(r#"{"defaults_version":2,"reading_width":1120}"#).unwrap();
        custom.migrate_defaults();
        assert_eq!(custom.reading_width, 1120.);
        let mut bad = PreferencesData {
            reading_width: f64::NAN,
            ..Default::default()
        };
        bad.validate();
        assert_eq!(bad.reading_width, 960.);
        bad.reading_width = 9999.;
        bad.validate();
        assert_eq!(bad.reading_width, 1600.);
    }

    #[test]
    fn code_indent_setting_defaults_validates_and_round_trips() {
        let old: PreferencesData = serde_json::from_str("{}").unwrap();
        assert_eq!(old.code_tab_size, "auto");
        for width in ["auto", "2", "4", "8"] {
            let mut data = PreferencesData {
                code_tab_size: width.into(),
                ..Default::default()
            };
            data.validate();
            let saved = serde_json::to_string(&data).unwrap();
            assert_eq!(
                serde_json::from_str::<PreferencesData>(&saved)
                    .unwrap()
                    .code_tab_size,
                width
            );
        }
        for width in ["0", "999", "invalid"] {
            let mut data = PreferencesData {
                code_tab_size: width.into(),
                ..Default::default()
            };
            data.validate();
            assert_eq!(data.code_tab_size, "auto");
        }
    }

    #[test]
    fn new_settings_import_old_app_once_and_take_precedence() {
        let directory = tempfile::tempdir().unwrap();
        let old = directory.path().join("MyMD/preferences.json");
        let new = directory.path().join("Pecan/preferences.json");
        let old_data = PreferencesData {
            english_font: "Avenir".into(),
            reading_width: 960.,
            ui_size: 18.,
            ..Default::default()
        };
        save(&old, &old_data).unwrap();
        let (bytes, imported) = read_config_with_legacy(&new).unwrap();
        assert!(imported);
        let imported_data: PreferencesData = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            imported_data, old_data,
            "renaming the app preserves user settings"
        );
        let recent = directory.path().join("Moye/preferences.json");
        let recent_data = PreferencesData {
            code_size: 18.,
            ui_chinese_font: "Heiti SC".into(),
            ..old_data.clone()
        };
        save(&recent, &recent_data).unwrap();
        let (bytes, imported) = read_config_with_legacy(&new).unwrap();
        assert!(imported);
        assert_eq!(
            serde_json::from_slice::<PreferencesData>(&bytes).unwrap(),
            recent_data,
            "Moye preferences take priority over the earlier MyMD settings"
        );
        save(&new, &PreferencesData::default()).unwrap();
        assert!(
            !read_config_with_legacy(&new).unwrap().1,
            "existing Pecan settings win"
        );
        std::fs::write(&new, b"broken settings").unwrap();
        assert_eq!(
            read_config_with_legacy(&new).unwrap(),
            (b"broken settings".to_vec(), false),
            "an invalid current file cannot silently revive old settings"
        );
        assert_eq!(
            read_config(&old).unwrap(),
            serde_json::to_vec_pretty(&old_data).unwrap(),
            "import never modifies the old application file"
        );
        assert_eq!(
            read_config(&recent).unwrap(),
            serde_json::to_vec_pretty(&recent_data).unwrap(),
            "importing settings preserves the previous app's file"
        );
    }

    #[test]
    fn presentation_comparison_matches_legacy_for_each_persisted_setting() {
        let base = PreferencesData::default();
        let legacy = |value: &PreferencesData| {
            let normalized = PreferencesData {
                auto_reload: value.auto_reload,
                quit_on_last_tab: value.quit_on_last_tab,
                theme: value.theme.clone(),
                ..base.clone()
            };
            normalized == *value
        };
        let check =
            |value: PreferencesData| assert_eq!(base.same_presentation(&value), legacy(&value));
        check(base.clone());
        check(PreferencesData {
            english_font: "custom".into(),
            ..base.clone()
        });
        check(PreferencesData {
            chinese_font: "custom".into(),
            ..base.clone()
        });
        check(PreferencesData {
            ui_font: "custom".into(),
            ..base.clone()
        });
        check(PreferencesData {
            ui_chinese_font: "custom".into(),
            ..base.clone()
        });
        check(PreferencesData {
            math_font: "custom".into(),
            ..base.clone()
        });
        check(PreferencesData {
            code_font: "custom".into(),
            ..base.clone()
        });
        check(PreferencesData {
            code_tab_size: "custom".into(),
            ..base.clone()
        });
        check(PreferencesData {
            mermaid_english_font: "custom".into(),
            ..base.clone()
        });
        check(PreferencesData {
            mermaid_chinese_font: "custom".into(),
            ..base.clone()
        });
        check(PreferencesData {
            theme: "custom".into(),
            ..base.clone()
        });
        check(PreferencesData {
            font_size: 19.3,
            ..base.clone()
        });
        check(PreferencesData {
            ui_size: 19.3,
            ..base.clone()
        });
        check(PreferencesData {
            code_size: 19.3,
            ..base.clone()
        });
        check(PreferencesData {
            math_size: 19.3,
            ..base.clone()
        });
        check(PreferencesData {
            line_height: 19.3,
            ..base.clone()
        });
        check(PreferencesData {
            reading_width: 19.3,
            ..base.clone()
        });
        check(PreferencesData {
            auto_reload: false,
            ..base.clone()
        });
        check(PreferencesData {
            quit_on_last_tab: true,
            ..base.clone()
        });
        check(PreferencesData {
            defaults_version: 0,
            ..base.clone()
        });
        check(PreferencesData {
            font_size: f64::NAN,
            ..PreferencesData::default()
        });
    }
}
