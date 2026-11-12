use std::{
    num::NonZeroUsize,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui_kit::{
    AnyElement, Context, Entity, ImageSource, IntoElement, ListAlignment, ListOffset, ListState,
    Render, Subscription, Task, Window,
    base::TextView,
    component::{
        ActiveTheme, StyledExt,
        text::{MarkdownExtensions, RangeHighlight, SelectionFormat, TextViewState},
    },
    div, list,
    prelude::*,
    px,
};
use lru::LruCache;
use pecan_document::{Block, Document, SearchMatch};

use crate::code::{CodeCard, CodeFormats};
use crate::diagram::{DiagramCard, RenderQueue};
use crate::presentation::ReadingPalette;
use crate::source::{SourceContent, SourceView};
use crate::{
    math::MathQueue,
    preferences::{Preferences, PreferencesData},
};

/// UI state is bounded independently of document length; evicted rows are reconstructed on demand.
const ROW_CACHE_ENTRIES: usize = 32;
/// Previews retain GPU textures, so their budget is stricter than text rows.
const DIAGRAM_CACHE_ENTRIES: usize = 8;
/// Pending asynchronous Markdown must occupy space until its parse completes.
const UNPARSED_ROW_HEIGHT: f32 = 600.;

enum Row {
    Code(Entity<CodeCard>),
    Markdown(Entity<MarkdownChunk>),
    Mermaid(Entity<DiagramCard>),
    Source(Entity<SourceView>),
}

pub(crate) struct DocumentView {
    pub(crate) document: Arc<Document>,
    pub(crate) scroll: ListState,
    pub(crate) selected_heading: Option<usize>,
    pub(crate) directory: Arc<Path>,
    rows: LruCache<usize, Row>,
    formats: Entity<CodeFormats>,
    _formats_subscription: Subscription,
    deferred_reveal: Option<(usize, Range<usize>, ListOffset)>,
    display_rows: Arc<[usize]>,
    queue: RenderQueue,
    math: MathQueue,
    matches: Arc<[SearchMatch]>,
    pending_reveal: Option<(usize, Range<usize>)>,
    available: gpui_kit::Size<gpui_kit::Pixels>,
}

impl DocumentView {
    pub(crate) fn new(
        document: Arc<Document>,
        directory: PathBuf,
        queue: RenderQueue,
        math: MathQueue,
        cx: &mut Context<Self>,
    ) -> Self {
        // Adjacent source pages of a fence share one native editor viewport.
        let display_rows: Arc<[usize]> = document
            .blocks()
            .iter()
            .enumerate()
            .filter_map(|(index, block)| match block {
                Block::CodePage { content, code, .. } if content.start != code.start => None,
                _ => Some(index),
            })
            .collect();
        let preference = Preferences::shared(cx).read(cx).data.code_tab_size.clone();
        let formats = cx.new(|cx| CodeFormats::new(document.clone(), preference, cx));
        let formats_subscription = cx.observe(&formats, |view, _, cx| {
            if let Some((block, range, position)) = view.deferred_reveal.take()
                && view.scroll.logical_scroll_top() == position
            {
                view.reveal(block, range, cx);
            }
        });
        let end = display_rows.len();
        let measure_rows = display_rows.clone();
        let scroll = ListState::new(end + 1, ListAlignment::Top, px(480.))
            .with_uniform_item_height(px(UNPARSED_ROW_HEIGHT));
        let weak = cx.weak_entity();
        scroll.set_item_measurement_readiness(move |index, cx| {
            if index == end {
                return true;
            }
            weak.upgrade()
                .is_some_and(|view| match view.read(cx).rows.peek(&measure_rows[index]) {
                    Some(Row::Markdown(chunk)) => !chunk.read(cx).loading,
                    Some(_) => true,
                    None => false,
                })
        });
        Self {
            document,
            display_rows,
            formats,
            _formats_subscription: formats_subscription,
            deferred_reveal: None,
            directory: Arc::from(directory),
            scroll,
            selected_heading: None,
            rows: LruCache::new(
                NonZeroUsize::new(ROW_CACHE_ENTRIES).expect("row cache capacity is positive"),
            ),
            queue,
            math,
            matches: Arc::default(),
            pending_reveal: None,
            available: gpui_kit::Size::default(),
        }
    }
    fn display_index(&self, block: usize) -> usize {
        self.display_rows
            .partition_point(|&row| row <= block)
            .saturating_sub(1)
    }
    pub(crate) fn reveal(&mut self, block: usize, range: Range<usize>, cx: &mut Context<Self>) {
        let item_ix = self.display_index(block);
        let source_index = self.display_rows[item_ix];
        let code_size = Preferences::shared(cx).read(cx).data.code_size;
        let offset =
            if let Block::CodePage { code, language, .. } = &self.document.blocks()[source_index] {
                if let Some(Row::Code(card)) = self.rows.peek(&source_index).filter(
                |row| matches!(row, Row::Code(card) if !card.read(cx).awaiting_initial_format()),
            ) {
                card.update(cx, |card, cx| {
                    card.reveal(range.clone(), cx);
                    card.offset_y(range.start, code_size, cx)
                })
            } else {
                // A not-yet-materialized card has no Rope. Count once from its borrowed
                // source; subsequent jumps use the native editor's line index.
                let end = range.start.clamp(code.start, code.end);
                let raw = &self.document.source()[code.clone()];
                let raw = raw
                    .strip_suffix("\r\n")
                    .or_else(|| raw.strip_suffix('\n'))
                    .unwrap_or(raw);
                let row = if let Some(formatted) = self.formats.read(cx).get(language, raw) {
                    let at = formatted
                        .range(end - code.start..(end - code.start + 1).min(raw.len()))
                        .start;
                    formatted.text()[..at]
                        .bytes()
                        .filter(|&b| b == b'\n')
                        .count()
                } else {
                    self.document.source()[code.start..end]
                        .bytes()
                        .filter(|&b| b == b'\n')
                        .count()
                };
                crate::code::row_offset_y(row, code_size)
            }
            } else {
                px(0.)
            };
        self.scroll.scroll_to(ListOffset {
            item_ix,
            offset_in_item: (offset - px(16.)).max(px(0.)),
        });
        self.deferred_reveal =
            (matches!(self.document.blocks()[source_index], Block::CodePage { .. })
                && !self.formats.read(cx).ready())
            .then(|| (block, range.clone(), self.scroll.logical_scroll_top()));
        self.pending_reveal = Some((block, range));
        cx.notify();
    }
    pub(crate) fn reveal_heading(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(heading) = self.document.headings().get(index) else {
            return;
        };
        self.deferred_reveal = None;
        self.selected_heading = Some(index);
        let block = heading.block_index();
        if self.document.blocks()[block].source_range().start == heading.source_range().start {
            self.pending_reveal = None;
            self.scroll.scroll_to(ListOffset {
                item_ix: self.display_index(block),
                offset_in_item: px(0.),
            });
            cx.notify();
        } else {
            // Nested headings retain their surrounding list/quote semantics.
            self.reveal(block, heading.source_range(), cx);
        }
    }
    /// Shares the reader's match list; code cards filter their own source range.
    pub(crate) fn search(&mut self, matches: Arc<[SearchMatch]>, cx: &mut Context<Self>) {
        self.matches = matches;
        for (index, row) in self.rows.iter_mut() {
            match row {
                Row::Code(view) => view.update(cx, |view, cx| {
                    view.highlight(self.matches.iter().map(SearchMatch::source_range), cx)
                }),
                Row::Markdown(view) => {
                    let ranges = block_highlights(&self.document, &self.matches, *index);
                    view.update(cx, |view, cx| view.highlight(ranges, cx));
                }
                Row::Mermaid(_) | Row::Source(_) => {}
            }
        }
        cx.notify();
    }
    #[cfg(test)]
    pub(crate) fn resident_rows(&self) -> usize {
        self.rows.len()
    }
    pub(crate) fn reset_presentation(&mut self, cx: &mut Context<Self>) {
        let preference = Preferences::shared(cx).read(cx).data.code_tab_size.clone();
        if self.formats.read(cx).preference != preference {
            self.formats = cx.new(|cx| CodeFormats::new(self.document.clone(), preference, cx));
            self._formats_subscription = cx.observe(&self.formats, |view, _, cx| {
                if let Some((block, range, position)) = view.deferred_reveal.take()
                    && view.scroll.logical_scroll_top() == position
                {
                    view.reveal(block, range, cx);
                }
            });
        }
        self.rows.clear();
        self.scroll.remeasure();
        cx.notify();
    }
    pub(crate) fn code_snapshot(&self, cx: &gpui_kit::App) -> crate::code::CodeSnapshot {
        self.formats.read(cx).snapshot()
    }
    pub(crate) fn suspend(&mut self, cx: &mut Context<Self>) {
        self.rows.clear();
        self.deferred_reveal = None;
        cx.notify();
    }
    fn markdown_row(
        &mut self,
        range: Range<usize>,
        index: usize,
        prefs: Arc<PreferencesData>,
        cx: &mut Context<Self>,
    ) -> Row {
        let block_source = &self.document.source()[range];
        // Only this slice can contain math; the shared reference suffix may hold `$` in URLs.
        let math = block_source.contains('$');
        let mut source = block_source.to_string();
        source.push_str(self.document.reference_suffix());
        let ranges = block_highlights(&self.document, &self.matches, index);
        let layout = if matches!(self.document.blocks()[index], Block::Callout { .. }) {
            MarkdownLayout::Note
        } else {
            MarkdownLayout::Prose
        };
        Row::Markdown(cx.new(|cx| {
            let mut chunk = MarkdownChunk::new(
                source,
                math,
                self.directory.clone(),
                ranges,
                MarkdownPreparation {
                    queue: self.math.clone(),
                    prefs,
                    formats: Some(self.formats.clone()),
                },
                cx,
            );
            chunk.layout = layout;
            chunk
        }))
    }
    fn row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if index == self.display_rows.len() {
            // Leave room to align even the last heading near the viewport top.
            return div()
                .w_full()
                .h((self.available.height - px(60.)).max(px(0.)))
                .into_any_element();
        }
        let index = self.display_rows[index];
        let prefs = Preferences::shared(cx).read(cx).data.clone();
        let palette = ReadingPalette::for_app(cx);
        if !self.rows.contains(&index) {
            let block = &self.document.blocks()[index];
            if matches!(block, Block::Mermaid { .. })
                && self
                    .rows
                    .iter()
                    .filter(|(_, row)| matches!(row, Row::Mermaid(_)))
                    .count()
                    >= DIAGRAM_CACHE_ENTRIES
            {
                let oldest = self
                    .rows
                    .iter()
                    .rev()
                    .find_map(|(index, row)| matches!(row, Row::Mermaid(_)).then_some(*index));
                if let Some(oldest) = oldest {
                    self.rows.pop(&oldest);
                }
            }
            let row = match block {
                Block::Mermaid { code, .. } => Row::Mermaid(
                    cx.new(|cx| DiagramCard::new(code.clone(), self.queue.clone(), window, cx)),
                ),
                Block::CodePage { code, language, .. } => {
                    let view = cx.new(|cx| {
                        CodeCard::with_formats(
                            self.document.clone(),
                            code.clone(),
                            language.clone(),
                            self.formats.clone(),
                            window,
                            cx,
                        )
                    });
                    view.update(cx, |view, cx| {
                        view.highlight(self.matches.iter().map(SearchMatch::source_range), cx)
                    });
                    Row::Code(view)
                }
                Block::Source { source, pages } => Row::Source(cx.new(|cx| {
                    SourceView::new(
                        SourceContent {
                            document: self.document.clone(),
                            pages: pages.clone(),
                            copy_range: source.clone(),
                        },
                        window,
                        cx,
                    )
                })),
                Block::Callout { body, .. } => {
                    self.markdown_row(body.clone(), index, prefs.clone(), cx)
                }
                Block::Markdown { source } => {
                    self.markdown_row(source.clone(), index, prefs.clone(), cx)
                }
                _ => self.markdown_row(block.source_range(), index, prefs.clone(), cx),
            };
            self.rows.put(index, row);
        }
        let pending = if self
            .pending_reveal
            .as_ref()
            .is_some_and(|(row, _)| self.display_rows[self.display_index(*row)] == index)
        {
            self.pending_reveal.take()
        } else {
            None
        };
        let row = self
            .rows
            .get(&index)
            .expect("visible row was inserted in the bounded cache");
        let element = match row {
            Row::Code(view) => {
                if let Some((_, range)) = pending {
                    view.update(cx, |view, cx| view.reveal(range, cx));
                }
                view.clone().into_any_element()
            }
            Row::Markdown(view) => {
                if let Some((_, range)) = pending {
                    let base = markdown_base(&self.document.blocks()[index]);
                    view.update(cx, |view, cx| {
                        view.pending_reveal =
                            Some(range.start.saturating_sub(base)..range.end.saturating_sub(base));
                        view.apply_highlights(cx);
                    });
                }
                view.clone().into_any_element()
            }
            Row::Mermaid(view) => view.clone().into_any_element(),
            Row::Source(view) => {
                if let Some((_, range)) = pending {
                    view.update(cx, |view, cx| view.reveal(range, window, cx));
                }
                view.clone().into_any_element()
            }
        };
        let callout = matches!(self.document.blocks()[index], Block::Callout { .. });
        let label = if let Block::Callout { source, body, .. } = &self.document.blocks()[index] {
            callout_label(&self.document.source()[source.start..body.start])
        } else {
            ""
        };
        let label = (!label.is_empty()).then(|| label.to_owned());
        let element = if callout {
            let lead = match row {
                Row::Markdown(view) if label.is_none() => Some(view.read(cx).first_ink.clone()),
                _ => None,
            };
            crate::notes::frame(
                crate::notes::NoteStyle {
                    icon: crate::notes::icon(None),
                    title: label,
                    fill: palette.highlight,
                    edge: palette.highlight_edge,
                    mark: palette.highlight_mark,
                    ink: palette.ink,
                    lead,
                },
                element,
                cx,
            )
            .into_any_element()
        } else {
            element
        };
        div()
            .debug_selector(move || format!("document-row-{index}"))
            .w_full()
            .flex()
            .justify_center()
            .px_8()
            .child(
                div()
                    .w_full()
                    .max_w(px(prefs.reading_width as f32 + 64.))
                    .px_8()
                    .bg(palette.paper)
                    .when(
                        !matches!(self.document.blocks()[index], Block::CodePage { .. }),
                        |row| row.py_2(),
                    )
                    .child(element),
            )
            .into_any_element()
    }
}

/// Byte where the Markdown actually given to the row begins.
fn markdown_base(block: &Block) -> usize {
    match block {
        Block::Callout { body, .. } => body.start,
        block => block.source_range().start,
    }
}

fn callout_label(opening: &str) -> &str {
    opening.trim().strip_prefix(":::").unwrap_or("").trim()
}

/// Matches inside one Markdown block, relative to the text that block renders.
fn block_highlights(
    document: &Document,
    matches: &[SearchMatch],
    index: usize,
) -> Vec<Range<usize>> {
    let block = &document.blocks()[index];
    let base = markdown_base(block);
    let end = match block {
        Block::Callout { body, .. } => body.end,
        block => block.source_range().end,
    };
    matches
        .iter()
        .filter(|found| found.block_index() == index)
        .filter_map(|found| {
            let range = found.source_range();
            (range.start >= base && range.end <= end)
                .then_some(range.start - base..range.end - base)
        })
        .collect()
}

impl Render for DocumentView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        let content = list(
            self.scroll.clone(),
            cx.processor(|view, index: usize, window, cx| view.row(index, window, cx)),
        )
        .size_full()
        .bg(ReadingPalette::for_app(cx).canvas)
        .py_5();
        div()
            .relative()
            .size_full()
            .child(content)
            .child(crate::viewer::measure_available(
                self.available,
                move |available, cx| {
                    let _ = weak.update(cx, |view, cx| {
                        if view.available != available {
                            view.available = available;
                            view.scroll.remeasure_items(
                                view.display_rows.len()..view.display_rows.len() + 1,
                            );
                            cx.notify();
                        }
                    });
                },
            ))
    }
}

#[derive(Clone, Copy)]
enum MarkdownLayout {
    Prose,
    Note,
}

struct MarkdownChunk {
    first_ink: crate::notes::LeadInk,
    layout: MarkdownLayout,
    text: Entity<TextViewState>,
    loading: bool,
    directory: Arc<Path>,
    ranges: Vec<Range<usize>>,
    needs_highlight: bool,
    pending_reveal: Option<Range<usize>>,
    _subscription: Subscription,
    extensions: MarkdownExtensions,
    math_pending: bool,
    _math_job: Option<Task<()>>,
}

struct MarkdownPreparation {
    queue: MathQueue,
    prefs: Arc<PreferencesData>,
    formats: Option<Entity<CodeFormats>>,
}

impl MarkdownChunk {
    fn new(
        source: String,
        math_pending: bool,
        directory: Arc<Path>,
        ranges: Vec<Range<usize>>,
        preparation: MarkdownPreparation,
        cx: &mut Context<Self>,
    ) -> Self {
        let MarkdownPreparation {
            queue,
            prefs,
            formats,
        } = preparation;
        let extensions = if math_pending {
            MarkdownExtensions::default()
        } else {
            crate::notes::extensions(crate::tables::extensions(crate::media::extensions(
                crate::code::extensions(MarkdownExtensions::default(), formats.clone()),
                &source,
                &directory,
            )))
        };
        let text = cx.new(|cx| {
            TextViewState::markdown(if math_pending { "" } else { &source }, cx)
                .selectable(true)
                .selection_format(SelectionFormat::Plain)
        });
        let loading =
            math_pending || !source.is_empty() && text.read(cx).rendered_text().source().is_empty();
        let subscription = cx.observe(&text, |view, _, cx| {
            view.apply_highlights(cx);
            if view.loading
                && !view.math_pending
                && !view.text.read(cx).rendered_text().source().is_empty()
            {
                view.loading = false;
                cx.notify();
            }
        });
        let math_job = math_pending.then(|| {
            let dark = cx.theme().is_dark();
            let worker_source = source.clone();
            let worker_directory = directory.clone();
            let background = cx.background_spawn(async move {
                let _permit = queue.rendering.slots.acquire().await;
                crate::notes::extensions(crate::tables::extensions(crate::media::extensions(
                    crate::code::extensions(queue.prepare(&worker_source, &prefs, dark), formats),
                    &worker_source,
                    &worker_directory,
                )))
            });
            cx.spawn(async move |weak, cx| {
                let extensions = background.await;
                let _ = weak.update(cx, |view, cx| {
                    view.extensions = extensions;
                    view.math_pending = false;
                    view.text.update(cx, |text, cx| text.set_text(&source, cx));
                    view.loading = view.text.read(cx).rendered_text().source().is_empty();
                    cx.notify();
                });
            })
        });
        Self {
            first_ink: Default::default(),
            layout: MarkdownLayout::Prose,
            text,
            loading,
            directory,
            ranges,
            needs_highlight: true,
            pending_reveal: None,
            _subscription: subscription,
            extensions,
            math_pending,
            _math_job: math_job,
        }
    }
    fn highlight(&mut self, ranges: Vec<Range<usize>>, cx: &mut Context<Self>) {
        self.ranges = ranges;
        self.needs_highlight = true;
        self.apply_highlights(cx);
        cx.notify();
    }
    fn apply_highlights(&mut self, cx: &mut Context<Self>) {
        if self.text.read(cx).rendered_text().is_empty() {
            return;
        }
        if self.needs_highlight {
            self.needs_highlight = false;
            let rendered = self.text.read(cx).rendered_text();
            let highlights: Vec<_> = self
                .ranges
                .iter()
                .filter_map(|range| rendered.range_for_source(range.clone()))
                .map(|range| RangeHighlight::new(range, cx.theme().accent.opacity(0.8)))
                .collect();
            self.text.update(cx, |text, cx| {
                let _ = text.set_range_highlights(highlights, cx);
            });
        }
        if let Some(source_range) = self.pending_reveal.take()
            && let Some(range) = self
                .text
                .read(cx)
                .rendered_text()
                .range_for_source(source_range)
        {
            self.text.update(cx, |text, cx| {
                let _ = text.reveal_range(range, cx);
            });
        }
    }
}

impl Render for MarkdownChunk {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let directory = self.directory.clone();
        let first_ink = self.first_ink.clone();
        let styles = crate::presentation::styles(cx);
        let style = match self.layout {
            MarkdownLayout::Prose => styles.reading,
            MarkdownLayout::Note => styles.note,
        };
        let body = styles.body;
        let palette = ReadingPalette::for_app(cx);
        div()
            .refine_style(&body)
            .text_color(palette.ink)
            .w_full()
            // Without a placeholder, empty asynchronous rows can fill the viewport,
            // evict one another from the LRU and restart parsing on every frame.
            .when(self.loading, |this| this.min_h(px(UNPARSED_ROW_HEIGHT)))
            .child(
                div().w_full().child(
                    TextView::new(&self.text)
                        .when(matches!(self.layout, MarkdownLayout::Note), |text| {
                            text.on_first_line_ink(move |ink, _, _| first_ink.set(ink))
                        })
                        .markdown_extensions(self.extensions.clone())
                        .style(style)
                        .selectable(true)
                        .selection_format(SelectionFormat::Plain)
                        .image_source(move |uri| {
                            let raw = uri.as_ref();
                            #[cfg(windows)]
                            if std::path::Path::new(raw).is_absolute() {
                                return crate::media::permitted_path(
                                    &directory,
                                    PathBuf::from(raw),
                                )
                                .map_or_else(|_| crate::media::blocked_image(), ImageSource::from);
                            }
                            if url::Url::parse(raw).is_ok() {
                                return ImageSource::from(uri.clone());
                            }
                            let path = url::Url::from_directory_path(&directory)
                                .ok()
                                .and_then(|base| base.join(raw).ok())
                                .and_then(|url| url.to_file_path().ok())
                                .unwrap_or_else(|| directory.join(raw));
                            crate::media::permitted_path(&directory, path)
                                .map_or_else(|_| crate::media::blocked_image(), ImageSource::from)
                        }),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Bounds, TestAppContext, WindowBounds, WindowOptions, size};

    #[gpui_kit::test]
    fn loading_prepares_hidden_code_and_reuses_results_across_rows_and_fonts(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            Preferences::shared(cx).update(cx, |prefs, _| {
                Arc::make_mut(&mut prefs.data).code_tab_size = "2".into()
            })
        });
        let body = "{\"items\":[{\"tail\":42}]}";
        let source = format!(
            "# Intro\n\n{}\n```json\n{body}\n```\n",
            "prose\n\n".repeat(200)
        );
        let document = Arc::new(Document::parse(source.clone()).unwrap());
        let block = document
            .blocks()
            .iter()
            .position(|block| matches!(block, Block::CodePage { .. }))
            .unwrap();
        let at = source.find("tail").unwrap();
        let (root, visual) = cx.add_window_view(|_, cx| {
            DocumentView::new(
                document,
                PathBuf::new(),
                RenderQueue::new(),
                MathQueue::new(),
                cx,
            )
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            let view = root.read(cx);
            assert!(
                !view.rows.contains(&block),
                "hidden code is not materialized"
            );
            let prepared = view
                .formats
                .read(cx)
                .get("json", body)
                .expect("hidden fence formatted on load");
            assert!(prepared.text().contains("\n  \"items\""));
            let state = view.formats.clone();
            root.update(cx, |view, cx| {
                view.reset_presentation(cx);
                view.suspend(cx);
                view.reveal(block, at..at + 4, cx);
            });
            assert_eq!(
                root.read(cx).formats.entity_id(),
                state.entity_id(),
                "font changes and suspension reuse the batch"
            );
        });
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(draw);
        }
        visual.update(|_, cx| {
            let view = root.read(cx);
            let Some(Row::Code(card)) = view.rows.peek(&block) else {
                panic!("code revealed");
            };
            let editor = card.read(cx).editor_state_for_test().read(cx);
            assert_eq!(
                editor.text().slice(editor.selected_range()).to_string(),
                "tail"
            );
            let formatted = view.formats.read(cx).get("json", body).unwrap();
            let row = formatted.text()[..formatted
                .range(at - source.find(body).unwrap()..at - source.find(body).unwrap() + 4)
                .start]
                .bytes()
                .filter(|&b| b == b'\n')
                .count();
            assert!(
                (view.scroll.logical_scroll_top().offset_in_item
                    - (crate::code::row_offset_y(row, 14.) - px(16.)))
                .abs()
                    < px(1.)
            );
            assert_eq!(view.document.source(), source);
        });
    }

    #[gpui_kit::test]
    fn fenced_notes_keep_a_single_rich_column_at_different_widths(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for body in [
            "`location_time` 取 SDK 返回的 `rtc_time` 字段值",
            "**参考文档**\n\n- 地理定位：[查看文档](https://example.com/location)\n- 反向地理编码：保留列表",
            "**更多 Markdown 结构**\n\n| 内容 | 示例 |\n| --- | --- |\n| 行内代码 | `request_id` |\n| 强调 | **重要结论** |\n\n```json\n{\n  \"reading\": true\n}\n```",
        ] {
            let document = Arc::new(Document::parse(format!(":::\n{body}\n:::\n")).unwrap());
            assert!(matches!(document.blocks()[0], Block::Callout { .. }));
            let (root, visual) = cx.add_window_view(|_, cx| {
                DocumentView::new(
                    document,
                    PathBuf::new(),
                    RenderQueue::new(),
                    MathQueue::new(),
                    cx,
                )
            });
            for width in [420., 960.] {
                visual.simulate_resize(size(px(width), px(700.)));
                for _ in 0..4 {
                    visual.run_until_parked();
                    visual.update(draw);
                }
                let icon = visual.debug_bounds("pecan-note-icon").expect("fence icon");
                let column = visual.debug_bounds("pecan-note-body").expect("rich body");
                assert!((column.left() - icon.right() - px(8.)).abs() <= px(0.5));
                assert!(icon.center().y > column.top() && icon.center().y < column.top() + px(29.));
                assert!(visual.debug_bounds("pecan-note-title").is_none());
                let note = visual.debug_bounds("pecan-note").expect("fence frame");
                assert!(
                    column.right() < note.right(),
                    "body keeps right padding at {width}px"
                );
                visual.update(|_, cx| {
                    let view = root.read(cx);
                    let Some(Row::Markdown(chunk)) = view.rows.peek(&0) else {
                        panic!("fenced Markdown must remain a selectable Markdown row");
                    };
                    let rendered = chunk.read(cx).text.read(cx).rendered_text();
                    assert!(rendered.source().starts_with(body));
                    for word in ["location_time", "rtc_time", "参考文档", "地理定位"] {
                        if let Some(start) = body.find(word) {
                            assert!(rendered.range_for_source(start..start + word.len()).is_some(),
                                "inline code, strong text and list links keep search mapping: {word}");
                        }
                    }
                    assert!(view.resident_rows() <= 32);
                });
            }
        }
    }

    #[gpui_kit::test]
    fn nested_code_keeps_equal_frame_padding_when_resized_and_folded(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let body = "**更多 Markdown 结构**\n\n| 内容 | 示例 |\n| --- | --- |\n| 行内代码 | `request_id` |\n\n```json\n{\n  \"reading\": true,\n  \"tab_size\": 4\n}\n```";
        for source in [
            format!(":::\n{body}\n:::\n"),
            format!(
                "> [!NOTE] 有标题的说明\n>\n{}\n",
                body.lines()
                    .map(|line| format!("> {line}\n"))
                    .collect::<String>()
            ),
        ] {
            let document = Arc::new(Document::parse(source).unwrap());
            let (_, visual) = cx.add_window_view(|_, cx| {
                DocumentView::new(
                    document,
                    PathBuf::new(),
                    RenderQueue::new(),
                    MathQueue::new(),
                    cx,
                )
            });
            for mode in [
                gpui_kit::component::ThemeMode::Light,
                gpui_kit::component::ThemeMode::Dark,
            ] {
                visual.update(|_, cx| gpui_kit::component::Theme::change(mode, None, cx));
                for width in [420., 960.] {
                    visual.simulate_resize(size(px(width), px(700.)));
                    for _ in 0..2 {
                        for _ in 0..4 {
                            visual.run_until_parked();
                            visual.update(draw);
                        }
                        let note = visual.debug_bounds("pecan-note").unwrap();
                        let heading = visual
                            .debug_bounds("pecan-note-heading")
                            .or_else(|| visual.debug_bounds("pecan-note-body-frame"))
                            .expect("the leading title or body remains inside the note padding");
                        let card = visual.debug_bounds("code-frame").unwrap();
                        let right = note.right() - card.right();
                        let bottom = note.bottom() - card.bottom();
                        let top = heading.top() - note.top();
                        let left = heading.left() - note.left();
                        for (side, padding) in [("bottom", bottom), ("top", top), ("left", left)] {
                            assert!(
                                (right - padding).abs() <= px(0.5),
                                "{side} padding {padding:?} differs from right {right:?} at {width}px"
                            );
                        }
                        assert!(right > px(0.) && card.left() > note.left());
                        let at = visual.debug_bounds("code-collapse").unwrap().center();
                        visual.simulate_click(at, Default::default());
                    }
                }
            }
        }
    }

    #[gpui_kit::test]
    fn fenced_long_links_keep_chinese_labels_at_natural_width(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = r#":::
**参考文档**
- 地理定位：[https://developers.google.com/maps/documentation/geolocation/requests-geolocation?hl=zh-cn](https://developers.google.com/maps/documentation/geolocation/requests-geolocation?hl=zh-cn)
- 反向地理编码：[https://developers.google.com/maps/documentation/geocoding/reverse-geocoding?hl=zh-cn](https://developers.google.com/maps/documentation/geocoding/reverse-geocoding?hl=zh-cn)
:::
"#;
        let document = Arc::new(Document::parse(source.into()).unwrap());
        let (root, visual) = cx.add_window_view(|_, cx| {
            DocumentView::new(
                document,
                PathBuf::new(),
                RenderQueue::new(),
                MathQueue::new(),
                cx,
            )
        });
        for width in [700., 420.] {
            visual.simulate_resize(size(px(width), px(700.)));
            for _ in 0..4 {
                visual.run_until_parked();
                visual.update(draw);
            }
            let body = visual.debug_bounds("pecan-note-body").unwrap();
            visual.update(|window, cx| {
                let first = window
                    .painted_underlines()
                    .into_iter()
                    .min_by(|a, b| a.bounds.top().partial_cmp(&b.bounds.top()).unwrap())
                    .expect("the first URL has a painted link underline");
                let prefix = first.bounds.left().0 / window.scale_factor() - f32::from(body.left());
                let font_size = Preferences::shared(cx).read(cx).data.font_size as f32;
                assert!(
                    prefix <= font_size * 7.,
                    "Chinese label and marker stretch to {prefix}px at {width}px"
                );
                let Some(Row::Markdown(chunk)) = root.read(cx).rows.peek(&0) else {
                    panic!("note row")
                };
                assert!(matches!(chunk.read(cx).layout, MarkdownLayout::Note));
                chunk.read(cx).text.clone().update(cx, |text, cx| {
                    text.select_all(cx);
                    assert!(text.selected_text().contains("地理定位"));
                    assert!(
                        text.selected_text()
                            .contains("requests-geolocation?hl=zh-cn")
                    );
                });
                assert_eq!(root.read(cx).document.source(), source);
            });
        }
    }

    #[gpui_kit::test]
    fn nested_fences_use_native_cards_and_preserve_interaction_state(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let code = "{\"items\":[1,{\"tail\":2}]}";
        for source in [
            format!(":::\n**嵌套代码**\n\n```json\n{code}\n```\n:::\n"),
            format!("> [!NOTE]\n> 嵌套代码\n>\n> ```json\n> {code}\n> ```\n"),
        ] {
            let document = Arc::new(Document::parse(source.clone()).unwrap());
            let (root, visual) = cx.add_window_view(|_, cx| {
                DocumentView::new(
                    document,
                    PathBuf::new(),
                    RenderQueue::new(),
                    MathQueue::new(),
                    cx,
                )
            });
            for _ in 0..4 {
                visual.run_until_parked();
                visual.update(draw);
            }
            let header = visual
                .debug_bounds("code-header")
                .expect("shared language toolbar");
            let copy = visual
                .debug_bounds("code-copy")
                .expect("shared copy control");
            assert!(copy.right() <= header.right() && copy.top() >= header.top());
            let at = copy.center();
            visual.simulate_click(at, Default::default());
            visual.update(|_, cx| {
                let copied = cx.read_from_clipboard().unwrap().text().unwrap();
                assert!(
                    copied.contains('\n'),
                    "nested fences are formatted on document load"
                );
            });
            let before = visual.debug_bounds("code-viewport").unwrap().size.height;
            assert!(before > px(100.));
            assert!(visual.debug_bounds("code-format").is_none());
            let at = visual.debug_bounds("code-copy").unwrap().center();
            visual.simulate_click(at, Default::default());
            visual.update(|_, cx| {
                let copied = cx.read_from_clipboard().unwrap().text().unwrap();
                assert!(copied.contains('\n'));
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&copied).unwrap(),
                    serde_json::from_str::<serde_json::Value>(code).unwrap()
                );
                assert_eq!(root.read(cx).document.source(), source);
            });
            let at = visual.debug_bounds("code-collapse").unwrap().center();
            visual.simulate_click(at, Default::default());
            for width in [420., 960.] {
                visual.simulate_resize(size(px(width), px(700.)));
                visual.update(draw);
                assert!(
                    visual.debug_bounds("code-viewport").is_none(),
                    "fold state survives redraw and resize"
                );
            }
            let at = visual.debug_bounds("code-collapse").unwrap().center();
            visual.simulate_click(at, Default::default());
            visual.update(draw);
            assert!(
                visual.debug_bounds("code-viewport").unwrap().size.height == before,
                "formatted editor is reused"
            );
        }
    }

    #[gpui_kit::test]
    fn formula_rows_finish_background_work_and_keep_source_ranges(cx: &mut TestAppContext) {
        let source = "中文 $E=mc^2$ English\n\n$$\n\\ce{N2 + 3H2 <=> 2NH3}\n$$\n";
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| {
                    MarkdownChunk::new(
                        source.into(),
                        true,
                        Arc::from(Path::new("")),
                        Vec::new(),
                        MarkdownPreparation {
                            queue: MathQueue::new(),
                            prefs: Arc::default(),
                            formats: None,
                        },
                        cx,
                    )
                })
            })
            .unwrap()
        });
        for _ in 0..4 {
            for tick in 0..10_000 {
                if !cx.executor().tick() {
                    break;
                }
                assert!(tick < 9999, "formula preparation did not settle");
            }
            cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        }
        cx.update(|cx| {
            let view = view.read(cx);
            assert!(
                !view.math_pending && !view.loading,
                "formula row must leave its loading placeholder"
            );
            let rendered = view.text.read(cx).rendered_text();
            assert_eq!(rendered.source(), source);
            let start = source.find("E=mc^2").unwrap();
            assert!(
                rendered.range_for_source(start..start + 6).is_some(),
                "inline formulas remain searchable"
            );
            assert!(rendered.as_str().contains("中文") && rendered.as_str().contains("N2"));
        });
    }
    #[gpui_kit::test]
    fn dollar_in_shared_reference_definition_does_not_defer_prose(cx: &mut TestAppContext) {
        let source =
            "普通段落 [文档][api]。\n\n## 附录\n\n[api]: https://example.com/items?$filter=open\n";
        let document = Arc::new(Document::parse(source.into()).unwrap());
        assert!(document.reference_suffix().contains('$'));
        assert!(!document.source()[document.blocks()[0].source_range()].contains('$'));
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| {
                    DocumentView::new(
                        document,
                        PathBuf::new(),
                        RenderQueue::new(),
                        MathQueue::new(),
                        cx,
                    )
                })
            })
            .unwrap()
        });
        let chunk = |view: &DocumentView, cx: &gpui_kit::App| match view.rows.peek(&0) {
            Some(Row::Markdown(chunk)) => chunk.read(cx).math_pending,
            _ => panic!("the paragraph row is materialized"),
        };
        cx.update_window(handle, |_, window, cx| {
            draw(window, cx);
            assert!(
                !chunk(view.read(cx), cx),
                "prose is not deferred to the formula queue"
            );
        })
        .unwrap();
        settle(cx, "reference row");
        cx.update_window(handle, |_, window, cx| {
            draw(window, cx);
            let view = view.read(cx);
            assert!(view.rows.iter().any(|(_, row)| matches!(row,
                Row::Markdown(chunk) if chunk.read(cx).text.read(cx).rendered_text().as_str().contains("普通段落"))));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    async fn large_document_renders_visible_rows_and_reveals_tail(cx: &mut TestAppContext) {
        let paragraph = "## 阅读章节\n\n中文长文档 **强调** 与 `code`。\n\n";
        let mut source = paragraph.repeat(10 * 1024 * 1024 / paragraph.len());
        source.push_str("## 最后一节\n\n末尾内容验证。\n");
        let document = Arc::new(Document::parse(source).unwrap());
        let tail = document.headings().last().unwrap().clone();
        let count = document.blocks().len();
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        gpui_kit::Point::default(),
                        size(px(900.), px(700.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|cx| {
                        DocumentView::new(
                            document,
                            PathBuf::new(),
                            RenderQueue::new(),
                            MathQueue::new(),
                            cx,
                        )
                    })
                },
            )
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| draw(window, cx))
            .unwrap();
        cx.update(|cx| {
            let rows = view.read(cx).resident_rows();
            assert!(
                rows > 0 && rows < count / 10,
                "first frame must materialize a small visible subset: {rows}/{count}"
            );
        });
        settle(cx, "initial render");
        for index in (0..count).step_by((count / 48).max(1)) {
            cx.update_window(handle, |_, window, cx| {
                view.update(cx, |view, cx| view.reveal(index, 0..0, cx));
                draw(window, cx);
            })
            .unwrap();
            settle(cx, "document jump");
        }
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.reveal(tail.block_index(), tail.source_range(), cx)
            });
            draw(window, cx);
        })
        .unwrap();
        settle(cx, "tail reveal");
        cx.update_window(handle, |_, window, cx| {
            draw(window, cx);
            let view = view.read(cx);
            assert!(
                view.rows.iter().any(|(_, row)| match row {
                    Row::Markdown(chunk) => chunk
                        .read(cx)
                        .text
                        .read(cx)
                        .rendered_text()
                        .as_str()
                        .contains("最后一节"),
                    _ => false,
                }),
                "the end of the document must be rendered after a direct jump"
            );
        })
        .unwrap();
        cx.update(|cx| {
            assert!(
                view.read(cx).resident_rows() <= ROW_CACHE_ENTRIES,
                "jumping through a long document must evict old UI rows"
            );
        });
    }

    #[gpui_kit::test]
    fn expanded_code_search_scrolls_the_document_and_keeps_tail_visible(cx: &mut TestAppContext) {
        let source = format!(
            "## Before\n\n```json\n{}tail_token\n```\n\n## After\n",
            "{\"value\":42}\n".repeat(5000)
        );
        let document = Arc::new(Document::parse(source).unwrap());
        let matches = pecan_document::SearchQuery::new("tail_token")
            .unwrap()
            .find(&document);
        let found = matches[0].clone();
        let tail = document.headings().last().unwrap().clone();
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| {
                    DocumentView::new(
                        document,
                        PathBuf::new(),
                        RenderQueue::new(),
                        MathQueue::new(),
                        cx,
                    )
                })
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                assert_eq!(
                    view.display_rows.len(),
                    3,
                    "a multi-page source fence has exactly one displayed row"
                );
                assert!(found.block_index() > view.display_rows[1]);
                assert_eq!(view.display_index(found.block_index()), 1);
                view.search(matches.into(), cx);
                view.reveal(found.block_index(), found.source_range(), cx);
            });
            draw(window, cx);
            draw(window, cx);
            let view = view.read(cx);
            assert!(
                view.pending_reveal.is_none(),
                "source continuation pages reveal in the first viewport"
            );
            assert_eq!(
                view.rows
                    .iter()
                    .filter(|(_, row)| matches!(row, Row::Code(_)))
                    .count(),
                1
            );
        })
        .unwrap();
        settle(cx, "code search");
        for (jump, row) in [5000, 2500, 5000].into_iter().enumerate() {
            cx.update_window(handle, |_, window, cx| {
                let (source_index, code, range) = {
                    let view = view.read(cx);
                    let source_index = view.display_rows[1];
                    let Block::CodePage { code, .. } = &view.document.blocks()[source_index] else {
                        panic!("display row must represent the full code fence");
                    };
                    let start = code.start + row * "{\"value\":42}\n".len();
                    let range = if row == 5000 { found.source_range() } else { start..start + 2 };
                    (source_index, code.clone(), range)
                };
                if jump > 0 {
                    let block = if row == 5000 { found.block_index() } else { source_index };
                    view.update(cx, |view, cx| view.reveal(block, range.clone(), cx));
                }
                draw(window, cx);
                draw(window, cx);
                let code_size = Preferences::shared(cx).read(cx).data.code_size;
                let view = view.read(cx);
                let scroll_top = view.scroll.logical_scroll_top();
                let expected_top = crate::code::row_offset_y(row, code_size) - px(16.);
                assert_eq!(scroll_top.item_ix, 1, "search stays in the single expanded code row");
                assert!((scroll_top.offset_in_item - expected_top).abs() < px(1.),
                    "outer list reaches row {row}: actual={scroll_top:?}, expected={expected_top:?}");
                let Some(Row::Code(card)) = view.rows.peek(&source_index) else {
                    panic!("expanded code card must be materialized after search");
                };
                let editor = card.read(cx).editor_state_for_test().read(cx);
                let visible = editor.visible_row_range().unwrap();
                assert!(visible.contains(&row) && visible.len() < 80,
                    "target row {row} must be visible while only viewport rows are shaped: {visible:?}");
                assert_eq!(editor.scroll_offset().y, px(0.), "code has no vertical scrolling");
                let selected = editor.selected_range();
                assert_eq!(selected, range.start - code.start..range.end - code.start,
                    "search selects the text after the document performs the scroll");
                let bounds = editor.range_to_bounds(&selected).unwrap();
                assert!(bounds.top() >= px(0.) && bounds.bottom() < window.viewport_size().height,
                    "selected code text is inside the document viewport: {bounds:?}");
                assert!(bounds.top() < px(80.), "search keeps the matched line near the top: {bounds:?}");
            }).unwrap();
            settle(cx, "expanded code search position");
        }
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.reveal_heading(1, cx));
            draw(window, cx);
            draw(window, cx);
            let view = view.read(cx);
            assert_eq!(view.display_index(tail.block_index()), 2);
            assert!(
                view.rows.contains(&tail.block_index()),
                "heading after code remains reachable"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn next_row_never_covers_code_bottom_when_code_straddles_window_top(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = format!(
            "## Before\n\n```json\n{}```\n\n## After\n\n代码块下面的纸页背景。\n",
            "{\"value\":42}\n".repeat(60)
        );
        let document = Arc::new(Document::parse(source).unwrap());
        let code_block = document
            .blocks()
            .iter()
            .position(|block| matches!(block, Block::CodePage { .. }))
            .unwrap();
        let (root, visual) = cx.add_window_view(|_, cx| {
            DocumentView::new(
                document,
                PathBuf::new(),
                RenderQueue::new(),
                MathQueue::new(),
                cx,
            )
        });
        visual.simulate_resize(size(px(960.), px(700.)));
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(draw);
        }
        let (item, height) = visual.update(|window, cx| {
            let item = root.read(cx).display_index(code_block);
            root.update(cx, |view, _| {
                view.scroll.scroll_to(ListOffset {
                    item_ix: item,
                    offset_in_item: px(0.),
                })
            });
            draw(window, cx);
            draw(window, cx);
            let height = root
                .read(cx)
                .scroll
                .bounds_for_item(item)
                .unwrap()
                .size
                .height;
            (item, height)
        });
        // Eighth-pixel steps reach a device-pixel midpoint at 1x and 2x.
        for step in 0..8 {
            visual.update(|window, cx| {
                root.update(cx, |view, _| {
                    view.scroll.scroll_to(ListOffset {
                        item_ix: item,
                        offset_in_item: height - px(300.) + px(step as f32 * 0.125),
                    })
                });
                draw(window, cx);
                draw(window, cx);
                let palette = ReadingPalette::for_app(cx);
                let scale = window.scale_factor();
                let quads = window.painted_quads();
                let frame = quads
                    .iter()
                    .find(|quad| {
                        quad.border_color == palette.border
                            && quad.border_widths.bottom.0 > 0.
                            && quad.border_widths.left.0 > 0.
                            && quad.bounds.size.width.0 > 400. * scale
                            && quad.bounds.size.height.0 > 100. * scale
                    })
                    .expect("code frame is painted")
                    .bounds;
                assert!(
                    frame.top().0 < 0. && frame.bottom().0 > 0.,
                    "fixture straddles the window origin: {frame:?}"
                );
                let paper = palette.paper.as_solid();
                let next = quads
                    .iter()
                    .find(|quad| {
                        quad.background.as_solid() == paper
                            && quad.bounds.size.width.0 > 400. * scale
                            && (quad.bounds.top().0 - frame.bottom().0).abs() <= 2. * scale
                    })
                    .expect("the row after the code is painted");
                assert!(
                    next.bounds.top() >= frame.bottom(),
                    "step {step}: next row {:?} covers the code bottom border {frame:?}",
                    next.bounds
                );
            });
        }
    }

    /// Replays every native quad from the production DocumentView into the real
    /// GPU renderer. Text sprites are intentionally omitted: this checks native
    /// frame/background ordering and clip masks, not glyph rendering.
    #[gpui_kit::test]
    #[cfg(target_os = "macos")]
    #[ignore = "Needs a Metal device; launch this test binary through LaunchServices."]
    fn document_code_bottom_survives_list_scrolling_in_real_metal_pixels(cx: &mut TestAppContext) {
        use gpui_kit::{DevicePixels, Scene};

        cx.update(gpui_kit::init);
        let mut renderer = gpui_platform::current_headless_renderer()
            .expect("Metal renderer factory")
            .expect("macOS provides a Metal renderer");

        for theme in ["light", "dark"] {
            cx.update(|cx| {
                Preferences::shared(cx).update(cx, |prefs, cx| {
                    prefs.change(|data| data.theme = theme.to_owned(), cx);
                });
                cx.set_reduce_motion(true);
            });

            for rows in [90, 5000] {
                // The first line also exercises the native horizontal-scroll
                // geometry; no text is wrapped and the fence is fully expanded.
                let source = format!(
                    "## Before\n\n正文滚动与原生代码边框。\n\n```json\n{{\n  \"long\": \"{}\",\n{}}}\n```\n\n## After\n\n代码块下面的纸页背景。\n",
                    "long_value_".repeat(120),
                    "  \"setting\": {{\"enable\":true,\"remark\":\"设备配置与性能检查\"}},\n"
                        .repeat(rows),
                );
                let document = Arc::new(Document::parse(source).unwrap());
                let code_block = document
                    .blocks()
                    .iter()
                    .position(|block| matches!(block, Block::CodePage { .. }))
                    .unwrap();
                let (root, visual) = cx.add_window_view(|_, cx| {
                    DocumentView::new(
                        document,
                        PathBuf::new(),
                        RenderQueue::new(),
                        MathQueue::new(),
                        cx,
                    )
                });
                visual.simulate_resize(size(px(960.), px(700.)));

                // Settle async Markdown before testing frame reuse. Do not use
                // the shared draw() helper: it calls refresh and masks stale
                // cached-frame bugs.
                for _ in 0..4 {
                    visual.run_until_parked();
                    visual.update(|window, cx| window.draw(cx).clear(cx));
                }
                let (code_item, code_height) = visual.update(|window, cx| {
                    let item = root.read(cx).display_index(code_block);
                    root.update(cx, |view, cx| {
                        view.scroll.scroll_to(ListOffset {
                            item_ix: item,
                            offset_in_item: px(0.),
                        });
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                    window.draw(cx).clear(cx);
                    let height = root
                        .read(cx)
                        .scroll
                        .bounds_for_item(item)
                        .expect("the code row was measured")
                        .size
                        .height;
                    (item, height)
                });
                visual.run_until_parked();

                // First paint the middle, then reach the real lower edge by
                // outer-list scrolling. Reusing the same CodeCard is deliberate.
                visual.update(|window, cx| {
                    root.update(cx, |view, cx| {
                        view.scroll.scroll_to(ListOffset {
                            item_ix: code_item,
                            offset_in_item: code_height / 2.,
                        });
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });

                for (stage, relative, value) in [
                    ("tail", false, code_height - px(380.)),
                    ("down", true, px(240.)),
                    ("up", true, px(-240.)),
                    ("fractional", false, code_height - px(379.75)),
                    ("return", false, code_height - px(380.)),
                ] {
                    visual.update(|_, cx| {
                        root.update(cx, |view, cx| {
                            if relative {
                                view.scroll.scroll_by(value);
                            } else {
                                view.scroll.scroll_to(ListOffset {
                                    item_ix: code_item,
                                    offset_in_item: value,
                                });
                            }
                            cx.notify();
                        });
                    });

                    // Capture the dirty scroll frame, an idle cached frame, and
                    // forced refresh separately to localize any cache regression.
                    for pass in ["scroll", "idle", "refresh"] {
                        let (image, frame, border, code, paper, scale) =
                            visual.update(|window, cx| {
                                if pass == "refresh" {
                                    window.refresh();
                                }
                                window.draw(cx).clear(cx);
                                let palette = ReadingPalette::for_app(cx);
                                let quads = window.painted_quads();
                                let scale = window.scale_factor();
                                let frame = quads
                                    .iter()
                                    .find(|quad| {
                                        quad.border_color == palette.border
                                            && quad.border_widths.top.0 > 0.
                                            && quad.border_widths.bottom.0 > 0.
                                            && quad.border_widths.left.0 > 0.
                                            && quad.border_widths.right.0 > 0.
                                            && quad.bounds.size.width.0 > 400. * scale
                                            && quad.bounds.size.height.0 > 100. * scale
                                    })
                                    .expect("native CodeCard border is in the painted scene")
                                    .bounds;
                                let mut scene = Scene::default();
                                for quad in quads {
                                    scene.insert_primitive(quad);
                                }
                                scene.finish();
                                let image = renderer
                                    .render_scene_to_image(
                                        &scene,
                                        size(
                                            DevicePixels((960. * scale) as i32),
                                            DevicePixels((700. * scale) as i32),
                                        ),
                                    )
                                    .unwrap();
                                (
                                    image,
                                    frame,
                                    palette.border.to_rgb(),
                                    palette.code.to_rgb(),
                                    palette.paper.as_solid().unwrap().to_rgb(),
                                    scale,
                                )
                            });

                        // Save first, so failed assertions still leave evidence.
                        if let Ok(directory) = std::env::var("PECAN_METAL_OUTPUT") {
                            image
                                .save(
                                    std::path::Path::new(&directory).join(format!(
                                        "code-list-{theme}-{rows}-{stage}-{pass}.png"
                                    )),
                                )
                                .unwrap();
                        }

                        let bottom = frame.bottom().0;
                        assert!(
                            bottom > 30. * scale && bottom < 650. * scale,
                            "fixture must show the actual lower code edge: {frame:?}",
                        );
                        let channels = [border.r, border.g, border.b]
                            .map(|channel| (channel * 255.).round() as u8);
                        let first_x = (frame.left().0 + 12. * scale).ceil() as u32;
                        let last_x = (frame.right().0 - 12. * scale).floor() as u32;
                        let first_y = (bottom.floor() as i32 - 3).max(0) as u32;
                        let last_y = (bottom.ceil() as u32 + 2).min(image.height() - 1);
                        let mut covered = 0;
                        let mut sampled = 0;
                        for x in (first_x..last_x).step_by(scale.ceil() as usize) {
                            sampled += 1;
                            if (first_y..=last_y).any(|y| {
                                let pixel = image.get_pixel(x, y).0;
                                pixel[3] >= 250
                                    && pixel[..3]
                                        .iter()
                                        .zip(channels)
                                        .all(|(&actual, expected)| actual.abs_diff(expected) <= 4)
                            }) {
                                covered += 1;
                            }
                        }
                        assert!(
                            sampled > 200 && covered * 100 >= sampled * 98,
                            "{theme}, rows={rows}, {stage}/{pass}: native bottom border \
                             covers {covered}/{sampled} columns; frame={frame:?}, \
                             border={border:?}, code={code:?}, paper={paper:?}",
                        );
                    }
                }
            }
        }
    }

    fn draw(window: &mut Window, cx: &mut gpui_kit::App) {
        window.refresh();
        window.draw(cx).clear(cx);
    }

    fn settle(cx: &TestAppContext, stage: &str) {
        // Bound executor work so a notification feedback loop fails instead of hanging CI.
        for _ in 0..10_000 {
            if !cx.executor().tick() {
                return;
            }
        }
        panic!(
            "UI work did not settle after 10000 executor ticks during {stage}; queued={:?}",
            cx.dispatcher.scheduler().pending_task_counts()
        );
    }
}

/// Document links reach the OS opener through the vendored `TextView` policy.
#[cfg(test)]
mod link_policy_tests {
    use gpui_kit::base::text::is_web_link;

    #[test]
    fn only_web_and_mail_links_reach_the_system_opener() {
        for allowed in [
            "https://example.com/a?b#c",
            "HTTP://example.com",
            " https://example.com",
            "mailto:team@example.com",
        ] {
            assert!(is_web_link(allowed), "{allowed}");
        }
        for denied in [
            "file:///C:/Windows/System32/calc.exe",
            "evil.exe",
            "./run.sh",
            "C:\\Users\\Public\\x.exe",
            "\\\\attacker\\share\\x.exe",
            "smb://attacker/share",
            "ms-msdt:/id PCWDiagnostic",
            "search-ms:query=x",
            "javascript:alert(1)",
            "https:example.com",
            "https://",
            "https://example.com\n--flag",
            "#heading",
            "",
        ] {
            assert!(!is_web_link(denied), "{denied}");
        }
    }
}
