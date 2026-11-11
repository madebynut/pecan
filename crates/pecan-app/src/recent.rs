//! Bounded recent paths; file contents never enter the history or its persistence tasks.
use gpui_kit::{App, AppContext, Context, Entity, Global, Subscription, Task};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

pub(crate) use pecan_config::recent::{Entry, Kind, display_path};
use pecan_config::recent::{History, read, save};
#[derive(Debug, Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = pecan, no_json)]
pub(crate) struct OpenRecent {
    pub(crate) entry: Entry,
}

pub(crate) struct Recent {
    data: Arc<History>,
    path: Option<PathBuf>,
    generation: Arc<AtomicU64>,
    write_lock: Arc<Mutex<()>>,
    save_job: Option<Task<()>>,
    _quit: Subscription,
}
struct SharedRecent(Entity<Recent>);
impl Global for SharedRecent {}
impl Recent {
    pub(crate) fn install(path: Option<PathBuf>, cx: &mut App) -> Entity<Self> {
        let data = path.as_deref().map_or_else(History::default, |path| {
            read(path).unwrap_or_else(|error| {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(%error, "unable to read recent history");
                }
                History::default()
            })
        });
        let entity = cx.new(|cx: &mut Context<Self>| {
            let quit = cx.on_app_quit(|recent, cx| {
                let path = recent
                    .path
                    .clone()
                    .filter(|_| recent.generation.load(Ordering::SeqCst) > 0);
                let data = recent.data.clone();
                let lock = recent.write_lock.clone();
                recent.generation.fetch_add(1, Ordering::SeqCst);
                recent.save_job = None;
                cx.background_spawn(async move {
                    if let Some(path) = path {
                        let _guard = lock.lock().expect("recent history write lock poisoned");
                        if let Err(error) = save(&path, &data) {
                            tracing::error!(%error, "unable to flush recent history before quit");
                        }
                    }
                })
            });
            Self {
                data: Arc::new(data),
                path,
                generation: Arc::new(AtomicU64::new(0)),
                write_lock: Arc::new(Mutex::new(())),
                save_job: None,
                _quit: quit,
            }
        });
        cx.set_global(SharedRecent(entity.clone()));
        entity
    }
    pub(crate) fn shared(cx: &mut App) -> Entity<Self> {
        if !cx.has_global::<SharedRecent>() {
            Self::install(None, cx);
        }
        cx.global::<SharedRecent>().0.clone()
    }
    pub(crate) fn entries(&self) -> &[Entry] {
        self.data.entries()
    }
    /// Paths are canonicalized by the successful background opener, never during a frame.
    pub(crate) fn record(&mut self, entry: Entry, cx: &mut Context<Self>) {
        if self.entries().first() == Some(&entry) {
            return;
        }
        if Arc::make_mut(&mut self.data).record(entry) {
            self.changed(cx);
        }
    }
    pub(crate) fn clear(&mut self, cx: &mut Context<Self>) {
        if self.entries().is_empty() {
            return;
        }
        Arc::make_mut(&mut self.data).clear();
        self.changed(cx);
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        let revision = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(path) = self.path.clone() {
            let data = self.data.clone();
            let current = self.generation.clone();
            let lock = self.write_lock.clone();
            self.save_job = Some(cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                cx.background_spawn(async move {
                    let _guard = lock.lock().expect("recent history write lock poisoned");
                    if current.load(Ordering::SeqCst) == revision
                        && let Err(error) = save(&path, &data)
                    {
                        tracing::error!(%error, "unable to save recent history");
                    }
                })
                .await;
            }));
        }
        cx.defer(crate::menus::refresh);
        cx.notify();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use std::path::Path;
    fn entry(root: &Path, name: &str, kind: Kind) -> Entry {
        Entry {
            path: root.join(name),
            kind,
        }
    }
    #[gpui_kit::test]
    fn immediate_shutdown_flushes_history_before_the_debounce(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recent.json");
        let recent = cx.update(|cx| Recent::install(Some(path.clone()), cx));
        let file = entry(directory.path(), "last.md", Kind::File);
        cx.update(|cx| recent.update(cx, |recent, cx| recent.record(file.clone(), cx)));
        cx.update(|cx| cx.shutdown());
        assert_eq!(
            read(&path).unwrap().entries(),
            [file],
            "quitting before 300ms still saves the latest record"
        );
    }

    #[gpui_kit::test]
    fn debounce_clear_and_restart_keep_the_latest_history(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Pecan/recent.json");
        let recent = cx.update(|cx| Recent::install(Some(path.clone()), cx));
        let file = entry(directory.path(), "first.md", Kind::File);
        cx.update(|cx| recent.update(cx, |recent, cx| recent.record(file.clone(), cx)));
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(350));
        cx.run_until_parked();
        assert_eq!(read(&path).unwrap().entries(), [file]);
        cx.update(|cx| {
            recent.update(cx, |recent, cx| {
                recent.record(entry(directory.path(), "second.md", Kind::File), cx);
                recent.clear(cx);
            })
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(350));
        cx.run_until_parked();
        assert!(
            read(&path).unwrap().entries().is_empty(),
            "a delayed old write must not restore cleared history"
        );
        let restored = cx.update(|cx| Recent::install(Some(path), cx));
        cx.update(|cx| assert!(restored.read(cx).entries().is_empty()));
    }
}
