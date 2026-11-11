use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use gpui_kit::{
    App, AppContext, Context, Entity, Global, Subscription, Task,
    component::{Theme, ThemeMode},
};

pub(crate) struct Preferences {
    /// Shared so per-frame readers clone a pointer rather than nine font strings.
    pub(crate) data: Arc<PreferencesData>,
    pub(crate) error: Option<String>,
    path: Option<PathBuf>,
    generation: Arc<AtomicU64>,
    write_lock: Arc<Mutex<()>>,
    save_job: Option<Task<()>>,
    _quit_subscription: Subscription,
}

struct SharedPreferences(Entity<Preferences>);
impl Global for SharedPreferences {}

impl Preferences {
    pub(crate) fn install(path: Option<PathBuf>, cx: &mut App) -> Entity<Self> {
        let mut error = None;
        let mut imported = false;
        let mut data = path
            .as_ref()
            .and_then(|path| match read_config_with_legacy(path) {
                Ok((bytes, legacy)) if bytes.len() <= 64 * 1024 => {
                    imported = legacy;
                    match serde_json::from_slice::<PreferencesData>(&bytes) {
                        Ok(data) => Some(data),
                        Err(e) => {
                            error = Some(format!("设置读取失败：{e}"));
                            None
                        }
                    }
                }
                Ok(_) => {
                    error = Some("设置文件超过 64 KiB".into());
                    None
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    error = Some(format!("设置读取失败：{e}"));
                    None
                }
            })
            .unwrap_or_default();
        let migrated = data.migrate_defaults();
        data.validate();
        let entity = cx.new(|cx: &mut Context<Self>| {
            let quit_subscription = cx.on_app_quit(|prefs, cx| {
                let path = prefs
                    .path
                    .clone()
                    .filter(|_| prefs.generation.load(Ordering::SeqCst) > 0);
                let data = prefs.data.clone();
                let lock = prefs.write_lock.clone();
                prefs.generation.fetch_add(1, Ordering::SeqCst);
                prefs.save_job = None;
                cx.background_spawn(async move {
                    if let Some(path) = path {
                        let _guard = lock.lock().expect("settings write lock poisoned");
                        if let Err(error) = save(&path, &data) {
                            tracing::error!(%error,"unable to flush settings before quit");
                        }
                    }
                })
            });
            Self {
                data: Arc::new(data),
                error,
                path,
                generation: Arc::new(AtomicU64::new(0)),
                write_lock: Arc::new(Mutex::new(())),
                save_job: None,
                _quit_subscription: quit_subscription,
            }
        });
        cx.set_global(SharedPreferences(entity.clone()));
        if migrated || imported {
            entity.update(cx, |prefs, cx| prefs.change(|_| {}, cx));
        }
        entity
    }
    pub(crate) fn shared(cx: &mut App) -> Entity<Self> {
        if !cx.has_global::<SharedPreferences>() {
            Self::install(None, cx);
        }
        cx.global::<SharedPreferences>().0.clone()
    }
    pub(crate) fn change(
        &mut self,
        update: impl FnOnce(&mut PreferencesData),
        cx: &mut Context<Self>,
    ) {
        let data = Arc::make_mut(&mut self.data);
        update(data);
        data.validate();
        apply_theme(&self.data, cx);
        self.error = None;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(path) = self.path.clone() {
            let data = self.data.clone();
            let current = self.generation.clone();
            let lock = self.write_lock.clone();
            self.save_job = Some(cx.spawn(async move |weak, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let result = cx
                    .background_spawn(async move {
                        let _guard = lock.lock().expect("settings write lock poisoned");
                        if current.load(Ordering::SeqCst) != generation {
                            return Ok(());
                        }
                        save(&path, &data)
                    })
                    .await;
                let _ = weak.update(cx, |prefs, cx| {
                    if prefs.generation.load(Ordering::SeqCst) == generation {
                        prefs.error = result.err().map(|e| format!("设置保存失败：{e}"));
                        cx.notify();
                    }
                });
            }));
        }
        cx.notify();
    }
}

pub(crate) fn apply_theme(data: &PreferencesData, cx: &mut App) {
    Theme::change(theme_mode(data, cx), None, cx);
    Theme::update(cx, |theme| {
        crate::brand::apply(theme);
        theme.font_family = crate::fonts::family(&data.ui_font).to_owned().into();
        theme.font_size = gpui_kit::px(data.ui_size as f32);
        theme.mono_font_family = crate::fonts::family(&data.code_font).to_owned().into();
        theme.mono_font_size = gpui_kit::px(data.code_size as f32);
    });
}

fn theme_mode(data: &PreferencesData, cx: &App) -> ThemeMode {
    match data.theme.as_str() {
        "dark" => ThemeMode::Dark,
        "light" => ThemeMode::Light,
        _ => match cx.window_appearance() {
            gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark => {
                ThemeMode::Dark
            }
            _ => ThemeMode::Light,
        },
    }
}

pub(crate) use pecan_config::{
    PreferencesData, config_directory, read_config_with_legacy, save_preferences as save,
};
