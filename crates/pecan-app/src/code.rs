//! Read-only native code with horizontal scrolling, folding and configuration formatting.
mod prepared;
use pecan_format as formatting;
pub(crate) use pecan_format::{CodeSnapshot, SLOTS};
pub(crate) use prepared::CodeFormats;
mod markdown;
use crate::{preferences::Preferences, presentation::ReadingPalette};
use gpui_kit::{
    ClipboardItem, Context, Entity, HighlightStyle, IntoElement, Render, SharedString,
    Subscription, Task, Window,
    base::input::{RopeExt, TabSize, TextDecoration, TextDecorationCollection},
    component::{
        IconName, StyledExt,
        input::{Editor, EditorState},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    div,
    prelude::*,
    px,
};
pub(crate) use markdown::extensions;
use pecan_document::Document;
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// Native editor padding, shared by short and scrollable fences.
const CODE_PADDING: f32 = 16.;

fn code_type(prefs: &crate::preferences::PreferencesData) -> gpui_kit::StyleRefinement {
    let mut style = gpui_kit::StyleRefinement::default();
    style.text.font_family = Some(crate::fonts::family(&prefs.code_font).to_owned().into());
    style.text.font_fallbacks = Some(crate::fonts::text_fallbacks(&prefs.chinese_font));
    style
}

/// Top-level fences borrow the document; nested Markdown already has a decoded body.
#[derive(Clone)]
enum CodeSource {
    Document(Arc<Document>),
    Fragment(SharedString),
}
impl CodeSource {
    fn as_str(&self) -> &str {
        match self {
            Self::Document(document) => document.source(),
            Self::Fragment(text) => text.as_ref(),
        }
    }
}

pub(crate) struct CodeCard {
    source: CodeSource,
    code: Range<usize>,
    language: String,
    language_icon: &'static crate::language_icons::LanguageIcon,
    formatter: Option<formatting::Format>,
    lines: usize,
    editor: Entity<EditorState>,
    highlights: TextDecorationCollection,
    matches: Vec<Range<usize>>,
    tab_width: usize,
    tab_override: Option<usize>,
    detected_width: usize,
    collapsed: bool,
    formatted: Option<Arc<formatting::FormattedCode>>,
    format_revision: Arc<AtomicU64>,
    revealed_source: Option<Range<usize>>,
    _format_subscription: Option<Subscription>,
    error: Option<String>,
    format_job: Option<Task<()>>,
    _editor_subscription: Subscription,
}
impl CodeCard {
    #[cfg(test)]
    pub(crate) fn new(
        document: Arc<Document>,
        code: Range<usize>,
        language: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::from_source(
            CodeSource::Document(document),
            code,
            language,
            None,
            window,
            cx,
        )
    }
    pub(crate) fn with_formats(
        document: Arc<Document>,
        code: Range<usize>,
        language: String,
        formats: Entity<CodeFormats>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::from_source(
            CodeSource::Document(document),
            code,
            language,
            Some(formats),
            window,
            cx,
        )
    }
    fn from_fragment(
        body: SharedString,
        language: String,
        formats: Option<Entity<CodeFormats>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let code = 0..body.len();
        Self::from_source(
            CodeSource::Fragment(body),
            code,
            language,
            formats,
            window,
            cx,
        )
    }
    fn from_source(
        source: CodeSource,
        code: Range<usize>,
        language: String,
        formats: Option<Entity<CodeFormats>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let raw = &source.as_str()[code.clone()];
        let display = display_source(&source, raw);
        let detected_width = formatting::tab_width(display, "auto");
        let formatted = formats
            .as_ref()
            .and_then(|formats| formats.read(cx).get(&language, display));
        let display = formatted.as_ref().map_or(display, |code| code.text());
        let lines = display.bytes().filter(|&byte| byte == b'\n').count() + 1;
        let preference = Preferences::shared(cx).read(cx).data.clone();
        let tab_override = preference
            .code_tab_size
            .parse::<usize>()
            .ok()
            .filter(|n| [2, 4, 8].contains(n));
        let tab_width = tab_override.unwrap_or(detected_width);
        let editor = cx.new(|cx| {
            let syntax_language = pecan_code::language(&language);
            EditorState::new(window, cx)
                .context_menu(false)
                .language(syntax_language)
                .line_number(false)
                .folding(true)
                .tab_size(TabSize {
                    tab_size: tab_width,
                    hard_tabs: false,
                })
                .soft_wrap(false)
                .scroll_beyond_last_line(Some(0))
                .default_value(display.to_owned())
        });
        let highlights = editor.update(cx, |editor, cx| {
            editor.create_decorations_collection(Vec::new(), cx)
        });
        let mut visible_rows = lines;
        let subscription = cx.observe(&editor, move |_, editor, cx| {
            let rows = editor.read(cx).display_map().display_row_count();
            if rows > 0 && rows != visible_rows {
                visible_rows = rows;
                cx.notify();
            }
        });
        let format_subscription = formats
            .as_ref()
            .filter(|formats| !formats.read(cx).ready())
            .map(|formats| {
                cx.observe_in(formats, window, |card, state, window, cx| {
                    card._format_subscription = None;
                    if card.format_revision.load(Ordering::Relaxed) == 0
                        && state.read(cx).ready()
                        && let Some(formatted) =
                            state.read(cx).get(&card.language, card.raw_display())
                    {
                        let display = formatted.text().to_owned();
                        card.formatted = Some(formatted);
                        card.replace_display(display, window, cx);
                    }
                })
            });
        let mut card = Self {
            source,
            code,
            language_icon: crate::language_icons::for_language(&language),
            formatter: formatting::Format::for_language(&language),
            language,
            lines,
            editor,
            highlights,
            matches: Vec::new(),
            tab_width,
            tab_override,
            detected_width,
            collapsed: false,
            formatted,
            format_revision: Arc::new(AtomicU64::new(0)),
            revealed_source: None,
            _format_subscription: format_subscription,
            error: None,
            format_job: None,
            _editor_subscription: subscription,
        };
        if formats.is_none() {
            card.request_format(window, cx, false);
        }
        card
    }
    #[cfg(test)]
    pub(crate) fn editor_state_for_test(&self) -> &Entity<EditorState> {
        &self.editor
    }
    fn raw_display(&self) -> &str {
        display_source(&self.source, &self.source.as_str()[self.code.clone()])
    }
    fn display_range(&self, range: Range<usize>, cx: &gpui_kit::App) -> Range<usize> {
        let local = range
            .start
            .max(self.code.start)
            .saturating_sub(self.code.start)
            ..range.end.min(self.code.end).saturating_sub(self.code.start);
        if local.start >= local.end {
            return 0..0;
        }
        if let Some(formatted) = &self.formatted {
            return formatted.range(local);
        }
        let len = self.editor.read(cx).text().len();
        local.start.min(len)..local.end.min(len)
    }
    pub(crate) fn highlight(
        &mut self,
        ranges: impl IntoIterator<Item = Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        self.matches = ranges
            .into_iter()
            .filter(|r| r.start < self.code.end && r.end > self.code.start)
            .collect();
        self.apply_highlights(cx);
    }
    fn apply_highlights(&self, cx: &mut Context<Self>) {
        let color = ReadingPalette::for_app(cx).wash;
        self.highlights.set(
            self.matches
                .iter()
                .filter_map(|range| {
                    let range = self.display_range(range.clone(), cx);
                    (!range.is_empty()).then(|| {
                        TextDecoration::new(
                            range,
                            HighlightStyle {
                                background_color: Some(color),
                                ..Default::default()
                            },
                        )
                    })
                })
                .collect(),
            cx,
        );
    }
    pub(crate) fn awaiting_initial_format(&self) -> bool {
        self.formatter.is_some()
            && self._format_subscription.is_some()
            && self.format_revision.load(Ordering::Relaxed) == 0
            && self.formatted.is_none()
    }
    pub(crate) fn offset_y(
        &self,
        source_offset: usize,
        code_size: f64,
        cx: &gpui_kit::App,
    ) -> gpui_kit::Pixels {
        let editor = self.editor.read(cx);
        let offset = self
            .display_range(source_offset..source_offset.saturating_add(1), cx)
            .start;
        let row = editor.text().offset_to_point(offset).row;
        let map = editor.display_map();
        let row = if map.display_row_count() == 0 {
            row
        } else {
            map.buffer_line_to_display_row(row)
        };
        row_offset_y(row, code_size)
    }
    pub(crate) fn reveal(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.collapsed = false;
        self.error = None;
        self.revealed_source = Some(range.clone());
        let range = self.display_range(range, cx);
        self.editor.update(cx, |editor, cx| {
            let position = editor.text().offset_to_position(range.start);
            editor.unfold_at(position, cx);
            editor.set_selected_range(range, cx);
        });
        cx.notify();
    }
    fn set_tab_width(&mut self, width: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.tab_override = width;
        let width = width.unwrap_or(self.detected_width);
        self.tab_width = width;
        self.editor.update(cx, |editor, cx| {
            editor.set_tab_size(
                TabSize {
                    tab_size: width,
                    hard_tabs: false,
                },
                cx,
            )
        });
        if self.supports_formatting() {
            self.request_format(window, cx, true);
        }
        cx.notify();
    }
    fn supports_formatting(&self) -> bool {
        self.formatter.is_some()
    }
    fn request_format(&mut self, window: &mut Window, cx: &mut Context<Self>, report_error: bool) {
        let Some(formatter) = self.formatter else {
            return;
        };
        let revision = self.format_revision.fetch_add(1, Ordering::Relaxed) + 1;
        let generation = self.format_revision.clone();
        self.format_job = None;
        self.error = None;
        let source = self.source.clone();
        let range = self.code.clone();
        let width = self.tab_width;
        let job = cx.background_spawn(async move {
            let _permit = SLOTS.acquire().await;
            if generation.load(Ordering::Relaxed) != revision {
                return None;
            }
            let raw = &source.as_str()[range];
            Some(formatter.format(display_source(&source, raw), width))
        });
        self.format_job = Some(cx.spawn_in(window, async move |view, cx| {
            let Some(result) = job.await else {
                return;
            };
            let _ = view.update_in(cx, |view, window, cx| {
                if view.format_revision.load(Ordering::Relaxed) != revision {
                    return;
                }
                view.format_job = None;
                match result {
                    Ok(formatted) => {
                        let display = formatted.text().to_owned();
                        view.formatted = Some(Arc::new(formatted));
                        view.replace_display(display, window, cx);
                    }
                    Err(error) => {
                        if report_error {
                            view.error = Some(format!("格式化失败：{error:#}"));
                        }
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }
    fn replace_display(&mut self, display: String, window: &mut Window, cx: &mut Context<Self>) {
        self.lines = display.bytes().filter(|&b| b == b'\n').count() + 1;
        self.error = None;
        self.editor.update(cx, |editor, cx| {
            editor.set_folding(false, window, cx);
            editor.set_folding(true, window, cx);
            editor.set_value(display, window, cx);
        });
        self.apply_highlights(cx);
        if let Some(range) = self.revealed_source.clone() {
            self.reveal(range, cx);
        }
        cx.notify();
    }
}
impl Drop for CodeCard {
    fn drop(&mut self) {
        self.format_revision.fetch_add(1, Ordering::Relaxed);
    }
}
fn display_source<'src>(source: &CodeSource, raw: &'src str) -> &'src str {
    // The AST has already removed the required fence newline from fragments.
    // Strip it only from raw document ranges; preserve intentional blank lines.
    match source {
        CodeSource::Document(_) => raw
            .strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(raw),
        CodeSource::Fragment(_) => raw,
    }
}

/// Border (1), toolbar (40), and text inset (8) precede the first line.
pub(crate) fn row_offset_y(row: usize, code_size: f64) -> gpui_kit::Pixels {
    let line_height = (code_size as f32 * 1.65).round();
    px(49. + row as f32 * line_height)
}
impl Render for CodeCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = Preferences::shared(cx).read(cx).data.clone();
        let interface = crate::presentation::interface_style(cx);
        let palette = ReadingPalette::for_app(cx);
        let line_height = (prefs.code_size as f32 * 1.65).round();
        let map = self.editor.read(cx).display_map();
        let rows = if map.folded_ranges().is_empty() {
            self.lines
        } else {
            map.display_row_count().max(1)
        };
        let height = rows as f32 * line_height + CODE_PADDING;
        let tab_weak = cx.weak_entity();
        let formatted = self.formatted.is_some();
        let editor = self.editor.clone();
        let auto_width = self.detected_width;
        let tab_override = self.tab_override;
        let source = self.source.clone();
        let code = self.code.clone();
        div()
            .debug_selector(|| "code-frame".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(palette.code)
            .border_1()
            .border_color(palette.border)
            .rounded(px(3.))
            .overflow_hidden()
            .child(
                div()
                    .debug_selector(|| "code-header".into())
                    .w_full()
                    .flex()
                    .justify_between()
                    .items_center()
                    .px(px(12.))
                    .py(px(6.))
                    .bg(palette.wash)
                    .refine_style(&interface)
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(palette.muted)
                    .child(
                        div()
                            .debug_selector(|| "code-language-label".into())
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                gpui_kit::svg()
                                    .path(self.language_icon.path)
                                    .size(px(16.))
                                    .flex_shrink_0()
                                    .text_color(palette.muted)
                                    .debug_selector(|| "code-language-logo".into()),
                            )
                            .child(if self.language.is_empty() {
                                "text".to_owned()
                            } else {
                                self.language.clone()
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .child(
                                crate::ui::tool_button(
                                    "code-indent",
                                    gpui_kit::assets::IconName::ListIndentIncrease,
                                    if self.supports_formatting() {
                                        "Tab 宽度（选择后自动格式化）"
                                    } else {
                                        "Tab 宽度（此语言保留原始排版）"
                                    },
                                )
                                .debug_selector(|| "code-indent".into())
                                .dropdown_menu(
                                    move |mut menu, _, _| {
                                        for (label, width) in [
                                            (format!("自动识别（{auto_width} 列）"), None),
                                            ("2 列".to_owned(), Some(2)),
                                            ("4 列".to_owned(), Some(4)),
                                            ("8 列".to_owned(), Some(8)),
                                        ] {
                                            let weak = tab_weak.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(label)
                                                    .checked(width == tab_override)
                                                    .on_click(move |_, window, cx| {
                                                        let _ = weak.update(cx, |view, cx| {
                                                            view.set_tab_width(width, window, cx)
                                                        });
                                                    }),
                                            );
                                        }
                                        menu
                                    },
                                ),
                            )
                            .child(
                                crate::ui::tool_button(
                                    "code-collapse",
                                    if self.collapsed {
                                        gpui_kit::assets::IconName::UnfoldVertical
                                    } else {
                                        gpui_kit::assets::IconName::FoldVertical
                                    },
                                    if self.collapsed {
                                        "展开代码块"
                                    } else {
                                        "折叠代码块"
                                    },
                                )
                                .debug_selector(|| "code-collapse".into())
                                .on_click(cx.listener(
                                    |view, _, _, cx| {
                                        view.collapsed = !view.collapsed;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                div().debug_selector(|| "code-copy".into()).child(
                                    crate::ui::tool_button("copy-code", IconName::Copy, "复制代码")
                                        .on_click(move |_, _, cx| {
                                            let text = if formatted {
                                                editor.read(cx).text().to_string()
                                            } else {
                                                source.as_str()[code.clone()].to_owned()
                                            };
                                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                                        }),
                                ),
                            ),
                    ),
            )
            .when_some(self.error.as_ref(), |this, error| {
                this.child(
                    div()
                        .px(px(12.))
                        .py(px(6.))
                        .text_size(px(12.))
                        .text_color(palette.muted)
                        .child(error.clone()),
                )
            })
            .when(!self.collapsed, |this| {
                this.child(
                    div()
                        .debug_selector(|| "code-viewport".into())
                        .w_full()
                        .h(px(height))
                        .overflow_hidden()
                        .child(
                            Editor::new(&self.editor)
                                .readonly(true)
                                .appearance(false)
                                .bordered(false)
                                .w_full()
                                .h_full()
                                .rounded_none()
                                .bg(palette.code)
                                .refine_style(&code_type(&prefs))
                                .text_size(px(prefs.code_size as f32))
                                .line_height(px(line_height))
                                .px(px(16.)),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext, TestAppContext, point};
    use pecan_document::Block;

    struct Fixture {
        card: Entity<CodeCard>,
        width: f32,
        top: f32,
        vertical_wheels: usize,
    }
    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("code-fixture")
                .w(px(self.width))
                .h(px(500.))
                .relative()
                .overflow_hidden()
                .on_scroll(
                    cx.listener(|fixture, event: &gpui_kit::ScrollEvent, window, _| {
                        if event.delta.pixel_delta(window.line_height()).y != px(0.) {
                            fixture.vertical_wheels += 1;
                        }
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(self.top))
                        .w_full()
                        .child(self.card.clone()),
                )
        }
    }

    fn code_document(body: &str) -> (Arc<Document>, Range<usize>) {
        let document = Arc::new(Document::parse(format!("```json\n{body}```\n")).unwrap());
        let Block::CodePage { code, .. } = &document.blocks()[0] else {
            panic!("expected fenced code")
        };
        let range = code.clone();
        (document, range)
    }

    #[gpui_kit::test]
    fn language_logo_keeps_toolbar_geometry_in_both_themes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (document, code) = code_document("{\"value\": 1}\n");
        let (_, visual) = cx.add_window_view(|window, cx| Fixture {
            card: cx.new(|cx| CodeCard::new(document, code, "sql".into(), window, cx)),
            width: 600.,
            top: 0.,
            vertical_wheels: 0,
        });
        for mode in [
            gpui_kit::component::ThemeMode::Light,
            gpui_kit::component::ThemeMode::Dark,
        ] {
            visual.update(|_, cx| {
                gpui_kit::component::Theme::change(mode, None, cx);
                gpui_kit::component::Theme::update(cx, crate::brand::apply);
            });
            draw_card(visual);
            let header = visual.debug_bounds("code-header").unwrap();
            let logo = visual.debug_bounds("code-language-logo").unwrap();
            let label = visual.debug_bounds("code-language-label").unwrap();
            assert_eq!(logo.size, gpui_kit::size(px(16.), px(16.)));
            assert!(
                (logo.center().y - header.center().y).abs() < px(0.1),
                "logo is vertically centered: {mode:?}"
            );
            assert!((logo.origin.x - header.origin.x - px(12.)).abs() < px(0.1));
            assert!(
                (label.size.height - px(16.)).abs() < px(0.1),
                "logo does not enlarge the header"
            );
        }
    }

    #[gpui_kit::test]
    fn expanded_editor_keeps_horizontal_scroll_and_shapes_only_visible_rows(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let line = "{\"value\":42}\n";
        let body = format!(
            "{{\"value\":\"{}\"}}\n{}",
            "long_中文_value ".repeat(60),
            line.repeat(49_999)
        );
        let (document, code) = code_document(&body);
        let (root, visual) = cx.add_window_view(|window, cx| Fixture {
            card: cx.new(|cx| CodeCard::new(document, code, "json".into(), window, cx)),
            width: 600.,
            top: 0.,
            vertical_wheels: 0,
        });
        for width in [600., 320.] {
            visual.update(|window, cx| {
                root.update(cx, |root, _| root.width = width);
                window.refresh();
                window.draw(cx).clear(cx);
                window.draw(cx).clear(cx);
            });
            let viewport = visual.debug_bounds("code-viewport").unwrap();
            assert!(
                viewport.size.height > px(1_000_000.),
                "all 50000 lines are expanded"
            );
            visual.update(|_, cx| {
                let editor = root.read(cx).card.read(cx).editor.read(cx);
                assert_eq!(editor.text().len(), body.len() - 1);
                assert_eq!(editor.display_map().wrap_row_count(), 50_000);
                assert!(
                    editor.visible_row_range().unwrap().len() < 40,
                    "parent clipping limits actual shaped rows despite full height"
                );
            });
        }
        let copy = visual.debug_bounds("code-copy").unwrap();
        visual.simulate_click(copy.center(), gpui_kit::Modifiers::default());
        visual.update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), body));
        visual.simulate_event(gpui_kit::ScrollEvent {
            position: point(px(150.), px(200.)),
            delta: gpui_kit::ScrollDelta::Pixels(point(px(-500.), px(0.))),
            ..Default::default()
        });
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
            let offset = root.read(cx).card.read(cx).editor.read(cx).scroll_offset();
            assert!(
                offset.x < px(0.),
                "horizontal native scrolling remains available"
            );
            assert_eq!(offset.y, px(0.), "there is no vertical code scrolling");
        });
        visual.simulate_event(gpui_kit::ScrollEvent {
            position: point(px(150.), px(200.)),
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-120.))),
            ..Default::default()
        });
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
            let fixture = root.read(cx);
            assert_eq!(
                fixture.vertical_wheels, 1,
                "vertical scrolling over code must reach the document parent"
            );
            assert_eq!(
                fixture.card.read(cx).editor.read(cx).scroll_offset().y,
                px(0.)
            );
        });
        for row in [25_000, 49_999] {
            let local = body.len() - (50_000 - row) * line.len();
            visual.update(|window, cx| {
                let card = root.read(cx).card.clone();
                let offset = card.read(cx).code.start + local;
                let y = card.read(cx).offset_y(offset, 14., cx).as_f32();
                root.update(cx, |root, _| root.top = -y);
                card.update(cx, |card, cx| card.reveal(offset..offset + 2, cx));
                window.refresh();
                window.draw(cx).clear(cx);
                window.draw(cx).clear(cx);
                let editor = card.read(cx).editor.read(cx);
                assert_eq!(editor.selected_range(), local..local + 2);
                let visible = editor.visible_row_range().unwrap();
                assert!(
                    visible.contains(&row) && visible.len() < 40,
                    "outer scrolling reaches row {row} while drawing only a few rows: {visible:?}"
                );
                assert_eq!(editor.scroll_offset().y, px(0.));
            });
        }
    }

    #[gpui_kit::test]
    fn reading_code_does_not_open_the_editors_second_context_menu(cx: &mut TestAppContext) {
        use std::{cell::Cell, rc::Rc};
        cx.update(gpui_kit::init);
        let (document, code) = code_document("{\"value\": 1}\n");
        let (root, visual) = cx.add_window_view(|window, cx| Fixture {
            card: cx.new(|cx| CodeCard::new(document, code, "json".into(), window, cx)),
            width: 600.,
            top: 0.,
            vertical_wheels: 0,
        });
        draw_card(visual);
        let calls = Rc::new(Cell::new(0));
        visual.update(|_, cx| {
            let editor = root.read(cx).card.read(cx).editor.clone();
            let calls = calls.clone();
            editor.update(cx, |editor, _| {
                editor.on_context_menu(Rc::new(move |_, _, _, _, _| calls.set(calls.get() + 1)));
            });
        });
        let viewport = visual.debug_bounds("code-viewport").unwrap();
        let position = viewport.origin + point(px(50.), px(15.));
        visual.simulate_mouse_down(position, gpui_kit::MouseButton::Right, Default::default());
        visual.simulate_mouse_up(position, gpui_kit::MouseButton::Right, Default::default());
        assert_eq!(
            calls.get(),
            0,
            "reading code must not open the native editor menu"
        );
        draw_card(visual);
        visual.simulate_click(position, Default::default());
        visual.simulate_keystrokes(crate::platform::choose("cmd-a cmd-c", "ctrl-a ctrl-c"));
        visual.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                root.read(cx)
                    .card
                    .read(cx)
                    .editor
                    .read(cx)
                    .text()
                    .to_string(),
                "selection and keyboard copy remain usable"
            );
        });
    }

    fn draw_card(visual: &mut gpui_kit::VisualTestContext) {
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
    }
    fn choose_indent(visual: &mut gpui_kit::VisualTestContext, keys: &str) {
        let button = visual
            .debug_bounds("code-indent")
            .expect("indent button is visible")
            .center();
        visual.simulate_click(button, Default::default());
        draw_card(visual);
        visual.simulate_keystrokes(keys);
        draw_card(visual);
    }

    #[gpui_kit::test]
    fn selecting_indent_formats_json_family_without_a_separate_format_click(
        cx: &mut TestAppContext,
    ) {
        let body = "{\n    \"a\": {\n        \"tail\": [1,2] // comment\n    }\n}\n";
        for language in ["json", "JSONC", "json-ld"] {
            let (document, code) = code_document(body);
            let original = document.source().to_owned();
            let (handle, root) = cx.update(|cx| {
                gpui_kit::init(cx);
                gpui_kit::open_window(Default::default(), cx, |window, cx| {
                    cx.new(|cx| Fixture {
                        card: cx
                            .new(|cx| CodeCard::new(document, code, language.into(), window, cx)),
                        width: 600.,
                        top: 0.,
                        vertical_wheels: 0,
                    })
                })
                .unwrap()
            });
            let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
            draw_card(&mut visual);
            visual.run_until_parked();
            draw_card(&mut visual);
            visual.update(|_, cx| assert!(root.read(cx).card.read(cx).formatted.is_some()));
            for (width, keys) in [
                (2, "down down enter"),
                (4, "down down down enter"),
                (8, "down down down down enter"),
                (4, "down enter"),
            ] {
                choose_indent(&mut visual, keys);
                visual.update(|_, cx| {
                    let card = root.read(cx).card.read(cx);
                    assert_eq!(
                        card.tab_width, width,
                        "{language} menu must set width {width}"
                    );
                    assert!(
                        card.formatted.is_some(),
                        "{language} choosing indent must format an untouched block"
                    );
                    let text = card.editor.read(cx).text().to_string();
                    assert!(text.contains(&format!("\n{}\"a\"", " ".repeat(width))));
                    assert!(
                        text.contains("// comment"),
                        "comments survive automatic formatting"
                    );
                    assert_eq!(card.source.as_str(), original);
                    assert!(card.editor.read(cx).display_map().is_fold_candidate(0));
                    assert!(card.error.is_none());
                });
            }
            let copy = visual.debug_bounds("code-copy").unwrap().center();
            visual.simulate_click(copy, Default::default());
            visual.update(|_, cx| {
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().unwrap(),
                    root.read(cx)
                        .card
                        .read(cx)
                        .editor
                        .read(cx)
                        .text()
                        .to_string()
                )
            });
            assert!(visual.debug_bounds("code-format").is_none());
            visual.update(|window, _| window.remove_window());
        }
    }

    #[gpui_kit::test]
    fn yaml_and_toml_indent_copy_search_and_collapse(cx: &mut TestAppContext) {
        for (language, body, needle, indented) in [
            (
                "yaml",
                "# 配置\nreader:\n  name: '设备'\n  port: 8080 # 端口\n",
                "设备",
                "name:",
            ),
            ("YML", "reader:\n  name: \"设备\"\n", "设备", "name:"),
            (
                "TOML",
                "# 配置\nports=[\n  8080, # 端口\n  8081,\n]\n",
                "端口",
                "8080,",
            ),
        ] {
            let (document, code) = code_document(body);
            let original = document.source().to_owned();
            let at = code.start + body.find(needle).unwrap();
            let (handle, root) = cx.update(|cx| {
                gpui_kit::init(cx);
                gpui_kit::open_window(Default::default(), cx, |window, cx| {
                    cx.new(|cx| Fixture {
                        card: cx
                            .new(|cx| CodeCard::new(document, code, language.into(), window, cx)),
                        width: 600.,
                        top: 0.,
                        vertical_wheels: 0,
                    })
                })
                .unwrap()
            });
            let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
            draw_card(&mut visual);
            visual.update(|_, cx| {
                let editor = root.read(cx).card.read(cx).editor.read(cx);
                let syntax_language = editor.language_name();
                assert_eq!(
                    syntax_language.as_ref(),
                    if language == "TOML" { "toml" } else { "yaml" }
                );
                assert!(
                    gpui_kit::component::highlighter::LanguageRegistry::singleton()
                        .language(&syntax_language)
                        .is_some_and(|grammar| grammar.language.is_some()),
                    "configuration cards must use an available syntax grammar"
                );
            });
            for (width, keys) in [(4, "down down down enter"), (2, "down down enter")] {
                choose_indent(&mut visual, keys);
                visual.update(|_, cx| {
                    let card = root.read(cx).card.read(cx);
                    assert_eq!(card.tab_width, width, "{language}");
                    assert!(card.error.is_none(), "{language}: {:?}", card.error);
                    assert!(card.formatted.is_some(), "{language} was formatted");
                    assert_eq!(
                        card.source.as_str(),
                        original,
                        "formatting never edits the document"
                    );
                    assert!(
                        card.editor
                            .read(cx)
                            .text()
                            .to_string()
                            .contains(&format!("\n{}{indented}", " ".repeat(width)))
                    );
                });
            }
            let copy = visual.debug_bounds("code-copy").unwrap().center();
            visual.simulate_click(copy, Default::default());
            visual.update(|_, cx| {
                let card = root.read(cx).card.clone();
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().unwrap(),
                    card.read(cx).editor.read(cx).text().to_string()
                );
                card.update(cx, |card, cx| card.reveal(at..at + needle.len(), cx));
                let editor = card.read(cx).editor.read(cx);
                assert_eq!(
                    &editor.text().to_string()[editor.selected_range()],
                    needle,
                    "formatted search maps to original source: {language}"
                );
            });
            draw_card(&mut visual);
            let collapse = visual.debug_bounds("code-collapse").unwrap().center();
            visual.simulate_click(collapse, Default::default());
            draw_card(&mut visual);
            assert!(visual.debug_bounds("code-viewport").is_none());
            visual.simulate_click(collapse, Default::default());
            draw_card(&mut visual);
            assert!(visual.debug_bounds("code-viewport").is_some());
            assert!(visual.debug_bounds("code-format").is_none());
            visual.update(|window, _| window.remove_window());
        }
    }

    #[gpui_kit::test]
    fn auto_format_on_indent_keeps_invalid_json_and_other_languages(cx: &mut TestAppContext) {
        for (language, body, error) in [
            ("json", "[1,,]\n", true),
            ("rust", "fn main() {\n    println!(\"ok\");\n}\n", false),
        ] {
            let (document, code) = code_document(body);
            let (handle, root) = cx.update(|cx| {
                gpui_kit::init(cx);
                gpui_kit::open_window(Default::default(), cx, |window, cx| {
                    cx.new(|cx| Fixture {
                        card: cx
                            .new(|cx| CodeCard::new(document, code, language.into(), window, cx)),
                        width: 600.,
                        top: 0.,
                        vertical_wheels: 0,
                    })
                })
                .unwrap()
            });
            let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
            draw_card(&mut visual);
            if language == "rust" {
                assert!(
                    visual.debug_bounds("code-indent").is_some(),
                    "unsupported languages can still adjust literal Tab display"
                );
                assert!(visual.debug_bounds("code-format").is_none());
                let copy = visual.debug_bounds("code-copy").unwrap().center();
                visual.simulate_click(copy, Default::default());
                visual.update(|_, cx| {
                    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), body)
                });
            } else {
                choose_indent(&mut visual, "down down enter");
            }
            visual.update(|_, cx| {
                let card = root.read(cx).card.read(cx);
                assert_eq!(card.tab_width, if language == "rust" { 4 } else { 2 });
                assert_eq!(card.error.is_some(), error, "{language}");
                assert!(card.formatted.is_none());
                assert_eq!(
                    card.editor.read(cx).text().to_string(),
                    body.trim_end_matches('\n')
                );
            });
            visual.update(|window, _| window.remove_window());
        }
    }

    #[gpui_kit::test]
    fn code_body_uses_the_subdued_fill_and_a_distinct_toolbar_in_both_themes(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (document, code) = code_document("{\n  \"value\": 1\n}\n");
        let (_, visual) = cx.add_window_view(|window, cx| Fixture {
            card: cx.new(|cx| CodeCard::new(document, code, "json".into(), window, cx)),
            width: 600.,
            top: 0.,
            vertical_wheels: 0,
        });
        for mode in [
            gpui_kit::component::ThemeMode::Light,
            gpui_kit::component::ThemeMode::Dark,
        ] {
            visual.update(|_, cx| {
                gpui_kit::component::Theme::change(mode, None, cx);
                gpui_kit::component::Theme::update(cx, crate::brand::apply);
            });
            draw_card(visual);
            let viewport = visual.debug_bounds("code-viewport").unwrap();
            visual.update(|window, cx| {
                let palette = ReadingPalette::for_app(cx);
                let scale = window.scale_factor();
                assert!(
                    (palette.code.l - palette.wash.l).abs() > 0.025,
                    "code body and toolbar should have visible tonal separation: {mode:?}"
                );
                let quads = window.painted_quads();
                let body = quads
                    .iter()
                    .find(|quad| {
                        quad.background.as_solid() == Some(palette.code)
                            && quad.bounds.size.height.0 > 50.
                            && (quad.bounds.top().0 - viewport.top().as_f32() * scale).abs() <= 1.
                            && (quad.bounds.bottom().0 - viewport.bottom().as_f32() * scale).abs()
                                <= 1.
                    })
                    .expect("native editor fill is painted");
                let header = quads
                    .iter()
                    .find(|quad| {
                        quad.background.as_solid() == Some(palette.wash)
                            && quad.bounds.size.width.0 > 500.
                    })
                    .expect("native toolbar fill is painted");
                assert!(body.bounds.bottom() > header.bounds.bottom());
                assert_ne!(body.background, header.background);
            });
        }
    }

    #[gpui_kit::test]
    fn automatic_format_copy_and_search_preserve_source_and_update_height(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let body = "{\"中文\":[1,{\"tail\":2}]}\n";
        let (document, code) = code_document(body);
        let source = document.source().to_owned();
        let (root, visual) = cx.add_window_view(|window, cx| Fixture {
            card: cx.new(|cx| CodeCard::new(document, code, "json".into(), window, cx)),
            width: 600.,
            top: 0.,
            vertical_wheels: 0,
        });
        let draw = |visual: &mut gpui_kit::VisualTestContext| {
            visual.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
                window.draw(cx).clear(cx);
            })
        };
        draw(visual);
        assert!(visual.debug_bounds("code-format").is_none());
        visual.run_until_parked();
        draw(visual);
        visual.update(|_, cx| {
            let card = root.read(cx).card.read(cx);
            assert_eq!(card.source.as_str(), source);
            assert!(card.formatted.is_some());
            assert_eq!(card.editor.read(cx).text().to_string(), "{\n    \"中文\": [\n        1,\n        {\n            \"tail\": 2\n        }\n    ]\n}");
            assert!(card.editor.read(cx).display_map().is_fold_candidate(0), "formatting keeps syntax folding available");
        });
        assert!(visual.debug_bounds("code-viewport").unwrap().size.height > px(150.));
        let button = visual.debug_bounds("code-copy").unwrap().center();
        visual.simulate_click(button, Default::default());
        visual.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                root.read(cx)
                    .card
                    .read(cx)
                    .editor
                    .read(cx)
                    .text()
                    .to_string()
            )
        });
        let button = visual.debug_bounds("code-collapse").unwrap().center();
        visual.simulate_click(button, Default::default());
        draw(visual);
        assert!(visual.debug_bounds("code-viewport").is_none());
        visual.update(|window, cx| {
            let card = root.read(cx).card.clone();
            card.update(cx, |card, cx| {
                let at = source.find("tail").unwrap();
                card.highlight(std::iter::once(at..at + 4), cx);
                card.reveal(at..at + 4, cx);
                let range = card.editor.read(cx).selected_range();
                assert_eq!(card.editor.read(cx).text().slice(range).to_string(), "tail");
                assert!(!card.collapsed);
                card.set_tab_width(Some(2), window, cx);
            });
        });
        visual.run_until_parked();
        draw(visual);
        visual.update(|_, cx| {
            let card = root.read(cx).card.clone();
            assert!(
                card.read(cx)
                    .editor
                    .read(cx)
                    .text()
                    .to_string()
                    .contains("\n  \"中文\"")
            );
            assert_eq!(card.read(cx).source.as_str(), source);
        });
    }

    #[gpui_kit::test]
    fn rapid_tab_changes_keep_only_the_latest_result_and_search(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let body = "{\"items\":[{\"tail\":42}]}\n";
        let (document, code) = code_document(body);
        let at = document.source().find("tail").unwrap();
        let (root, visual) = cx.add_window_view(|window, cx| Fixture {
            card: cx.new(|cx| CodeCard::new(document, code, "json".into(), window, cx)),
            width: 600.,
            top: 0.,
            vertical_wheels: 0,
        });
        visual.update(|window, cx| {
            root.read(cx).card.clone().update(cx, |card, cx| {
                card.reveal(at..at + 4, cx);
                for width in [2, 8, 4] {
                    card.set_tab_width(Some(width), window, cx);
                }
            });
        });
        draw_card(visual);
        visual.update(|_, cx| {
            let card = root.read(cx).card.read(cx);
            let editor = card.editor.read(cx);
            assert_eq!(card.tab_width, 4);
            assert!(editor.text().to_string().contains("\n    \"items\""));
            assert_eq!(
                editor.text().slice(editor.selected_range()).to_string(),
                "tail"
            );
            assert!(card.error.is_none());
        });
    }

    #[gpui_kit::test]
    fn native_structure_fold_shrinks_card_and_search_unfolds_target(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let body = "{\n    \"a\": {\n        \"b\": 1,\n        \"tail\": 2\n    }\n}\n";
        let (document, code) = code_document(body);
        let at = document.source().find("tail").unwrap();
        let (root, visual) = cx.add_window_view(|window, cx| Fixture {
            card: cx.new(|cx| CodeCard::new(document, code, "json".into(), window, cx)),
            width: 600.,
            top: 0.,
            vertical_wheels: 0,
        });
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
        let viewport = visual.debug_bounds("code-viewport").unwrap();
        // Native gutter: first fold chevron relative to the editor's measured text area.
        let fold_at = visual.update(|_, cx| {
            let editor = root.read(cx).card.read(cx).editor.read(cx);
            editor.text_bounds().unwrap().origin + point(px(8.), px(11.5))
        });
        visual.simulate_mouse_move(fold_at, None, Default::default());
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
        visual.simulate_click(fold_at, Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
        visual.update(|_, cx| {
            let card = root.read(cx).card.read(cx);
            assert!(
                !card
                    .editor
                    .read(cx)
                    .display_map()
                    .folded_ranges()
                    .is_empty(),
                "native gutter click folds the object"
            );
        });
        assert!(visual.debug_bounds("code-viewport").unwrap().size.height < viewport.size.height);
        visual.update(|_, cx| {
            root.read(cx).card.clone().update(cx, |card, cx| {
                card.reveal(at..at + 4, cx);
                assert!(
                    card.editor
                        .read(cx)
                        .display_map()
                        .folded_ranges()
                        .is_empty()
                );
                assert_eq!(card.offset_y(at, 14., cx), row_offset_y(3, 14.));
                assert_eq!(
                    card.editor
                        .read(cx)
                        .text()
                        .slice(card.editor.read(cx).selected_range())
                        .to_string(),
                    "tail"
                );
            });
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
        assert_eq!(
            visual.debug_bounds("code-viewport").unwrap().size.height,
            viewport.size.height
        );
    }

    #[test]
    fn configuration_fixture_preserves_comments_and_tab_width() {
        let document =
            Document::parse(include_str!("../../../samples/code-layout.md").to_owned()).unwrap();
        let widths: Vec<_> = document
            .blocks()
            .iter()
            .filter_map(|block| {
                let Block::CodePage { code, language, .. } = block else {
                    return None;
                };
                if !["json", "jsonc", "json-ld"].contains(&language.as_str()) {
                    return None;
                }
                let source = &document.source()[code.clone()];
                for width in [2, 4, 8] {
                    let formatted = formatting::Format::Json.format(source, width).unwrap();
                    assert_eq!(
                        formatting::Format::Json
                            .format(formatted.text(), width)
                            .unwrap()
                            .text(),
                        formatted.text()
                    );
                }
                Some(formatting::tab_width(source, "auto"))
            })
            .collect();
        assert_eq!(widths, [2, 4, 4]);
    }

    #[gpui_kit::test]
    fn invalid_json_keeps_code_and_reports_failure(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (document, code) = code_document("[1,,]\n");
        let (root, visual) = cx.add_window_view(|window, cx| Fixture {
            card: cx.new(|cx| CodeCard::new(document, code, "json".into(), window, cx)),
            width: 600.,
            top: 0.,
            vertical_wheels: 0,
        });
        visual.update(|window, cx| {
            root.read(cx)
                .card
                .clone()
                .update(cx, |card, cx| card.set_tab_width(Some(2), window, cx))
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            let card = root.read(cx).card.read(cx);
            assert!(card.error.is_some());
            assert!(card.formatted.is_none());
            assert_eq!(card.editor.read(cx).text().to_string(), "[1,,]");
        });
    }

    #[gpui_kit::test]
    #[cfg(target_os = "macos")]
    #[ignore = "requires a macOS graphical session with Metal access"]
    fn native_border_bottom_survives_scrolling_in_real_metal_pixels(cx: &mut TestAppContext) {
        use gpui_kit::{DevicePixels, Scene, rgb, size};
        struct Outline {
            height: f32,
            top: f32,
        }
        impl Render for Outline {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .bg(rgb(0xffffff))
                    .overflow_hidden()
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .left(px(20.))
                            .top(px(self.top))
                            .w(px(400.))
                            .h(px(self.height))
                            .bg(rgb(0xffffff))
                            .border_1()
                            .border_color(rgb(0xff00ff))
                            .rounded(px(3.))
                            .overflow_hidden()
                            .child(div().w_full().h_full().bg(rgb(0xffffff))),
                    )
            }
        }
        cx.update(gpui_kit::init);
        let (root, visual) = cx.add_window_view(|_, _| Outline {
            height: 480.,
            top: -180.,
        });
        let mut renderer = gpui_platform::current_headless_renderer().unwrap().unwrap();
        for (height, top) in [
            (480., -180.),
            (480., -180.25),
            (3000., -2700.),
            (3000., -2700.25),
        ] {
            let (image, scale) = visual.update(|window, cx| {
                window.resize(size(px(480.), px(360.)));
                root.update(cx, |root, cx| {
                    root.height = height;
                    root.top = top;
                    cx.notify();
                });
                window.refresh();
                window.draw(cx).clear(cx);
                let mut scene = Scene::default();
                for quad in window.painted_quads() {
                    scene.insert_primitive(quad);
                }
                scene.finish();
                let scale = window.scale_factor();
                (
                    renderer
                        .render_scene_to_image(
                            &scene,
                            size(
                                DevicePixels((480. * scale) as i32),
                                DevicePixels((360. * scale) as i32),
                            ),
                        )
                        .unwrap(),
                    scale,
                )
            });
            if let Ok(directory) = std::env::var("PECAN_METAL_OUTPUT") {
                image
                    .save(
                        std::path::Path::new(&directory).join(format!("border-{height}-{top}.png")),
                    )
                    .unwrap();
            }
            let bottom = ((height + top) * scale) as u32;
            for x in (24. * scale) as u32..(416. * scale) as u32 {
                assert!(
                    (bottom - 3..=bottom + 2).any(|y| {
                        let pixel = image.get_pixel(x, y).0;
                        pixel[0] > 180 && pixel[2] > 180 && pixel[1] < 80
                    }),
                    "native border bottom pixel disappeared at x={x}, height={height}, top={top}"
                );
            }
        }
    }

    #[gpui_kit::test]
    fn short_code_removes_only_the_fence_newline_and_uses_its_own_size(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (body, expected_lines) in [("42\n", 1), ("42\n\n", 2), ("42\r\n", 1)] {
            let (document, code) = code_document(body);
            let (root, visual) = cx.add_window_view(|window, cx| Fixture {
                card: cx.new(|cx| CodeCard::new(document, code, "text".into(), window, cx)),
                width: 500.,
                top: 0.,
                vertical_wheels: 0,
            });
            for (body_size, code_size) in [(16., 14.), (24., 14.), (16., 18.)] {
                visual.update(|window, cx| {
                    Preferences::shared(cx).update(cx, |prefs, _| {
                        let data = Arc::make_mut(&mut prefs.data);
                        data.font_size = body_size;
                        data.code_size = code_size;
                    });
                    root.read(cx).card.clone().update(cx, |_, cx| cx.notify());
                    window.refresh();
                    window.draw(cx).clear(cx);
                    window.draw(cx).clear(cx);
                    let card = root.read(cx).card.read(cx);
                    assert_eq!(
                        card.editor.read(cx).display_map().wrap_row_count(),
                        expected_lines
                    );
                    assert!(
                        (card.editor.read(cx).line_height().unwrap()
                            - px((code_size as f32 * 1.65).round()))
                        .abs()
                            < px(0.1),
                        "body {body_size}, code {code_size}: actual line height {:?}",
                        card.editor.read(cx).line_height()
                    );
                });
                let viewport = visual.debug_bounds("code-viewport").unwrap();
                assert!(
                    (viewport.size.height
                        - px(expected_lines as f32 * (code_size as f32 * 1.65).round()
                            + CODE_PADDING))
                    .abs()
                        < px(0.1)
                );
            }
        }
    }
}
