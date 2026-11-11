//! Shared bounds for background rendering across readers and exports.
use crate::Renderer;
use std::sync::Arc;
/// Shared renderer caches and at most two simultaneous CPU render tasks.
#[derive(Clone, Debug)]
pub struct RenderQueue {
    /// Native renderer, whose Clone shares its existing cache.
    pub renderer: Renderer,
    /// Semaphore shared with every clone of this queue.
    pub slots: Arc<async_lock::Semaphore>,
}
impl Default for RenderQueue {
    fn default() -> Self {
        Self::new()
    }
}
impl RenderQueue {
    /// Create one renderer and one shared two-task gate.
    pub fn new() -> Self {
        Self {
            renderer: Renderer::new(),
            slots: Arc::new(async_lock::Semaphore::new(2)),
        }
    }
}
