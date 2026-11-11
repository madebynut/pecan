//! Highlight containers: DingTalk `:::` fences and GitHub `[!NOTE]` alerts.
//! The body is ordinary Markdown, so lists, quotes and code keep their own layout.
use gpui_kit::{
    App, AvailableSpace, Bounds, Hsla, IntoElement, Pixels, SharedString, Window,
    assets::IconName,
    base::{StyledExt as _, TextView},
    canvas,
    component::{
        ActiveTheme, Icon,
        text::{
            MarkdownExtensions, MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast,
        },
        v_flex,
    },
    div, point,
    prelude::*,
    px, relative, rgb, size,
};
use pecan_document::{AlertKind, alert_marker};
use std::{cell::Cell, rc::Rc};

pub(crate) type LeadInk = Rc<Cell<Option<Bounds<Pixels>>>>;

use crate::presentation::ReadingPalette;

const ICON_SIZE: f32 = 20.;

pub(crate) struct NoteStyle {
    pub(crate) icon: IconName,
    pub(crate) title: Option<String>,
    pub(crate) fill: Hsla,
    pub(crate) edge: Hsla,
    pub(crate) mark: Hsla,
    pub(crate) ink: Hsla,
    pub(crate) lead: Option<LeadInk>,
}

/// Titled alerts stack their body below the heading; untitled notes share the
/// first body line with the icon, leaving Markdown in one continuous column.
pub(crate) fn frame(style: NoteStyle, body: impl IntoElement, cx: &mut App) -> impl IntoElement {
    let typography = crate::presentation::styles(cx).body;
    let titled = style.title.is_some();
    let lead = style.lead.unwrap_or_default();
    let title_lead = lead.clone();
    let heading = style.title.map(|title| {
        div()
            .debug_selector(|| "pecan-note-heading".into())
            .pl(px(ICON_SIZE + 8.))
            .min_h(px(22.))
            .line_height(relative(1.25))
            .child(
                div()
                    .debug_selector(|| "pecan-note-title".into())
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(style.ink)
                    .child(
                        TextView::markdown("pecan-note-title-text", title)
                            .style(
                                crate::presentation::styles(cx)
                                    .note
                                    .with_paragraph_gap(gpui_kit::rems(0.)),
                            )
                            .markdown_extensions(
                                MarkdownExtensions::default().inline_code_emphasis(),
                            )
                            .on_first_line_ink(move |ink, _, _| title_lead.set(ink)),
                    ),
            )
    });
    v_flex()
        .debug_selector(|| "pecan-note".into())
        .refine_style(&typography)
        .relative()
        .w_full()
        .gap_2()
        .min_h(px(ICON_SIZE + 32.))
        .rounded(px(4.))
        .bg(style.fill)
        .border_1()
        .border_color(style.edge)
        .p(px(16.))
        .children(heading)
        .child(
            div()
                .debug_selector(|| "pecan-note-body-frame".into())
                .w_full()
                .min_w_0()
                .when(!titled, |body| body.pl(px(ICON_SIZE + 8.)))
                .child(
                    div()
                        .debug_selector(|| "pecan-note-body".into())
                        .w_full()
                        .min_w_0()
                        .child(body),
                ),
        )
        // The text has already reported its real first-row glyph extent. The
        // symbol is a native element placed in the gutter without changing
        // the Markdown's source, selection, wrapping, or vertical spacing.
        .child(
            canvas(
                move |bounds, window, cx| {
                    let center = lead.get().map_or(bounds.center().y, |ink| ink.center().y);
                    let mut icon = div()
                        .debug_selector(|| "pecan-note-icon".into())
                        .size(px(ICON_SIZE))
                        .child(
                            Icon::new(style.icon)
                                .size(px(ICON_SIZE))
                                .text_color(style.mark),
                        )
                        .into_any_element();
                    icon.layout_as_root(
                        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                        window,
                        cx,
                    );
                    icon.prepaint_at(
                        point(bounds.left(), center - px(ICON_SIZE / 2.)),
                        window,
                        cx,
                    );
                    icon
                },
                |_, mut icon, window, cx| icon.paint(window, cx),
            )
            .absolute()
            .left(px(16.))
            .top(px(16.))
            .size(px(ICON_SIZE)),
        )
}

pub(crate) fn extensions(base: MarkdownExtensions) -> MarkdownExtensions {
    let base = base.inline_code_emphasis();
    let nested = base.clone();
    base.plugin(Alerts { nested })
}

struct Alerts {
    nested: MarkdownExtensions,
}

struct Alert {
    kind: AlertKind,
    title: String,
    body: SharedString,
}

fn marker(source: &str) -> Option<(AlertKind, &str)> {
    let line = source.lines().next()?.trim_start();
    let content = line.strip_prefix('>')?.trim_start();
    alert_marker(content)
}

/// Drop the `[!TYPE]` line and one quote marker from the rest, leaving Markdown
/// the nested reader can parse as lists and paragraphs.
fn alert_body(source: &str) -> String {
    let mut lines = source.lines();
    let _ = lines.next();
    let mut body = String::new();
    for line in lines {
        let indent = line.bytes().take_while(|byte| *byte == b' ').count();
        let content = if indent <= 3
            && let Some(rest) = line[indent..].strip_prefix('>')
        {
            rest.strip_prefix(' ').unwrap_or(rest)
        } else {
            line
        };
        body.push_str(content);
        body.push('\n');
    }
    body
}

impl MarkdownPlugin for Alerts {
    fn is_block(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "pecan-alert"
    }
    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        if !matches!(node, markdown_ast::Node::Blockquote(_)) {
            return None;
        }
        let raw = cx.node_source(node)?;
        let (kind, title) = marker(raw)?;
        let body = alert_body(raw);
        let title = if title.is_empty() {
            kind.label().to_owned()
        } else {
            title.to_owned()
        };
        Some(
            MarkdownNode::new(
                self.name(),
                Alert {
                    kind,
                    title: title.clone(),
                    body: body.clone().into(),
                },
            )
            .text(format!("{title}\n{body}"))
            .markdown(raw.to_owned()),
        )
    }
    fn render(&self, node: &MarkdownNode, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(alert) = node.data::<Alert>() else {
            return div().into_any_element();
        };
        let palette = ReadingPalette::for_app(cx);
        let dark = cx.theme().is_dark();
        let (fill, edge, mark) = colors(alert.kind, dark);
        let style = crate::presentation::styles(cx).note;
        let index = node.source_range().map_or(0, |range| range.start);
        let body = alert.body.trim();
        let markdown = (!body.is_empty()).then(|| {
            TextView::markdown(("pecan-alert", index), alert.body.clone())
                .style(style)
                .markdown_extensions(self.nested.clone())
        });
        frame(
            NoteStyle {
                icon: icon(Some(alert.kind)),
                title: Some(alert.title.clone()),
                fill,
                edge,
                mark,
                ink: palette.ink,
                lead: None,
            },
            div().w_full().children(markdown),
            cx,
        )
        .into_any_element()
    }
}

/// Shared Lucide symbol for reading and document export. A generic callout uses the bulb.
pub(crate) fn icon(kind: Option<AlertKind>) -> IconName {
    match kind {
        None | Some(AlertKind::Tip) => IconName::Lightbulb,
        Some(AlertKind::Note | AlertKind::Info) => IconName::Info,
        Some(AlertKind::Important) => IconName::CircleAlert,
        Some(AlertKind::Warning) => IconName::TriangleAlert,
        Some(AlertKind::Caution) => IconName::Ban,
        Some(AlertKind::Error) => IconName::CircleX,
    }
}

use pecan_assets::notes::light_colors;
fn colors(kind: AlertKind, dark: bool) -> (Hsla, Hsla, Hsla) {
    let (fill, edge, mark) = if dark {
        match kind {
            AlertKind::Note | AlertKind::Info => (0x1c2838, 0x2c4564, 0x8eb6ef),
            AlertKind::Tip => (0x163024, 0x24543a, 0x7dcea0),
            AlertKind::Important => (0x2a1d38, 0x4a3470, 0xc4a6e8),
            AlertKind::Warning => (0x3a3226, 0x5c4b34, 0xf0c060),
            AlertKind::Caution | AlertKind::Error => (0x3a1e1e, 0x6a3030, 0xf0a0a0),
        }
    } else {
        light_colors(kind)
    };
    (rgb(fill).into(), rgb(edge).into(), rgb(mark).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn untitled_notes_put_rich_body_beside_the_icon(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext, Context, Entity, Render, base::TextViewState};
        struct Fixture {
            text: Entity<TextViewState>,
            lead: LeadInk,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div().w(px(640.)).child(frame(
                    NoteStyle {
                        icon: icon(None),
                        title: None,
                        fill: rgb(0xfff6e8).into(),
                        edge: rgb(0xf0e0c0).into(),
                        mark: rgb(0xc4841a).into(),
                        ink: rgb(0x242424).into(),
                        lead: Some(self.lead.clone()),
                    },
                    TextView::new(&self.text)
                        .style(crate::presentation::styles(cx).reading)
                        .selectable(true)
                        .on_first_line_ink({
                            let lead = self.lead.clone();
                            move |ink, _, _| lead.set(ink)
                        }),
                    cx,
                ))
            }
        }
        cx.update(gpui_kit::init);
        for source in [
            "`location_time` 取 SDK 返回的 `rtc_time` 字段值",
            "**参考文档**\n\n- 地理定位：[查看文档](https://example.com/location)\n- 反向地理编码：保留列表",
            "- **第一项**\n- 第二项 `code`",
        ] {
            let (root, visual) = cx.add_window_view(|_, cx| Fixture {
                text: cx.new(|cx| TextViewState::markdown(source, cx)),
                lead: Default::default(),
            });
            visual.run_until_parked();
            visual.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
            let icon = visual.debug_bounds("pecan-note-icon").expect("note icon");
            let body = visual.update(|_, cx| root.read(cx).text.read(cx).bounds());
            assert!(
                (body.left() - icon.right() - px(8.)).abs() <= px(0.5),
                "the rich body follows the icon with one 8px gutter: {source:?}, {icon:?}, {body:?}"
            );
            let ink = visual
                .update(|_, cx| root.read(cx).lead.get())
                .expect("first line glyphs");
            assert!(
                (ink.center().y - icon.center().y).abs() <= px(0.51),
                "symbol follows actual ink: {ink:?}, {icon:?}"
            );
            assert!(
                icon.center().y > body.top() && icon.center().y < body.top() + px(29.),
                "the icon aligns with the first line, not a separate heading or the whole body: {source:?}, {icon:?}, {body:?}"
            );
            visual.update(|_, cx| {
                root.read(cx).text.clone().update(cx, |text, cx| {
                    text.select_all(cx);
                    assert!(
                        !text.selected_text().is_empty(),
                        "rich Markdown stays selectable"
                    );
                    assert!(
                        text.rendered_text()
                            .as_str()
                            .contains(if source.starts_with('`') {
                                "location_time"
                            } else {
                                "第一项"
                            })
                            || text.rendered_text().as_str().contains("参考文档")
                    );
                });
            });
        }
    }

    #[gpui_kit::test]
    fn note_titles_do_not_inherit_different_container_sizes(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{Context, Render};
        struct Fixture {
            inherited_size: f32,
            lead: LeadInk,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .text_size(px(self.inherited_size))
                    .line_height(px(self.inherited_size * 1.5))
                    .child(frame(
                        NoteStyle {
                            icon: icon(None),
                            title: Some("标题 Title".into()),
                            fill: rgb(0xffffff).into(),
                            edge: rgb(0xeeeeee).into(),
                            mark: rgb(0x444444).into(),
                            ink: rgb(0x222222).into(),
                            lead: Some(self.lead.clone()),
                        },
                        div().child("正文"),
                        cx,
                    ))
            }
        }
        cx.update(gpui_kit::init);
        let (root, visual) = cx.add_window_view(|_, _| Fixture {
            inherited_size: 14.,
            lead: Default::default(),
        });
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let first_ink = visual
            .update(|_, cx| root.read(cx).lead.get())
            .expect("title glyphs");
        let first = visual
            .debug_bounds("pecan-note-title")
            .expect("first title")
            .size;
        root.update(visual, |root, cx| {
            root.inherited_size = 22.;
            cx.notify();
        });
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let second = visual
            .debug_bounds("pecan-note-title")
            .expect("second title")
            .size;
        assert_eq!(
            first, second,
            "note typography must be identical in reading and interface containers"
        );
        visual.update(|_, cx| {
            crate::preferences::Preferences::shared(cx).update(cx, |prefs, cx| {
                prefs.change(|data| data.font_size = 24., cx)
            });
        });
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        let enlarged = visual
            .debug_bounds("pecan-note-title")
            .expect("resized title")
            .size;
        assert!(
            visual
                .update(|_, cx| root.read(cx).lead.get())
                .expect("resized glyphs")
                .size
                .height
                > first_ink.size.height
                && enlarged.height >= first.height,
            "note titles follow the body font size setting"
        );
        assert_eq!(
            visual.debug_bounds("pecan-note-icon").expect("icon").size,
            gpui_kit::size(px(20.), px(20.)),
            "note icons stay at 20px"
        );
    }

    #[gpui_kit::test]
    fn rendered_alert_is_a_colored_note_not_a_quote(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext, Context, Entity, Render, base::TextViewState};
        struct Fixture {
            text: Entity<TextViewState>,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                TextView::new(&self.text)
                    .markdown_extensions(extensions(MarkdownExtensions::default()))
            }
        }
        cx.update(gpui_kit::init);
        let source = "> [!NOTE]\n> **说明**是蓝色。\n\n> [!ERROR]\n> 已经出错。\n";
        let (root, visual) = cx.add_window_view(|_, cx| Fixture {
            text: cx.new(|cx| TextViewState::markdown(source, cx)),
        });
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        assert!(
            visual.debug_bounds("pecan-note").is_some(),
            "a bold sentence after the marker should still be a colored note"
        );
        let icon = visual
            .debug_bounds("pecan-note-icon")
            .expect("note icon bounds");
        let header = visual
            .debug_bounds("pecan-note-heading")
            .expect("note header bounds");
        assert_eq!(icon.size, gpui_kit::size(px(20.), px(20.)));
        assert!(
            icon.center().y >= header.top() && icon.center().y <= header.bottom(),
            "the symbol aligns with the title row; glyph alignment is covered by the observed title fixture: {icon:?}, {header:?}"
        );
        let rendered = visual.update(|_, cx| {
            root.read(cx)
                .text
                .read(cx)
                .rendered_text()
                .as_str()
                .to_owned()
        });
        assert!(
            !rendered.contains("[!NOTE]"),
            "marker stays visible in the quote: {rendered}"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "Requires the native Metal renderer and installed system fonts"]
    fn native_note_ink_alignment_renders_with_real_fonts() {
        use gpui_kit::{
            AppContext, Context, Entity, HeadlessAppContext, Render, WindowBounds, WindowOptions,
            base::TextViewState,
        };
        use std::sync::Arc;
        struct Fixture {
            text: Entity<TextViewState>,
            title: Option<String>,
            lead: LeadInk,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let dark = cx.theme().is_dark();
                let (fill, edge, mark) = colors(AlertKind::Note, dark);
                let lead = self.lead.clone();
                let untitled = self.title.is_none();
                div()
                    .size_full()
                    .bg(cx.theme().background)
                    .p(px(24.))
                    .child(frame(
                        NoteStyle {
                            icon: icon(Some(AlertKind::Note)),
                            title: self.title.clone(),
                            fill,
                            edge,
                            mark,
                            ink: ReadingPalette::for_app(cx).ink,
                            lead: Some(self.lead.clone()),
                        },
                        TextView::new(&self.text)
                            .style(crate::presentation::styles(cx).note)
                            .markdown_extensions(extensions(MarkdownExtensions::default()))
                            .on_first_line_ink(move |ink, _, _| {
                                if untitled {
                                    lead.set(ink);
                                }
                            }),
                        cx,
                    ))
            }
        }
        for (index, title, body, font_size, width) in [
            (
                0,
                None,
                "业务方云端服务接收到各自终端组件上报的定位数据后，需统一数据结构后投递到 RMQ 中",
                16.,
                840.,
            ),
            (
                1,
                Some("安心阅读本地文件"),
                "碧果在本机解析和转换文档，不改写源文件。",
                16.,
                840.,
            ),
            (
                2,
                Some("**中文标题** English `version`"),
                "**加粗**、*斜体*、~~删除线~~。",
                24.,
                420.,
            ),
            (
                3,
                None,
                "English text with **bold**, *italic*, ~~removed~~ and `code` beside a symbol.",
                16.,
                840.,
            ),
        ] {
            let mut headless = HeadlessAppContext::with_platform(
                Arc::new(gpui_apple::AppleTextSystem::new()),
                Arc::new(crate::assets::ReaderAssets),
                gpui_platform::current_headless_renderer,
            );
            headless.allow_parking();
            let (handle, fixture) = headless.update(|cx| {
                gpui_kit::init(cx);
                crate::fonts::install(cx);
                crate::preferences::Preferences::install(None, cx).update(cx, |prefs, cx| {
                    prefs.change(
                        |data| {
                            data.font_size = font_size;
                            data.theme = "light".into();
                        },
                        cx,
                    )
                });
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(0.), px(0.)),
                            size(px(width), px(400.)),
                        ))),
                        ..Default::default()
                    },
                    cx,
                    |_, cx| {
                        cx.new(|cx| Fixture {
                            text: cx.new(|cx| TextViewState::markdown(body, cx)),
                            title: title.map(str::to_owned),
                            lead: Default::default(),
                        })
                    },
                )
                .unwrap()
            });
            for _ in 0..3 {
                headless.run_until_parked();
                headless
                    .update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                    .unwrap();
            }
            let image = headless
                .update_window(handle, |_, window, cx| {
                    let ink = fixture.read(cx).lead.get().expect("native glyphs measured");
                    assert!(ink.size.height > px(7.), "native glyph extent: {ink:?}");
                    window
                        .render_to_image()
                        .expect("native text and symbols rasterized")
                })
                .unwrap();
            if let Some(directory) = std::env::var_os("PECAN_NOTE_OUTPUT") {
                image
                    .save(
                        std::path::PathBuf::from(directory)
                            .join(format!("native-note-{index}.png")),
                    )
                    .unwrap();
            }
        }
    }

    #[test]
    fn blockquote_marker_is_plain_text() {
        let source = "> [!NOTE]\n> 说明是蓝色。\n\n> [!TIP]\n> 技巧\n>\n> - 甲\n";
        let ast = markdown::to_mdast(source, &markdown::ParseOptions::gfm()).expect("parse");
        let markdown_ast::Node::Root(root) = &ast else {
            panic!("root")
        };
        let markdown_ast::Node::Blockquote(quote) = &root.children[0] else {
            panic!("quote")
        };
        assert!(!quote.children.is_empty());
        assert!(marker(source).is_some(), "marker was not recognized");
        let rich = "> [!NOTE] **标题** *斜体* ~~旧版~~ `code`\n> 正文";
        assert_eq!(
            marker(rich),
            Some((AlertKind::Note, "**标题** *斜体* ~~旧版~~ `code`")),
            "custom title keeps its Markdown source"
        );
    }

    #[test]
    fn alert_body_keeps_lists_after_the_marker_line() {
        let source = "> [!WARNING] 注意\n> 第一段\n>\n> - 甲\n> - 乙\n";
        assert_eq!(alert_body(source), "第一段\n\n- 甲\n- 乙\n");
    }
}
