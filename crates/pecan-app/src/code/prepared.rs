//! GPUI task lifecycle for the independent formatting batch.
use gpui_kit::{AppContext as _, Context, Task};
use pecan_document::Document;
use pecan_format::{CodeSnapshot, FormattedCode, SLOTS};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
pub(crate) struct CodeFormats {
    pub(crate) preference: String,
    prepared: Option<CodeSnapshot>,
    cancel: Arc<AtomicBool>,
    _job: Task<()>,
}
impl CodeFormats {
    pub(crate) fn new(document: Arc<Document>, preference: String, cx: &mut Context<Self>) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let configured = preference.clone();
        let background = cx.background_spawn(async move {
            let _permit = SLOTS.acquire().await;
            CodeSnapshot::prepare(&document, configured, &worker_cancel)
        });
        let job = cx.spawn(async move |weak, cx| {
            let prepared = background.await;
            let _ = weak.update(cx, |state, cx| {
                state.prepared = Some(prepared);
                cx.notify();
            });
        });
        Self {
            preference,
            prepared: None,
            cancel,
            _job: job,
        }
    }
    pub(crate) fn get(&self, language: &str, source: &str) -> Option<Arc<FormattedCode>> {
        self.prepared.as_ref()?.get(language, source)
    }
    pub(crate) fn snapshot(&self) -> CodeSnapshot {
        self.prepared.clone().unwrap_or_default()
    }
    pub(crate) fn ready(&self) -> bool {
        self.prepared.is_some()
    }
}
impl Drop for CodeFormats {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
