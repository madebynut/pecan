//! Rich Markdown inside GFM cells separated with HTML line breaks.
use crate::presentation::ReadingPalette;
use gpui_kit::{
    App, Axis, IntoElement, ScrollHandle, SharedString, Window,
    base::{RoundedFrameCover, ScrollableMask, TextView},
    component::text::{
        MarkdownExtensions, MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast,
    },
    div,
    prelude::*,
    px,
};

pub(crate) fn extensions(base: MarkdownExtensions) -> MarkdownExtensions {
    let base = base.inline_code_emphasis();
    let nested = base.clone();
    base.plugin(RichTables { nested })
}

struct RichTables {
    nested: MarkdownExtensions,
}
struct Table {
    rows: Vec<Vec<SharedString>>,
    align: Vec<markdown_ast::AlignKind>,
}

/// Replace only actual HTML nodes, so `<br>` inside an inline code capsule is preserved.
fn cell_markdown(node: &markdown_ast::Node, source: &str) -> Option<String> {
    let markdown_ast::Node::TableCell(cell) = node else {
        return None;
    };
    let origin = cell.children.first()?.position()?.start.offset;
    let mut edits = Vec::new();
    let mut ordered = None;
    for child in &cell.children {
        let markdown_ast::Node::Html(html) = child else {
            continue;
        };
        let tag = html.value.trim().to_ascii_lowercase();
        let replacement = match tag.as_str() {
            "<br>" | "<br/>" | "<br />" => "  \n".to_string(),
            "<ul>" | "</ul>" | "</ol>" => "\n".to_string(),
            "<ol>" => {
                ordered = Some(1);
                "\n".to_string()
            }
            "<li>" => {
                if let Some(number) = &mut ordered {
                    let marker = format!("{number}. ");
                    *number += 1;
                    marker
                } else {
                    "- ".to_string()
                }
            }
            "</li>" => "\n".to_string(),
            _ => continue,
        };
        let position = child.position()?;
        edits.push((
            position.start.offset - origin..position.end.offset - origin,
            replacement,
        ));
    }
    if edits.is_empty() {
        return None;
    }
    let mut source = source
        .get(origin..cell.children.last()?.position()?.end.offset)?
        .to_string();
    for (range, replacement) in edits.into_iter().rev() {
        source.replace_range(range, &replacement);
    }
    Some(source.trim().to_string())
}

impl MarkdownPlugin for RichTables {
    fn name(&self) -> &str {
        "pecan-rich-table"
    }
    fn is_block(&self) -> bool {
        true
    }
    fn parse(
        &self,
        node: &markdown_ast::Node,
        context: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Table(table) = node else {
            return None;
        };
        // Keep ordinary inline-only tables on the native fast path, without
        // copying every cell just to discover that no block content is present.
        let has_blocks = table.children.iter().any(|row| {
            let markdown_ast::Node::TableRow(row) = row else { return false };
            row.children.iter().any(|cell| {
                let markdown_ast::Node::TableCell(cell) = cell else { return false };
                cell.children.iter().any(|child| matches!(child, markdown_ast::Node::Html(html) if matches!(html.value.trim().to_ascii_lowercase().as_str(), "<br>" | "<br/>" | "<br />" | "<ul>" | "<ol>" | "<li>")))
            })
        });
        if !has_blocks {
            return None;
        }
        let mut rich = false;
        let rows: Vec<Vec<SharedString>> = table
            .children
            .iter()
            .filter_map(|row| {
                let markdown_ast::Node::TableRow(row) = row else {
                    return None;
                };
                Some(
                    row.children
                        .iter()
                        .map(|cell| {
                            if let Some(source) = cell_markdown(cell, context.source()) {
                                rich = true;
                                source.into()
                            } else {
                                cell_source(cell, context.source())
                                    .unwrap_or_default()
                                    .trim()
                                    .to_string()
                                    .into()
                            }
                        })
                        .collect(),
                )
            })
            .collect();
        if !rich {
            return None;
        }
        let source = context.node_source(node).unwrap_or_default().to_string();
        Some(
            MarkdownNode::new(
                self.name(),
                Table {
                    rows,
                    align: table.align.clone(),
                },
            )
            .text(source.clone())
            .markdown(source),
        )
    }
    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = ReadingPalette::for_app(cx);
        let style = crate::presentation::styles(cx).reading;
        let index = node.source_range().map_or(0, |range| range.start);
        let scroll = window
            .use_keyed_state(("rich-table-scroll", index), cx, |_, _| {
                ScrollHandle::default()
            })
            .read(cx)
            .clone();
        let mut body = div()
            .grid()
            .flex_shrink_0()
            .min_w_full()
            .debug_selector(|| "rich-table-track".into());
        if let Some(table) = node.data::<Table>() {
            let cols = table.align.len().max(1);
            body = body.grid_cols_max_content_stretch_last(cols as u16);
            for (row_index, row) in table.rows.iter().enumerate() {
                for col in 0..cols {
                    let source = row.get(col).cloned().unwrap_or_default();
                    body = body.child(
                        div()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .whitespace_nowrap()
                            .px_4()
                            .py_3()
                            .debug_selector(move || format!("rich-cell-{row_index}-{col}"))
                            .when(row_index > 0, |cell| {
                                cell.border_t_1().border_color(palette.border)
                            })
                            .when(col > 0, |cell| {
                                cell.border_l_1().border_color(palette.border)
                            })
                            .when(row_index == 0, |cell| {
                                cell.bg(palette.wash)
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            })
                            .when(
                                table.align.get(col) == Some(&markdown_ast::AlignKind::Center),
                                |cell| cell.text_center(),
                            )
                            .when(
                                table.align.get(col) == Some(&markdown_ast::AlignKind::Right),
                                |cell| cell.text_right(),
                            )
                            .child(
                                div()
                                    .debug_selector(move || {
                                        format!("rich-content-{row_index}-{col}")
                                    })
                                    .child(
                                        TextView::markdown(
                                            ("cell", row_index * cols + col),
                                            source,
                                        )
                                        .style(style.clone())
                                        .markdown_extensions(self.nested.clone()),
                                    ),
                            ),
                    );
                }
            }
        }
        // A non-shrinking flex child gives the grid its max-content width.
        // Keep the gesture mask outside the moving track so vertical wheel
        // input still reaches the document's virtual list.
        div()
            .id(("rich-table", index))
            .w_full()
            .relative()
            .border_1()
            .border_color(palette.border)
            .rounded(px(3.))
            .child(
                div()
                    .id("rich-table-viewport")
                    .debug_selector(|| "rich-table-viewport".into())
                    .w_full()
                    .flex()
                    .overflow_hidden()
                    .track_scroll(&scroll)
                    .child(body),
            )
            .child(ScrollableMask::new(Axis::Horizontal, &scroll).id("rich-table-wheel"))
            .child(RoundedFrameCover::uniform(px(3.)).frame_border(px(1.)))
    }
}

fn cell_source<'a>(node: &markdown_ast::Node, source: &'a str) -> Option<&'a str> {
    let markdown_ast::Node::TableCell(cell) = node else {
        return None;
    };
    source.get(
        cell.children.first()?.position()?.start.offset
            ..cell.children.last()?.position()?.end.offset,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{preferences::Preferences, presentation::reading_style};

    #[gpui_kit::test]
    fn table_rows_keep_explicit_breaks_without_width_induced_wrapping(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::{AppContext, Context, Entity, Render, base::TextViewState};
        struct Fixture {
            text: Entity<TextViewState>,
            width: f32,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let prefs = Preferences::shared(cx).read(cx).data.clone();
                div()
                    .w(px(self.width))
                    .text_size(px(16.))
                    .line_height(px(28.8))
                    .child(
                        TextView::new(&self.text)
                            .style(reading_style(&prefs, ReadingPalette::for_app(cx), cx))
                            .markdown_extensions(extensions(MarkdownExtensions::default())),
                    )
            }
        }
        cx.update(gpui_kit::init);
        let long = "long unwrapped value with 中文 and `code` ".repeat(10);
        let sources = [
            format!("| Key | Value |\n| --- | --- |\n| id | {long} |\n| status | ready |\n"),
            format!(
                "| Key | Value |\n| --- | --- |\n| id | {long}<br>next line |\n| list | <ul><li>first item</li><li>second item</li></ul> |\n"
            ),
        ];
        for source in sources {
            let (root, visual) = cx.add_window_view(|_, cx| Fixture {
                text: cx.new(|cx| TextViewState::markdown(&source, cx)),
                width: 4000.,
            });
            let mut heights = Vec::new();
            for width in [4000., 420., 700., 4000.] {
                visual.update(|window, cx| {
                    root.update(cx, |view, _| view.width = width);
                    window.refresh();
                    window.draw(cx).clear(cx);
                });
                for _ in 0..5 {
                    visual.run_until_parked();
                    visual.update(|window, cx| {
                        window.refresh();
                        window.draw(cx).clear(cx);
                    });
                }
                heights
                    .push(visual.update(|_, cx| root.read(cx).text.read(cx).bounds().size.height));
                if let Some(viewport) = visual.debug_bounds("rich-table-viewport") {
                    let track = visual.debug_bounds("rich-table-track").unwrap();
                    let header = visual.debug_bounds("rich-cell-0-0").unwrap();
                    let first = visual.debug_bounds("rich-cell-1-0").unwrap();
                    let second = visual.debug_bounds("rich-cell-1-1").unwrap();
                    let first_content = visual.debug_bounds("rich-content-1-0").unwrap();
                    let second_content = visual.debug_bounds("rich-content-1-1").unwrap();
                    assert!(
                        (first_content.center().y - first.center().y).abs() <= px(1.),
                        "short table value vertically centers in its tall row: {first_content:?} / {first:?}"
                    );
                    assert!(
                        (second_content.center().y - second.center().y).abs() <= px(1.),
                        "multiline table value vertically centers as one block"
                    );
                    assert_eq!(
                        header.size.width, first.size.width,
                        "rows share column widths"
                    );
                    assert_eq!(
                        header.left(),
                        first.left(),
                        "column boundaries stay aligned"
                    );
                    assert!(
                        first.size.width < second.size.width / 2.,
                        "short columns fit their content"
                    );
                    if width == 420. {
                        assert!(
                            track.size.width > viewport.size.width,
                            "the complete track must overflow horizontally"
                        );
                        let position = viewport.center();
                        visual.simulate_event(gpui_kit::ScrollEvent {
                            position,
                            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                                px(-100_000.),
                                px(0.),
                            )),
                            ..Default::default()
                        });
                        visual.update(|window, cx| {
                            window.refresh();
                            window.draw(cx).clear(cx);
                        });
                        let rightmost = visual.debug_bounds("rich-table-track").unwrap();
                        assert!(
                            (rightmost.right() - viewport.right()).abs() <= px(1.),
                            "the last column must be fully reachable: {rightmost:?} in {viewport:?}"
                        );
                        visual.simulate_event(gpui_kit::ScrollEvent {
                            position,
                            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                                px(100_000.),
                                px(0.),
                            )),
                            ..Default::default()
                        });
                    }
                }
            }
            assert!(
                heights[0] > px(60.),
                "table rows must render before measurement: {heights:?}"
            );
            assert!(
                heights
                    .iter()
                    .all(|height| (*height - heights[0]).abs() < px(1.)),
                "table height must only follow explicit line breaks, not viewport width: {heights:?}"
            );
        }
    }

    #[gpui_kit::test]
    fn short_table_fills_width_and_preserves_three_explicit_lines(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::{AppContext, Context, Entity, Render, base::TextViewState};
        struct Fixture {
            text: Entity<TextViewState>,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let prefs = Preferences::shared(cx).read(cx).data.clone();
                div()
                    .w(px(700.))
                    .text_size(px(16.))
                    .line_height(px(28.8))
                    .child(
                        TextView::new(&self.text)
                            .style(reading_style(&prefs, ReadingPalette::for_app(cx), cx))
                            .markdown_extensions(extensions(MarkdownExtensions::default())),
                    )
            }
        }
        cx.update(gpui_kit::init);
        let (_, visual) = cx.add_window_view(|_, cx| Fixture {
            text: cx.new(|cx| {
                TextViewState::markdown(
                    "| 模块 | 内容 |\n| --- | --- |\n| 简短 | 第一行<br>第二行<br>第三行 |\n",
                    cx,
                )
            }),
        });
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
        }
        let viewport = visual.debug_bounds("rich-table-viewport").unwrap();
        let track = visual.debug_bounds("rich-table-track").unwrap();
        let last_header = visual.debug_bounds("rich-cell-0-1").unwrap();
        let last_cell = visual.debug_bounds("rich-cell-1-1").unwrap();
        let content = visual.debug_bounds("rich-content-1-1").unwrap();
        let short = visual.debug_bounds("rich-content-1-0").unwrap();
        assert!((track.right() - viewport.right()).abs() < px(1.));
        assert!(
            (last_header.right() - viewport.right()).abs() < px(1.),
            "header fills remaining width"
        );
        assert_eq!(last_cell.right(), last_header.right());
        assert!(
            content.size.height > short.size.height * 2.5,
            "three explicit lines must not be folded into one: {content:?}, short {short:?}"
        );
        assert!(
            (short.center().y - last_cell.center().y).abs() <= px(1.),
            "short labels are vertically centered"
        );
    }

    #[test]
    fn table_breaks_become_markdown_lists_without_changing_inline_code() {
        let source = "| 模块 | 说明 |\n|---|---|\n| 服务 | * **第一项** `code`；<br> <br>* 第二项 [链接](https://example.com) |\n| 标签 | `<br>`<br>下一行 |\n| HTML | <ol><li>甲</li><li>乙</li></ol> |\n";
        let root = markdown::to_mdast(source, &markdown::ParseOptions::gfm()).unwrap();
        let markdown_ast::Node::Root(root) = root else {
            panic!("root")
        };
        let markdown_ast::Node::Table(table) = &root.children[0] else {
            panic!("table")
        };
        let cell = |row: usize| {
            let markdown_ast::Node::TableRow(row) = &table.children[row] else {
                panic!("row")
            };
            cell_markdown(&row.children[1], source).unwrap()
        };
        let list = cell(1);
        let parsed = markdown::to_mdast(&list, &markdown::ParseOptions::gfm()).unwrap();
        let markdown_ast::Node::Root(parsed) = parsed else {
            panic!("root")
        };
        let markdown_ast::Node::List(list) = &parsed.children[0] else {
            panic!("cell must contain a real list, {parsed:?}")
        };
        assert_eq!(list.children.len(), 2);
        assert!(cell(2).contains("`<br>`  \n下一行"));
        assert_eq!(cell(3), "1. 甲\n2. 乙");
    }
}
