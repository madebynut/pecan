//! Unsaved drafts are mirrored to disk, so a crash, logout or OS-initiated quit loses nothing.
//!
//! Each running process owns one session directory under the recovery root and holds an
//! exclusive lock on its `lock` file. On startup, a session whose lock can be taken belongs
//! to a process that has ended; its drafts are offered back to the user once.
use super::super::{MAX_OPEN_BYTES, Reader, Tab};
use super::Readers;
use crate::reading::DocumentView;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, EntityId, Global, PromptLevel, Task,
    Window, component::input::Rope,
};
use pecan_document::{Document, MAX_DOCUMENT_BYTES};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

/// How long typing may go unrecorded.
const INTERVAL: Duration = Duration::from_secs(2);
const SNAPSHOT_VERSION: u32 = 1;
/// JSON escaping can grow a draft up to sixfold.
const MAX_SNAPSHOT_BYTES: u64 = 6 * MAX_DOCUMENT_BYTES as u64 + 64 * 1024;

/// The current process's recovery session.
pub(crate) struct DraftStore {
    root: PathBuf,
    session: PathBuf,
    /// Files whose drafts were deliberately discarded; a late write must not recreate them.
    discarded: Arc<Mutex<HashSet<PathBuf>>>,
    _lock: File,
}
impl Global for DraftStore {}

/// Create this process's session under `root`. Without it, drafts stay in memory only.
pub(crate) fn install(root: PathBuf, cx: &mut App) {
    match create_session(&root) {
        Ok((session, lock)) => cx.set_global(DraftStore {
            root,
            session,
            discarded: Arc::default(),
            _lock: lock,
        }),
        Err(error) => tracing::warn!(%error, "draft recovery is unavailable"),
    }
}

fn create_session(root: &Path) -> std::io::Result<(PathBuf, File)> {
    private_dir(root)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let session = root.join(format!("{}-{nanos:x}", std::process::id()));
    private_dir(&session)?;
    let lock = File::create(session.join("lock"))?;
    lock.lock()?;
    Ok((session, lock))
}

fn private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    /// The file the draft edits; `None` for the welcome guide.
    path: Option<PathBuf>,
    title: String,
    /// Where relative images resolve.
    directory: PathBuf,
    /// BLAKE3 of the source the draft started from, to detect later external edits.
    saved_hash: String,
    draft: String,
}

struct Write {
    file: PathBuf,
    path: Option<PathBuf>,
    title: String,
    directory: PathBuf,
    saved: Arc<Document>,
    draft: Rope,
}

fn write_snapshot(
    session: &Path,
    discarded: &Mutex<HashSet<PathBuf>>,
    write: Write,
) -> anyhow::Result<()> {
    let snapshot = Snapshot {
        version: SNAPSHOT_VERSION,
        path: write.path,
        title: write.title,
        directory: write.directory,
        saved_hash: blake3::hash(write.saved.source().as_bytes())
            .to_hex()
            .to_string(),
        draft: write.draft.to_string(),
    };
    let bytes = serde_json::to_vec(&snapshot)?;
    let mut temporary = tempfile::NamedTempFile::new_in(session)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    let discarded = discarded
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !discarded.contains(&write.file) {
        pecan_document::files::persist(temporary, &write.file)?;
    }
    Ok(())
}

fn discard(discarded: &Mutex<HashSet<PathBuf>>, file: PathBuf) {
    let mut discarded = discarded
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Err(error) = fs::remove_file(&file)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, file = %file.display(), "unable to discard a recovered draft");
    }
    discarded.insert(file);
}

/// Per-window bookkeeping of which drafts are on disk, and at which revision.
pub(in crate::app) struct Drafts {
    recorded: HashMap<EntityId, (PathBuf, u64)>,
    job: Option<Task<()>>,
    _ticker: Task<()>,
}
impl Drafts {
    pub(in crate::app) fn new(cx: &mut Context<Reader>) -> Self {
        let ticker = cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor().timer(INTERVAL).await;
                if weak
                    .update(cx, |reader, cx| reader.sync_drafts(cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            recorded: HashMap::new(),
            job: None,
            _ticker: ticker,
        }
    }
}

impl Reader {
    /// Write changed drafts and delete those that were saved or closed.
    pub(in crate::app) fn sync_drafts(&mut self, cx: &mut Context<Self>) {
        let Some(store) = cx.try_global::<DraftStore>() else {
            return;
        };
        // One batch at a time keeps a later deletion from racing an earlier write.
        if self.drafts.job.is_some() {
            return;
        }
        let session = store.session.clone();
        let discarded = store.discarded.clone();
        let mut writes = Vec::new();
        let mut dirty = HashSet::new();
        for tab in &self.tabs {
            let Some(edit) = tab.edit.as_ref().filter(|edit| edit.dirty) else {
                continue;
            };
            let id = tab.view.entity_id();
            dirty.insert(id);
            if self.drafts.recorded.get(&id).map(|(_, revision)| *revision) == Some(edit.revision) {
                continue;
            }
            let file = session.join(format!("{:016x}.json", id.as_u64()));
            self.drafts
                .recorded
                .insert(id, (file.clone(), edit.revision));
            writes.push(Write {
                file,
                path: tab.path.clone(),
                title: tab.title.clone(),
                directory: tab.view.read(cx).directory.to_path_buf(),
                saved: edit.saved.clone(),
                draft: edit.editor.read(cx).text().clone(),
            });
        }
        let mut removals = Vec::new();
        self.drafts.recorded.retain(|id, (file, _)| {
            let keep = dirty.contains(id);
            if !keep {
                removals.push(file.clone());
            }
            keep
        });
        if writes.is_empty() && removals.is_empty() {
            return;
        }
        let background = cx.background_spawn(async move {
            for write in writes {
                let file = write.file.clone();
                if let Err(error) = write_snapshot(&session, &discarded, write) {
                    tracing::warn!(%error, file = %file.display(), "unable to record a draft");
                }
            }
            for file in removals {
                if let Err(error) = fs::remove_file(&file)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(%error, file = %file.display(), "unable to remove a draft");
                }
            }
        });
        self.drafts.job = Some(cx.spawn(async move |weak, cx| {
            background.await;
            let _ = weak.update(cx, |reader, _| reader.drafts.job = None);
        }));
    }

    /// Delete the recorded drafts of these tabs because the user chose not to keep them.
    pub(in crate::app) fn discard_drafts(
        &mut self,
        ids: impl IntoIterator<Item = EntityId>,
        cx: &App,
    ) {
        let Some(store) = cx.try_global::<DraftStore>() else {
            return;
        };
        for id in ids {
            if let Some((file, _)) = self.drafts.recorded.remove(&id) {
                discard(&store.discarded, file);
            }
        }
    }

    /// Delete every recorded draft of this window: each was saved or deliberately discarded.
    pub(in crate::app) fn discard_all_drafts(&mut self, cx: &App) {
        let ids: Vec<_> = self.drafts.recorded.keys().copied().collect();
        self.discard_drafts(ids, cx);
    }

    fn restore_drafts(
        &mut self,
        drafts: Vec<Prepared>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<PathBuf> {
        let session = cx
            .try_global::<DraftStore>()
            .map(|store| store.session.clone());
        let mut restored = Vec::new();
        for prepared in drafts {
            let Prepared {
                file,
                title,
                draft,
                draft_document,
                original,
                fallback,
            } = prepared;
            let identity = original.as_ref().map(|original| original.identity.clone());
            let open = identity.as_ref().is_some_and(|identity| {
                self.tabs
                    .iter()
                    .any(|tab| tab.identity.as_ref() == Some(identity))
            });
            let bytes = draft.len().max(
                original
                    .as_ref()
                    .map_or(0, |original| original.loaded.document.source().len()),
            );
            if bytes > MAX_OPEN_BYTES || !self.make_room(bytes, cx) {
                self.status = "打开文档已达预算，部分草稿留待下次启动时恢复".into();
                continue;
            }
            if let Some(tab) = self.tabs.get(self.active) {
                tab.view.update(cx, |view, cx| view.suspend(cx));
            }
            let (tab, keep_saved) = match original.filter(|_| !open) {
                // The file is unchanged since the draft began: edit it in place, with the
                // usual conflict check on save.
                Some(Original {
                    path,
                    identity,
                    loaded,
                }) => {
                    let resources = Arc::new(loaded.resources);
                    let directory = resources.directory().to_path_buf();
                    let document = Arc::new(loaded.document);
                    (
                        Tab {
                            path: Some(path),
                            identity: Some(identity),
                            title,
                            view: cx.new(|cx| {
                                DocumentView::new(
                                    document,
                                    directory,
                                    self.queue.clone(),
                                    self.math.clone(),
                                    cx,
                                )
                            }),
                            resources: Some(resources),
                            edit: None,
                        },
                        true,
                    )
                }
                // The file changed, moved or was never Markdown: show the draft itself.
                // Saving to a changed file reports a conflict instead of overwriting it.
                _ => {
                    let Fallback {
                        path,
                        identity,
                        directory,
                    } = fallback;
                    let document = Arc::new(draft_document);
                    (
                        Tab {
                            path: if open { None } else { path },
                            identity: if open { None } else { identity },
                            title,
                            view: cx.new(|cx| {
                                DocumentView::new(
                                    document,
                                    directory,
                                    self.queue.clone(),
                                    self.math.clone(),
                                    cx,
                                )
                            }),
                            resources: None,
                            edit: None,
                        },
                        false,
                    )
                }
            };
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
            let Some(id) = self.ensure_editor(window, cx) else {
                continue;
            };
            let tab = &mut self.tabs[self.active];
            tab.view.update(cx, |view, cx| view.suspend(cx));
            let Some(edit) = tab.edit.as_mut() else {
                continue;
            };
            if keep_saved {
                edit.editor
                    .update(cx, |editor, cx| editor.set_value(draft, window, cx));
            } else {
                // Nothing on disk matches this draft, so it stays unsaved until written.
                edit.saved_text = Rope::new();
            }
            edit.revision += 1;
            edit.dirty = true;
            edit.show_source = true;
            let revision = edit.revision;
            // Adopt the orphaned snapshot rather than rewriting it.
            if let Some(session) = &session {
                let adopted = session.join(format!("{:016x}.json", id.as_u64()));
                match fs::rename(&file, &adopted) {
                    Ok(()) => {
                        self.drafts.recorded.insert(id, (adopted, revision));
                    }
                    Err(error) => {
                        tracing::warn!(%error, "unable to adopt a recovered draft");
                    }
                }
            }
            restored.push(file);
        }
        if !restored.is_empty() {
            self.status = format!("已恢复 {} 份未保存的草稿，请检查后保存", restored.len());
            self.focus_document(window, cx);
        }
        self.sync_drafts(cx);
        cx.notify();
        restored
    }
}

/// Drafts left behind by a process that ended, still locked against other instances.
struct Orphan {
    directory: PathBuf,
    lock: Option<File>,
    snapshots: Vec<(PathBuf, Snapshot)>,
}

fn scan(root: &Path, current: &Path) -> Vec<Orphan> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut orphans = Vec::new();
    for entry in entries.flatten() {
        let directory = entry.path();
        if directory == current || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        // A session whose lock is still held belongs to a running instance.
        let lock = match File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("lock"))
        {
            Ok(lock) => match lock.try_lock() {
                Ok(()) => lock,
                Err(_) => continue,
            },
            Err(error) => {
                tracing::warn!(%error, directory = %directory.display(), "unable to inspect drafts");
                continue;
            }
        };
        let mut snapshots = Vec::new();
        for file in fs::read_dir(&directory).into_iter().flatten().flatten() {
            let file = file.path();
            if file.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            match read_snapshot(&file) {
                Ok(snapshot) => snapshots.push((file, snapshot)),
                Err(error) => {
                    // Keep unreadable data out of future scans without deleting it.
                    tracing::warn!(%error, file = %file.display(), "unreadable draft set aside");
                    let _ = fs::rename(&file, file.with_extension("corrupt"));
                }
            }
        }
        let mut orphan = Orphan {
            directory,
            lock: Some(lock),
            snapshots,
        };
        if orphan.snapshots.is_empty() {
            remove_session(&mut orphan);
        } else {
            orphans.push(orphan);
        }
    }
    orphans
}

fn read_snapshot(file: &Path) -> anyhow::Result<Snapshot> {
    let mut bytes = Vec::new();
    File::open(file)?
        .take(MAX_SNAPSHOT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_SNAPSHOT_BYTES,
        "draft snapshot is too large"
    );
    let snapshot: Snapshot = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        snapshot.version == SNAPSHOT_VERSION,
        "unknown draft snapshot version"
    );
    anyhow::ensure!(
        snapshot.draft.len() <= MAX_DOCUMENT_BYTES,
        "draft exceeds 32 MiB"
    );
    Ok(snapshot)
}

/// Remove a session once it holds no drafts. Set-aside `.corrupt` files keep it in place.
fn remove_session(orphan: &mut Orphan) {
    // Windows cannot delete a file while this process still holds it open.
    drop(orphan.lock.take());
    let empty = fs::read_dir(&orphan.directory)
        .is_ok_and(|entries| entries.flatten().all(|entry| entry.file_name() == "lock"));
    if empty {
        let _ = fs::remove_file(orphan.directory.join("lock"));
        let _ = fs::remove_dir(&orphan.directory);
    }
}

struct Original {
    path: PathBuf,
    identity: PathBuf,
    loaded: pecan_import::LoadedDocument,
}

struct Fallback {
    path: Option<PathBuf>,
    identity: Option<PathBuf>,
    directory: PathBuf,
}

struct Prepared {
    file: PathBuf,
    title: String,
    draft: String,
    draft_document: Document,
    original: Option<Original>,
    fallback: Fallback,
}

/// Decide how one draft reopens. `None` means it is already on disk and can be dropped.
fn prepare(file: PathBuf, snapshot: Snapshot) -> anyhow::Result<Option<Prepared>> {
    let draft_hash = blake3::hash(snapshot.draft.as_bytes()).to_hex().to_string();
    let current = snapshot
        .path
        .as_ref()
        .filter(|path| pecan_import::is_markdown(path))
        .and_then(|path| fs::read(path).ok())
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string());
    if current.as_deref() == Some(draft_hash.as_str()) {
        return Ok(None);
    }
    let draft_document = Document::parse(snapshot.draft.clone())?;
    let original = match (&snapshot.path, current) {
        (Some(path), Some(current)) if current == snapshot.saved_hash => {
            let identity = pecan_document::files::canonicalize(path)?;
            Some(Original {
                path: path.clone(),
                identity,
                loaded: crate::workspace::load(path)?,
            })
        }
        _ => None,
    };
    let path = snapshot.path.filter(|path| path.is_file());
    let identity = path
        .as_deref()
        .and_then(|path| pecan_document::files::canonicalize(path).ok());
    let directory = Some(snapshot.directory)
        .filter(|directory| directory.is_dir())
        .or_else(|| {
            path.as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        })
        .or_else(std::env::home_dir)
        .unwrap_or_default();
    Ok(Some(Prepared {
        file,
        title: snapshot.title,
        draft: snapshot.draft,
        draft_document,
        original,
        fallback: Fallback {
            path,
            identity,
            directory,
        },
    }))
}

fn front_reader(cx: &App) -> Option<(AnyWindowHandle, Entity<Reader>)> {
    let active = cx.active_window();
    let readers = cx.try_global::<Readers>()?;
    let live = || {
        readers
            .0
            .iter()
            .filter_map(|(window, reader)| Some((*window, reader.upgrade()?)))
    };
    live()
        .find(|(window, _)| Some(*window) == active)
        .or_else(|| live().next_back())
}

/// Offer drafts from sessions that ended without saving them.
pub(crate) fn offer(cx: &mut App) {
    let Some(store) = cx.try_global::<DraftStore>() else {
        return;
    };
    let root = store.root.clone();
    let session = store.session.clone();
    cx.spawn(async move |cx| {
        let mut orphans = cx
            .background_spawn(async move { scan(&root, &session) })
            .await;
        let count: usize = orphans.iter().map(|orphan| orphan.snapshots.len()).sum();
        if count == 0 {
            return;
        }
        let Some((handle, reader)) = cx.update(|cx| front_reader(cx)) else {
            return;
        };
        let Ok(answer) = handle.update(cx, |_, window, cx| {
            window.prompt(
                PromptLevel::Warning,
                &format!("发现 {count} 份未保存的草稿"),
                Some("碧果上次没有正常退出，或系统在退出时没有询问是否保存。恢复后草稿会以源码打开，原文件不会被修改。"),
                &["恢复", "稍后", "丢弃"],
                cx,
            )
        }) else {
            return;
        };
        match answer.await.ok() {
            Some(0) => {
                let snapshots: Vec<_> = orphans
                    .iter_mut()
                    .flat_map(|orphan| std::mem::take(&mut orphan.snapshots))
                    .collect();
                let prepared = cx
                    .background_spawn(async move {
                        snapshots
                            .into_iter()
                            .filter_map(|(file, snapshot)| match prepare(file.clone(), snapshot) {
                                Ok(Some(prepared)) => Some(prepared),
                                Ok(None) => {
                                    let _ = fs::remove_file(&file);
                                    None
                                }
                                Err(error) => {
                                    tracing::warn!(%error, file = %file.display(), "unable to reopen a draft");
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                let _ = handle.update(cx, |_, window, cx| {
                    reader.update(cx, |reader, cx| reader.restore_drafts(prepared, window, cx))
                });
                cx.background_spawn(async move {
                    for orphan in &mut orphans {
                        remove_session(orphan);
                    }
                })
                .await;
            }
            Some(2) => {
                cx.background_spawn(async move {
                    for orphan in &mut orphans {
                        for (file, _) in std::mem::take(&mut orphan.snapshots) {
                            let _ = fs::remove_file(file);
                        }
                        remove_session(orphan);
                    }
                })
                .await;
            }
            // Later: release the locks and ask again at the next launch.
            _ => drop(orphans),
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(path: Option<PathBuf>, saved: &str, draft: &str) -> Snapshot {
        Snapshot {
            version: SNAPSHOT_VERSION,
            path,
            title: "a.md".into(),
            directory: PathBuf::new(),
            saved_hash: blake3::hash(saved.as_bytes()).to_hex().to_string(),
            draft: draft.into(),
        }
    }

    #[test]
    fn sessions_of_running_processes_are_skipped_and_ended_ones_are_offered() {
        let root = tempfile::tempdir().unwrap();
        let (live, live_lock) = create_session(root.path()).unwrap();
        let (ended, ended_lock) = create_session(root.path()).unwrap();
        let (current, _current_lock) = create_session(root.path()).unwrap();
        for session in [&live, &ended, &current] {
            let bytes = serde_json::to_vec(&snapshot(None, "", "# Draft")).unwrap();
            fs::write(session.join("1.json"), bytes).unwrap();
        }
        fs::write(ended.join("2.json"), b"{not json").unwrap();
        drop(ended_lock);
        let (empty, empty_lock) = create_session(root.path()).unwrap();
        drop(empty_lock);

        let mut orphans = scan(root.path(), &current);
        assert_eq!(
            orphans.len(),
            1,
            "only the ended session with drafts is offered"
        );
        assert_eq!(orphans[0].directory, ended);
        assert_eq!(orphans[0].snapshots.len(), 1);
        assert_eq!(orphans[0].snapshots[0].1.draft, "# Draft");
        assert!(
            ended.join("2.corrupt").exists(),
            "unreadable drafts are set aside"
        );
        assert!(!empty.exists(), "empty ended sessions are removed");
        assert!(
            scan(root.path(), &current).is_empty(),
            "a session being recovered is locked against other instances"
        );
        for (file, _) in std::mem::take(&mut orphans[0].snapshots) {
            fs::remove_file(file).unwrap();
        }
        remove_session(&mut orphans[0]);
        assert!(ended.exists(), "set-aside files keep their session");
        drop(live_lock);
    }

    #[test]
    fn discarded_drafts_are_not_recreated_by_a_late_write() {
        let session = tempfile::tempdir().unwrap();
        let discarded = Mutex::default();
        let file = session.path().join("1.json");
        let write = || Write {
            file: file.clone(),
            path: None,
            title: "a.md".into(),
            directory: PathBuf::new(),
            saved: Arc::new(Document::parse(String::new()).unwrap()),
            draft: Rope::from("draft"),
        };
        write_snapshot(session.path(), &discarded, write()).unwrap();
        assert_eq!(read_snapshot(&file).unwrap().draft, "draft");
        discard(&discarded, file.clone());
        write_snapshot(session.path(), &discarded, write()).unwrap();
        assert!(!file.exists());
        assert_eq!(fs::read_dir(session.path()).unwrap().count(), 0);
    }

    use crate::app::tests::settle;
    use gpui_kit::{EntityInputHandler as _, TestAppContext, WindowOptions};

    fn json_files(directory: &Path) -> Vec<PathBuf> {
        fs::read_dir(directory)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect()
    }

    fn reader_window(
        cx: &mut TestAppContext,
        path: Option<PathBuf>,
    ) -> (AnyWindowHandle, Entity<Reader>) {
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Reader::new(path, pecan_config::startup::Launch::Returning, window, cx))
            })
            .unwrap()
        });
        settle(cx);
        (handle, reader)
    }

    #[gpui_kit::test]
    fn drafts_are_recorded_restored_in_place_and_removed_once_saved_or_discarded(
        cx: &mut TestAppContext,
    ) {
        let root = tempfile::tempdir().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.md");
        fs::write(&path, "# Saved\n").unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::menus::install(cx);
            install(root.path().to_path_buf(), cx);
        });
        let session = cx.update(|cx| cx.global::<DraftStore>().session.clone());
        let (first, editing) = reader_window(cx, Some(path.clone()));
        cx.update_window(first, |_, window, cx| {
            editing.update(cx, |reader, cx| reader.toggle_source(window, cx));
            let editor = editing.read(cx).tabs[0]
                .edit
                .as_ref()
                .unwrap()
                .editor
                .clone();
            editor.update(cx, |editor, cx| {
                let end = editor.text().to_string().encode_utf16().count();
                editor.replace_text_in_range(Some(0..end), "# Draft\n", window, cx);
            });
        })
        .unwrap();
        settle(cx);
        editing.update(cx, |reader, cx| reader.sync_drafts(cx));
        settle(cx);
        let recorded = json_files(&session);
        assert_eq!(recorded.len(), 1, "a dirty draft is mirrored to disk");
        let snapshot = read_snapshot(&recorded[0]).unwrap();
        assert_eq!(snapshot.draft, "# Draft\n");
        assert_eq!(snapshot.path.as_deref(), Some(path.as_path()));

        // Restore a copy, as a later process would, into a window without the file open.
        let orphan = root.path().join("orphan.json");
        fs::copy(&recorded[0], &orphan).unwrap();
        let prepared = prepare(orphan.clone(), read_snapshot(&orphan).unwrap())
            .unwrap()
            .unwrap();
        let (second, restoring) = reader_window(cx, None);
        cx.update_window(second, |_, window, cx| {
            restoring.update(cx, |reader, cx| {
                assert_eq!(reader.restore_drafts(vec![prepared], window, cx).len(), 1);
            });
        })
        .unwrap();
        settle(cx);
        assert!(
            !orphan.exists(),
            "the restored snapshot is adopted, not duplicated"
        );
        assert_eq!(json_files(&session).len(), 2);
        restoring.read_with(cx, |reader, cx| {
            let tab = &reader.tabs[reader.active];
            assert_eq!(tab.path.as_deref(), Some(path.as_path()));
            assert!(tab.protected() && tab.source_mode());
            let edit = tab.edit.as_ref().unwrap();
            assert_eq!(edit.editor.read(cx).text().to_string(), "# Draft\n");
        });
        cx.update_window(second, |_, window, cx| {
            restoring.update(cx, |reader, cx| {
                reader.save_active(super::super::SaveMode::Existing, window, cx)
            });
        })
        .unwrap();
        settle(cx);
        assert_eq!(fs::read_to_string(&path).unwrap(), "# Draft\n");
        restoring.update(cx, |reader, cx| reader.sync_drafts(cx));
        settle(cx);
        assert_eq!(json_files(&session).len(), 1, "a saved draft is removed");

        cx.update_window(first, |_, window, cx| {
            editing.update(cx, |reader, cx| reader.close_tab(0, window, cx));
        })
        .unwrap();
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("不保存");
        settle(cx);
        assert!(
            json_files(&session).is_empty(),
            "a declined draft is removed at once"
        );
    }

    #[test]
    fn drafts_reopen_in_place_only_when_the_file_is_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a.md");
        fs::write(&path, "# Saved").unwrap();
        let file = directory.path().join("draft.json");

        let unchanged = prepare(
            file.clone(),
            snapshot(Some(path.clone()), "# Saved", "# Draft"),
        )
        .unwrap()
        .unwrap();
        assert!(unchanged.original.is_some());
        assert_eq!(unchanged.draft, "# Draft");

        fs::write(&path, "# Edited elsewhere").unwrap();
        let changed = prepare(
            file.clone(),
            snapshot(Some(path.clone()), "# Saved", "# Draft"),
        )
        .unwrap()
        .unwrap();
        assert!(changed.original.is_none());
        assert_eq!(changed.fallback.path.as_deref(), Some(path.as_path()));

        fs::write(&path, "# Draft").unwrap();
        assert!(
            prepare(
                file.clone(),
                snapshot(Some(path.clone()), "# Saved", "# Draft")
            )
            .unwrap()
            .is_none(),
            "a draft already on disk is not offered"
        );

        fs::remove_file(&path).unwrap();
        let deleted = prepare(file, snapshot(Some(path), "# Saved", "# Draft"))
            .unwrap()
            .unwrap();
        assert!(deleted.original.is_none() && deleted.fallback.path.is_none());
    }
}
