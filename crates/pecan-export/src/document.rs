//! Reuse Markdown semantics and the existing renderers without constructing reading rows.
use super::{Snapshot, images::Images, note_icons::Icons};
use anyhow::{Context as _, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use http_client::HttpClient;
use pecan_document::AlertKind;
use pecan_math::{MathFont, MathStyle, Presentation};
use pecan_mermaid::Appearance;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::{
    borrow::Cow,
    collections::HashSet,
    fmt::Write as _,
    path::Path,
    sync::{Arc, LazyLock},
};

pub(super) const MAX_OUTPUT: usize = 128 * 1024 * 1024;
pub(super) const MAX_NOTE_DEPTH: usize = 32;

pub(super) fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH
        | Options::ENABLE_HEADING_ATTRIBUTES
}

pub(super) enum NoteMarker {
    Open {
        kind: Option<AlertKind>,
        titled: bool,
    },
    Close,
}

pub(super) fn highlight_edge(raw: &str) -> Option<NoteMarker> {
    let (_, tail) = raw.split_once("class=\"")?;
    let (classes, _) = tail.split_once('"')?;
    let has = |class| classes.split_ascii_whitespace().any(|token| token == class);
    if has("pecan-highlight-end") {
        Some(NoteMarker::Close)
    } else if has("pecan-highlight") {
        let kind = [
            AlertKind::Note,
            AlertKind::Info,
            AlertKind::Tip,
            AlertKind::Important,
            AlertKind::Warning,
            AlertKind::Caution,
            AlertKind::Error,
        ]
        .into_iter()
        .find(|kind| has(kind.class()));
        Some(NoteMarker::Open {
            kind,
            titled: kind.is_some() || has("pecan-highlight-titled"),
        })
    } else {
        None
    }
}

/// Only the first generated title prefix is replaced; body and source emoji stay literal.
pub(super) fn note_text<'a>(text: &'a str, pending: &mut Option<AlertKind>) -> &'a str {
    pending
        .take()
        .and_then(|kind| text.strip_prefix(kind.icon()))
        .map_or(text, |tail| tail.strip_prefix(' ').unwrap_or(tail))
}

#[cfg(test)]
pub(super) const NOTE_FIXTURE: &str = "> [!NOTE] 💡 自定义\n> 保留 ℹ️ 正文。\n\n> [!NOTE]\n> 第二条提示。\n\n:::\n💡 提示正文 `ℹ️`\n:::\n\n```text\n⚠️ 💡 ℹ️ ❗ 🚫 ❌\n```\n";

/// Markers are empty divs around the note. Open the first and close the second
/// so the paragraphs, including lists, sit inside the highlight.
fn wrap_highlights(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(offset) = html[cursor..].find("<div class=\"pecan-highlight") {
        let start = cursor + offset;
        out.push_str(&html[cursor..start]);
        let tail = &html[start..];
        let Some(tag_end) = tail.find('>') else {
            out.push_str(tail);
            return out;
        };
        let opening = &tail[..=tag_end];
        if tail[tag_end + 1..].starts_with("</div>") {
            let marker_len = opening.len() + "</div>".len();
            let marker = &tail[..marker_len];
            match highlight_edge(marker) {
                Some(NoteMarker::Open { .. }) => out.push_str(opening),
                Some(NoteMarker::Close) => out.push_str("</div>"),
                None => out.push_str(marker),
            }
            cursor = start + marker_len;
        } else {
            out.push_str(opening);
            cursor = start + opening.len();
        }
    }
    out.push_str(&html[cursor..]);
    out
}

pub(super) fn export_markdown(source: &str) -> std::borrow::Cow<'_, str> {
    let alerts = pecan_document::expand_alerts(source);
    if !alerts.contains(":::") {
        return alerts;
    }
    std::borrow::Cow::Owned(pecan_document::expand_callouts(alerts.as_ref()).into_owned())
}

pub(super) fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn highlighted_html(source: &str, language: &str) -> Result<String> {
    let mut output = String::with_capacity(source.len() * 2);
    for (range, ink) in super::code::spans(source, language) {
        let text = escape(&source[range]);
        if ink == super::code::Ink::default() {
            output.push_str(&text);
            continue;
        }
        output.push_str("<span style=\"");
        if let Some([red, green, blue]) = ink.rgb {
            write!(output, "color:#{red:02x}{green:02x}{blue:02x};")?;
        }
        if ink.bold {
            output.push_str("font-weight:600;");
        }
        if ink.italic {
            output.push_str("font-style:italic;");
        }
        write!(output, "\">{text}</span>")?;
    }
    Ok(output)
}

/// Inline code accepts emphasis only; block syntax, links and HTML remain literal.
/// Most identifiers take the allocation-light literal path.
pub(super) fn code_events(text: &str) -> Vec<Event<'_>> {
    let literal = || vec![Event::Text(text.into())];
    if !text.contains(['*', '_', '~']) {
        return literal();
    }
    let core = text.trim_matches(' ');
    let leading = text.len() - text.trim_start_matches(' ').len();
    let trailing = text.len() - text.trim_end_matches(' ').len();
    let mut events: Vec<_> = Parser::new_ext(core, Options::ENABLE_STRIKETHROUGH).collect();
    if !matches!(events.first(), Some(Event::Start(Tag::Paragraph)))
        || !matches!(events.last(), Some(Event::End(TagEnd::Paragraph)))
        || !events.iter().any(|event| {
            matches!(
                event,
                Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough)
            )
        })
        || events[1..events.len() - 1].iter().any(|event| {
            !matches!(
                event,
                Event::Text(_)
                    | Event::SoftBreak
                    | Event::HardBreak
                    | Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough)
                    | Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough)
            )
        })
    {
        return literal();
    }
    events.pop();
    events.remove(0);
    if leading > 0 {
        events.insert(0, Event::Text(text[..leading].into()));
    }
    if trailing > 0 {
        events.push(Event::Text(text[text.len() - trailing..].into()));
    }
    events
}

/// Complete HTML with only embedded resources, no scripts or external style sheets.
pub(super) async fn html(snapshot: &Snapshot, client: Arc<dyn HttpClient>) -> Result<String> {
    let mut images = Images::new(&snapshot.directory, client);
    let mut icons = Icons::default();
    let mut language_icons = super::language_icons::Icons::default();
    let mut formats = super::code::Formats::new(
        snapshot.codes.clone(),
        snapshot.preferences.code_tab_size.clone(),
    );
    let mut note_heading = None;
    let mut note_prefix = None;
    let mut note_title_open = false;
    let mut events = Vec::new();
    let mut inserts = Vec::new();
    let source = export_markdown(snapshot.document.source());
    // Cells with lists, tasks or breaks arrive as block events, as in Word and PDF.
    let mut parser = super::table::events(source.as_ref()).peekable();
    let mut insert_bytes = 0;
    let mut event_bytes = 0;
    // Raw HTML is cleaned as one fragment so nested tables/links remain well formed.
    // Trusted renderer output is inserted afterwards, keeping its SVG/CSS intact.
    let insert = |html: String,
                  events: &mut Vec<Event<'_>>,
                  inserts: &mut Vec<String>,
                  bytes: &mut usize|
     -> Result<()> {
        *bytes += html.len();
        ensure!(*bytes <= MAX_OUTPUT, "HTML 超过 128 MiB 导出预算");
        events.push(Event::Html(
            format!("<span data-pecan-export=\"{}\"></span>", inserts.len()).into(),
        ));
        inserts.push(html);
        Ok(())
    };
    while let Some(event) = parser.next() {
        match event {
            Event::Start(Tag::Paragraph) => {
                events.push(Event::Start(Tag::Paragraph));
                if let Some(kind) = note_heading.take() {
                    insert(
                        format!(
                            "<span class=\"note-heading\">{}<span class=\"note-heading-text\">",
                            icons.html(kind)?
                        ),
                        &mut events,
                        &mut inserts,
                        &mut insert_bytes,
                    )?;
                    note_title_open = true;
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if note_title_open {
                    insert(
                        "</span></span>".into(),
                        &mut events,
                        &mut inserts,
                        &mut insert_bytes,
                    )?;
                    note_title_open = false;
                }
                events.push(Event::End(TagEnd::Paragraph));
            }
            Event::Code(text) => {
                let mut code = String::from("<code>");
                pulldown_cmark::html::push_html(&mut code, code_events(&text).into_iter());
                code.push_str("</code>");
                insert(code, &mut events, &mut inserts, &mut insert_bytes)?;
            }
            Event::Text(text) => {
                // Tight lists have no paragraph event, but keep a note mark in their first item.
                if let Some(kind) = note_heading.take() {
                    insert(
                        icons.html(kind)?,
                        &mut events,
                        &mut inserts,
                        &mut insert_bytes,
                    )?;
                    events.push(Event::Text(" ".into()));
                }
                let content = note_text(&text, &mut note_prefix);
                let text = if content.len() == text.len() {
                    text
                } else {
                    content.to_owned().into()
                };
                events.push(Event::Text(text));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_owned()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                let mut code = String::new();
                for event in parser.by_ref() {
                    match event {
                        Event::End(TagEnd::CodeBlock) => break,
                        Event::Text(text) => code.push_str(&text),
                        _ => {}
                    }
                }
                let code = code.strip_suffix('\n').unwrap_or(&code);
                let html = if language.eq_ignore_ascii_case("mermaid") {
                    let _permit = snapshot.diagrams.slots.acquire().await;
                    let diagram = snapshot
                        .diagrams
                        .renderer
                        .render_with_fonts(
                            code,
                            Appearance::Light,
                            &snapshot.preferences.diagram_fonts(),
                        )
                        .context("导出 Mermaid")?;
                    let svg = STANDARD.encode(diagram.svg());
                    format!(
                        "<figure class=\"diagram\"><img alt=\"Mermaid 图表\" src=\"data:image/svg+xml;base64,{svg}\"></figure>"
                    )
                } else {
                    let formatted = formats.resolve(&language, code).await;
                    let code = formatted.as_ref().map_or(code, |output| output.text());
                    let code = highlighted_html(code, &language)?;
                    let logo = language_icons.html(&language);
                    format!(
                        "<section class=\"code-block\"><div class=\"code-language\">{logo}<span>{}</span></div><pre><code>{code}</code></pre></section>",
                        escape(if language.is_empty() {
                            "text"
                        } else {
                            &language
                        })
                    )
                };
                insert(html, &mut events, &mut inserts, &mut insert_bytes)?;
            }
            Event::InlineMath(ref source) | Event::DisplayMath(ref source) => {
                let style = if matches!(event, Event::DisplayMath(_)) {
                    MathStyle::Display
                } else {
                    MathStyle::Inline
                };
                let _permit = snapshot.math.slots.acquire().await;
                let formula = snapshot
                    .math
                    .renderer
                    .render(
                        source,
                        Presentation {
                            font: if snapshot.preferences.math_font == "modern" {
                                MathFont::LatinModern
                            } else {
                                MathFont::StixTwo
                            },
                            style,
                            font_size: snapshot.preferences.math_size.round() as u8,
                            color: 0x242424,
                        },
                    )
                    .context("导出 LaTeX")?;
                let (width, height) = formula.dimensions();
                let svg = STANDARD.encode(formula.svg());
                let image = format!(
                    "<img class=\"formula\" alt=\"{}\" width=\"{width}\" height=\"{height}\" style=\"vertical-align:-{}px\" src=\"data:image/svg+xml;base64,{svg}\">",
                    escape(formula.speech()),
                    height - formula.baseline()
                );
                let html = if style == MathStyle::Display {
                    format!("<div class=\"display-math\">{image}</div>")
                } else {
                    image
                };
                insert(html, &mut events, &mut inserts, &mut insert_bytes)?;
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let embedded = images.embed(&dest_url).await?;
                events.push(Event::Start(Tag::Image {
                    link_type,
                    dest_url: embedded.into(),
                    title,
                    id,
                }));
            }
            Event::Html(raw) | Event::InlineHtml(raw) => {
                let mut generic_icon = false;
                if let Some(marker) = highlight_edge(&raw) {
                    match marker {
                        NoteMarker::Open { kind, titled } => {
                            note_heading = titled.then_some(kind);
                            note_prefix = kind;
                            generic_icon = kind.is_none() && !titled;
                        }
                        NoteMarker::Close => {
                            note_heading = None;
                            note_prefix = None;
                        }
                    }
                }
                let raw = rewrite_html_images(&raw, &mut images, None).await?;
                events.push(Event::Html(raw.into()));
                // Keep the generic gutter mark even when the body is only code or an image.
                if generic_icon {
                    insert(
                        icons.html(None)?,
                        &mut events,
                        &mut inserts,
                        &mut insert_bytes,
                    )?;
                }
            }
            other => events.push(other),
        }
        if let Some(event) = events.last() {
            event_bytes += match event {
                Event::Text(text)
                | Event::Html(text)
                | Event::InlineHtml(text)
                | Event::Code(text) => text.len(),
                Event::Start(
                    Tag::Image {
                        dest_url, title, ..
                    }
                    | Tag::Link {
                        dest_url, title, ..
                    },
                ) => dest_url.len() + title.len(),
                _ => 32,
            };
        }
        ensure!(
            event_bytes + insert_bytes <= MAX_OUTPUT,
            "HTML 超过 128 MiB 导出预算"
        );
    }
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, events.into_iter());
    // The markers are empty divs around the note. Turn them into one wrapper so the
    // paragraphs sit inside the highlight instead of beside two empty tags.
    let body = wrap_highlights(&body);
    // pulldown-cmark writes column alignment as a style. The sanitizer drops
    // every style and already allows `align` on cells, so translate that one
    // attribute first. Any other declaration is left for the sanitizer.
    let body = cell_alignment(&body);
    ensure!(
        body.len() + insert_bytes <= MAX_OUTPUT,
        "HTML 超过 128 MiB 导出预算"
    );
    let mut clean = ammonia::Builder::default();
    clean
        .add_tags(["input"])
        .add_tag_attributes("input", ["type", "disabled", "checked"])
        .add_allowed_classes(
            "div",
            [
                "footnote-definition",
                "pecan-highlight",
                "pecan-highlight-titled",
                "pecan-note",
                "pecan-info",
                "pecan-tip",
                "pecan-important",
                "pecan-warning",
                "pecan-caution",
                "pecan-error",
            ],
        )
        .add_tag_attributes("span", ["data-pecan-export"])
        .add_tag_attributes("th", ["align"])
        .add_tag_attributes("td", ["align"])
        .add_generic_attributes(["id"])
        .url_schemes(HashSet::from(["http", "https", "mailto", "data"]))
        .url_relative(ammonia::UrlRelative::Custom(Box::new(fragment_url)));
    let body = clean.clean(&body).to_string();
    // Replace in one pass; repeated String::replace would copy every image for each formula.
    static PLACEHOLDERS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"<span data-pecan-export="(\d+)"></span>"#).expect("valid pattern")
    });
    let mut rendered = String::with_capacity(body.len() + insert_bytes);
    let mut end = 0;
    for capture in PLACEHOLDERS.captures_iter(&body) {
        let matched = capture.get(0).expect("regex has a full match");
        rendered.push_str(&body[end..matched.start()]);
        let index = capture[1].parse::<usize>()?;
        if let Some(html) = inserts.get(index) {
            rendered.push_str(html);
        }
        end = matched.end();
        ensure!(rendered.len() <= MAX_OUTPUT, "HTML 超过 128 MiB 导出预算");
    }
    rendered.push_str(&body[end..]);
    let rendered = align_note_symbols(&rendered, &snapshot.preferences)?;
    let prefs = &snapshot.preferences;
    let families = super::typography::families(&prefs.english_font, &prefs.chinese_font);
    let code_families = super::typography::families(&prefs.code_font, &prefs.chinese_font);
    let english = css_font(families.english_family());
    let chinese = css_font(families.chinese_family());
    let code = css_font(code_families.english_family());
    let css = format!(
        " :root{{--code-body:#{:06x};--code-header:#{:06x}}}body{{font-family:{english},{chinese},system-ui; font-size:{}px;line-height:{};max-width:{}px}}pre,code{{font-family:{code},{chinese},ui-monospace,monospace}}pre{{font-size:{}px}}",
        pecan_assets::LIGHT_CODE_BODY,
        pecan_assets::LIGHT_CODE_HEADER,
        prefs.font_size,
        prefs.line_height,
        prefs.reading_width,
        prefs.code_size
    );
    let html = format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"pecan-language-mark-license\" content=\"{}\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:; base-uri 'none'; form-action 'none'\"><title>{}</title><style>{}\n{css}</style></head><body>{rendered}</body></html>",
        escape(&pecan_assets::language::license()),
        escape(&snapshot.title),
        include_str!("style.css")
    );
    ensure!(html.len() <= MAX_OUTPUT, "HTML 超过 128 MiB 导出预算");
    Ok(html)
}

/// Rewrite `style="text-align: left|center|right"` on a table cell into the
/// `align` attribute. The value must be the whole style: an extra declaration
/// is not alignment and stays for the sanitizer to remove.
fn cell_alignment(html: &str) -> Cow<'_, str> {
    const MARKER: &str = "style=\"text-align: ";
    if !html.contains(MARKER) {
        return Cow::Borrowed(html);
    }
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(MARKER) {
        let value_at = at + MARKER.len();
        let Some(end) = rest[value_at..].find('"') else {
            break;
        };
        let value = &rest[value_at..value_at + end];
        let aligned = matches!(value, "left" | "center" | "right") && cell_tag_open(&rest[..at]);
        out.push_str(&rest[..at]);
        if aligned {
            out.push_str("align=\"");
            out.push_str(value);
            out.push('"');
            rest = &rest[value_at + end + 1..];
        } else {
            out.push_str(MARKER);
            rest = &rest[value_at..];
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// The text ending at `before` is inside an opening `td` or `th` tag.
fn cell_tag_open(before: &str) -> bool {
    let Some(start) = before.rfind('<') else {
        return false;
    };
    let inside = &before[start + 1..];
    if inside.contains('>') {
        return false;
    }
    let name = inside
        .split(|c: char| c.is_ascii_whitespace())
        .next()
        .unwrap_or("");
    name.eq_ignore_ascii_case("td") || name.eq_ignore_ascii_case("th")
}

/// Symbols share the text baseline, rather than the center of a CSS line box.
/// The fixed, generated symbol markup is handled after sanitization; arbitrary
/// document HTML is never granted trusted style attributes by this pass.
fn align_note_symbols(body: &str, prefs: &pecan_config::PreferencesData) -> Result<String> {
    use super::typography::{Fonts, Role, Style};
    static ICON: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"<img class="note-icon"[^>]*>"#).expect("generated icon pattern")
    });
    static TAG: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"<[^>]*>").expect("generated inline tags"));
    let families = super::typography::families(&prefs.english_font, &prefs.chinese_font);
    let mut fonts = Fonts::new(
        families.english_family(),
        families.chinese_family(),
        &prefs.code_font,
    );
    let mut out = String::with_capacity(body.len());
    let mut end = 0;
    for found in ICON.find_iter(body) {
        let after = &body[found.end()..];
        // A generated title already lives inside a paragraph. An untitled
        // symbol precedes its first paragraph and moves into its hanging gutter.
        let paragraph = after.trim_start().starts_with("<p>");
        let content_start = if paragraph {
            after.find("<p>").map_or(0, |at| at + 3)
        } else {
            0
        };
        let content = &after[content_start..];
        let limit = content
            .find("</p>")
            .or_else(|| content.find("</li>"))
            .unwrap_or(0);
        let lead = &content[..limit];
        let mut style = Style::new(Role::Body, 1., [0x24; 3]);
        let mut ink: Option<super::typography::Ink> = None;
        let mut cursor = 0;
        let (mut bold, mut italic) = (0u32, 0u32);
        for tag in TAG.find_iter(lead) {
            let text = super::word::decode_entities(&lead[cursor..tag.start()]);
            if let Some(measured) = fonts.text_ink(&text, &style) {
                ink = Some(ink.map_or(measured, |old| old.union(measured)));
            }
            match tag.as_str() {
                "<strong>" | "<b>" => bold += 1,
                "</strong>" | "</b>" => bold = bold.saturating_sub(1),
                "<em>" | "<i>" => italic += 1,
                "</em>" | "</i>" => italic = italic.saturating_sub(1),
                "<code>" => {
                    style.role = Role::Code;
                    style.size = 0.88;
                }
                "</code>" => {
                    style.role = Role::Body;
                    style.size = 1.;
                }
                "<br>" | "<br />" => {
                    cursor = lead.len();
                    break;
                }
                _ => {}
            }
            style.bold = bold > 0;
            style.italic = italic > 0;
            cursor = tag.end();
        }
        if let Some(measured) =
            fonts.text_ink(&super::word::decode_entities(&lead[cursor..]), &style)
        {
            ink = Some(ink.map_or(measured, |old| old.union(measured)));
        }
        let drop = ink.map_or(0.5, |ink| ink.object_drop(1.));
        // Both the symbol and its baseline follow the text size, including print.
        let icon = found.as_str().replacen(
            "class=\"note-icon\"",
            &format!("class=\"note-icon\" style=\"vertical-align:-{drop:.5}em\""),
            1,
        );
        out.push_str(&body[end..found.start()]);
        if paragraph {
            out.push_str("<p class=\"note-lead\">");
            out.push_str(&icon);
            end = found.end() + content_start;
        } else {
            out.push_str(&icon);
            end = found.end();
        }
        ensure!(out.len() <= MAX_OUTPUT, "HTML 超过 128 MiB 导出预算");
    }
    out.push_str(&body[end..]);
    Ok(out)
}

/// Exported documents have no base URL; only local anchors remain portable.
fn fragment_url(url: &str) -> Option<Cow<'_, str>> {
    url.starts_with('#').then_some(Cow::Borrowed(url))
}

fn css_font(font: &str) -> String {
    let escaped = font
        .chars()
        .map(|c| {
            if matches!(c, '\\' | '\'' | '"' | '<' | '>' | '\n' | '\r') {
                ' '
            } else {
                c
            }
        })
        .collect::<String>();
    format!("'{escaped}'")
}

/// Copy image assets to a fresh companion directory without modifying the source file.
pub(super) async fn markdown(
    snapshot: &Snapshot,
    path: &Path,
    client: Arc<dyn HttpClient>,
) -> Result<()> {
    let source = snapshot.document.source();
    let parent = path.parent().context("Markdown 导出路径没有父目录")?;
    let assets = tempfile::Builder::new()
        .prefix("pecan-assets-")
        .tempdir_in(parent)?;
    let mut images = Images::new(&snapshot.directory, client);
    let mut edits = Vec::new();
    let mut parser = Parser::new_ext(source, options()).into_offset_iter();
    while let Some((event, range)) = parser.next() {
        match event {
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                let destination = images.copy(&dest_url, assets.path()).await?;
                let mut alt = String::new();
                for (event, _) in parser.by_ref() {
                    match event {
                        Event::End(TagEnd::Image) => break,
                        Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                        Event::SoftBreak | Event::HardBreak => alt.push(' '),
                        _ => {}
                    }
                }
                let alt = alt
                    .replace('\\', "\\\\")
                    .replace('[', "\\[")
                    .replace(']', "\\]");
                let title = if title.is_empty() {
                    String::new()
                } else {
                    format!(" \"{}\"", title.replace('\\', "\\\\").replace('"', "\\\""))
                };
                edits.push((range, format!("![{alt}](<{destination}>{title})")));
            }
            Event::Html(raw) | Event::InlineHtml(raw) => {
                let replaced = rewrite_html_images(&raw, &mut images, Some(assets.path())).await?;
                if raw.as_ref() != replaced {
                    edits.push((range, replaced));
                }
            }
            _ => {}
        }
    }
    let mut output = String::with_capacity(source.len());
    let mut end = 0;
    for (range, replacement) in edits {
        ensure!(range.start >= end, "图片源范围重叠，无法保留原始 Markdown");
        output.push_str(&source[end..range.start]);
        output.push_str(&replacement);
        end = range.end;
    }
    output.push_str(&source[end..]);
    ensure!(output.len() <= MAX_OUTPUT, "Markdown 超过 128 MiB 导出预算");
    super::atomic_write(path, output.as_bytes())?;
    if std::fs::read_dir(assets.path())?.next().is_some() {
        let _ = assets.keep();
    }
    Ok(())
}

async fn rewrite_html_images(
    raw: &str,
    images: &mut Images,
    assets: Option<&Path>,
) -> Result<String> {
    // Matching only tag boundaries; the existing HTML tokenizer decodes attributes
    // and entities. Code-fence text never reaches this function.
    static TAGS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?is)<img\b(?:[^>"']|"[^"]*"|'[^']*')*>"#).expect("valid pattern")
    });
    let mut output = String::new();
    let mut end = 0;
    for tag in TAGS.find_iter(raw) {
        output.push_str(&raw[end..tag.start()]);
        if let Some(image) = pecan_document::html_image(tag.as_str()) {
            let uri = match assets {
                Some(directory) => images.copy(&image.uri, directory).await?,
                None => images.embed(&image.uri).await?,
            };
            write!(
                output,
                "<img src=\"{}\" alt=\"{}\"",
                escape(&uri),
                escape(&image.alt)
            )?;
            if let Some(width) = image.width {
                write!(output, " width=\"{width}\"")?;
            }
            if let Some(height) = image.height {
                write!(output, " height=\"{height}\"")?;
            }
            output.push('>');
        } else {
            output.push_str(tag.as_str());
        }
        end = tag.end();
        ensure!(output.len() <= MAX_OUTPUT, "HTML 超过 128 MiB 导出预算");
    }
    output.push_str(&raw[end..]);
    ensure!(output.len() <= MAX_OUTPUT, "HTML 超过 128 MiB 导出预算");
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pecan_config::PreferencesData;
    use pecan_document::Document;
    use pecan_math::RenderQueue as MathQueue;
    use pecan_mermaid::RenderQueue;

    fn snapshot(source: &str, directory: &Path) -> Snapshot {
        Snapshot {
            document: Arc::new(Document::parse(source.to_owned()).unwrap()),
            directory: directory.into(),
            title: "验证文档.md".into(),
            preferences: Arc::new(PreferencesData::default()),
            diagrams: RenderQueue::new(),
            math: MathQueue::new(),
            codes: pecan_format::CodeSnapshot::default(),
            resources: None,
        }
    }

    #[test]
    fn code_emphasis_keeps_whitespace_and_unsupported_syntax_literal() {
        for source in [
            "snake_case",
            r"snake\_case",
            "snake_case &amp;",
            "# comment",
            "[link](url)",
            "<b>html</b>",
            "`literal` *strong*",
            "# _comment_",
            "[link](url) *strong*",
            "<b>html</b> *strong*",
            "    ",
        ] {
            let events = code_events(source);
            let text: String = events
                .iter()
                .filter_map(|event| match event {
                    Event::Text(text) => Some(text.as_ref()),
                    _ => None,
                })
                .collect();
            assert_eq!(text, source, "literal code {source:?}");
        }
        let events = code_events("  **version**  ");
        let text: String = events
            .iter()
            .filter_map(|event| match event {
                Event::Text(text) => Some(text.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(
            text, "  version  ",
            "source whitespace must survive emphasis parsing"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Start(Tag::Strong)))
        );
    }

    #[test]
    fn html_formats_all_code_containers_but_markdown_preserves_source() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let pattern = regex::Regex::new(r"(?s)<pre><code>(.*?)</code></pre>").unwrap();
            for width in [2, 4, 8] {
                let mut snapshot = snapshot(super::super::FORMAT_FIXTURE, directory.path());
                Arc::make_mut(&mut snapshot.preferences).code_tab_size = width.to_string();
                let output = html(&snapshot, crate::network::client()).await.unwrap();

                let actual: Vec<_> = pattern
                    .captures_iter(&output)
                    .map(|c| {
                        let fragment = format!("<code>{}</code>", &c[1]);
                        let doc = roxmltree::Document::parse(&fragment).unwrap();
                        doc.descendants()
                            .filter(|node| node.is_text())
                            .filter_map(|node| node.text())
                            .collect::<String>()
                    })
                    .collect();
                assert_eq!(actual, super::super::expected_code(width));
                let path = directory.path().join("source.md");
                markdown(&snapshot, &path, crate::network::client())
                    .await
                    .unwrap();
                assert_eq!(
                    std::fs::read_to_string(path).unwrap(),
                    super::super::FORMAT_FIXTURE
                );
                assert_eq!(snapshot.document.source(), super::super::FORMAT_FIXTURE);
            }
        });
    }

    #[test]
    fn deeply_nested_notes_fail_with_a_budget_error() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let source = format!(
                "{}Body\n\n{}",
                "<div class=\"pecan-highlight\"></div>\n\n".repeat(MAX_NOTE_DEPTH + 1),
                "<div class=\"pecan-highlight-end\"></div>\n\n".repeat(MAX_NOTE_DEPTH + 1),
            );
            let snapshot = snapshot(&source, directory.path());
            for result in [
                super::super::pdf::pdf(&snapshot, crate::network::client()).await,
                super::super::word::docx(&snapshot, crate::network::client()).await,
            ] {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("提示块嵌套超过 32 层")
                );
            }
        });
    }
    #[test]
    fn typed_and_generic_notes_embed_flat_icons_without_replacing_user_emoji() {
        futures_lite::future::block_on(async {
            assert!(matches!(
                highlight_edge("<div class=\"pecan-highlight pecan-note\"></div>"),
                Some(NoteMarker::Open {
                    kind: Some(AlertKind::Note),
                    titled: true,
                })
            ));
            for (source, titled) in [
                ("::: 标题\n正文\n:::", true),
                ("::: **加粗** *italic* ~~旧版~~\n正文\n:::", true),
                ("::: \n正文\n:::", false),
            ] {
                let expanded = export_markdown(source);
                assert!(matches!(
                    highlight_edge(expanded.lines().next().unwrap()),
                    Some(NoteMarker::Open { kind: None, titled: actual }) if actual == titled
                ));
            }
            assert!(highlight_edge("<div class=\"pecan-highlighted\"></div>").is_none());
            let directory = tempfile::tempdir().unwrap();
            let source = format!("{NOTE_FIXTURE}\n:::\n```rust\nlet only_code = true;\n```\n:::\n");
            let html = html(
                &snapshot(&source, directory.path()),
                crate::network::client(),
            )
            .await
            .unwrap();
            assert_eq!(html.matches("class=\"note-icon\"").count(), 4);
            assert!(
                !html.contains("<div class=\"pecan-highlight\"></div>"),
                "a note after embedded code must wrap its icon and content"
            );
            assert!(
                html.contains("<div class=\"pecan-highlight\">\n<img class=\"note-icon\""),
                "generic marks sit in the gutter, outside any optional paragraph"
            );
            assert!(
                html.contains(
                    "<div class=\"pecan-highlight pecan-note\">\n<p><span class=\"note-heading\"><img class=\"note-icon\""
                ),
                "typed title marks stay inline"
            );
            assert!(html.contains("only_code"));
            for expected in [
                "💡 自定义",
                "保留 ℹ️ 正文。",
                "💡 提示正文",
                "⚠️ 💡 ℹ️ ❗ 🚫 ❌",
                "<code>ℹ️</code>",
            ] {
                assert!(
                    html.contains(expected),
                    "HTML lacks original content {expected}"
                );
            }
            assert!(
                !html.contains("ℹ️ <strong>"),
                "generated alert prefix became an SVG"
            );
            assert!(
                !html.contains("content: \"💡\""),
                "CSS does not draw emoji marks"
            );
            assert!(!html.contains("data-pecan-export"));
        });
    }
    #[test]
    fn html_tries_explicit_font_families_before_generic_fallbacks() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            for (english, chinese, code) in [
                (".SystemMonospace", ".SystemUIFont", ".SystemMonospace"),
                ("Latin Custom", "CJK Custom", "Code Custom"),
                ("Font'</style><script>", "CJK\\\nFont", "Code\"Font"),
            ] {
                let mut snapshot = snapshot("正文 `code`", directory.path());
                let prefs = Arc::make_mut(&mut snapshot.preferences);
                prefs.english_font = english.into();
                prefs.chinese_font = chinese.into();
                prefs.code_font = code.into();
                let body = super::super::typography::families(english, chinese);
                let code = super::super::typography::families(code, chinese);
                let output = html(&snapshot, crate::network::client()).await.unwrap();
                assert!(output.contains(&format!(
                    "body{{font-family:{},{},system-ui;",
                    css_font(body.english_family()),
                    css_font(body.chinese_family()),
                )));
                assert!(output.contains(&format!(
                    "pre,code{{font-family:{},{},ui-monospace,monospace}}",
                    css_font(code.english_family()),
                    css_font(body.chinese_family()),
                )));
                assert_eq!(output.matches("</style>").count(), 1);
                assert!(!output.contains("<script>"));
                assert!(!output.contains(".SystemUIFont"));
                assert!(!output.contains(".SystemMonospace"));
            }
        });
    }
    #[test]
    fn html_table_columns_keep_their_alignment() {
        futures_lite::future::block_on(async {
            let input = "\
| 左 | 中 | 右 |
| :--- | :---: | ---: |
| 甲 | 乙 | 丙 |
| - 一项 | 说明 | 12 |

<td style=\"text-align: center; color: red\">注入</td>
<div style=\"text-align: center\">旁路</div>
";
            let directory = tempfile::tempdir().unwrap();
            let html = html(&snapshot(input, directory.path()), crate::network::client())
                .await
                .unwrap()
                .replace('\n', "");
            for (tag, align, text) in [
                ("th", "left", "左"),
                ("th", "center", "中"),
                ("th", "right", "右"),
                ("td", "left", "甲"),
                ("td", "center", "乙"),
                ("td", "right", "丙"),
                ("td", "right", "12"),
            ] {
                let cell = format!("<{tag} align=\"{align}\">{text}</{tag}>");
                assert!(html.contains(&cell), "missing {cell}");
            }
            assert!(
                html.contains("<td align=\"left\"><ul><li>一项</li></ul></td>"),
                "a list keeps the column: {html}"
            );
            assert!(
                !html.contains("style=\"text-align"),
                "alignment is an attribute, and other styles stay stripped"
            );
            assert!(!html.contains("color: red"));
            assert!(html.contains("注入") && html.contains("旁路"));
            assert!(!html.contains("<div align="));
        });
    }

    #[test]
    fn html_table_cells_render_lists_and_tasks_like_the_document_body() {
        futures_lite::future::block_on(async {
            let input = "| 场景 | 内容 |\n| --- | --- |\n| 任务 | - [x] **已完成**<br>- [ ] 待办 |\n| 有序 | 3. 第三项<br>4. 第四项 |\n| 单项 | - 单个项目 |\n| 混合 | 说明<br><br>- 第一项<br>- 第二项<br><br>结尾 |\n| 星号 | * **第一项**<br>* `第二项` |\n| 嵌套 | <ul><li>父项<ul><li>子项</li></ul></li></ul> |\n| 原样 | `* literal<br>`与`<br>` |\n| 换行 | 第一行<br>第二行 |\n";
            let directory = tempfile::tempdir().unwrap();
            let html = html(&snapshot(input, directory.path()), crate::network::client())
                .await
                .unwrap()
                .replace('\n', "");
            let cell = |row: &str| {
                let label = format!("<td>{row}</td><td>");
                let start = html
                    .find(&label)
                    .unwrap_or_else(|| panic!("row {row} exists"))
                    + label.len();
                &html[start..start + html[start..].find("</td>").expect("cell end")]
            };
            let tasks = cell("任务");
            let boxes: Vec<&str> = tasks
                .match_indices("<input")
                .map(|(at, _)| &tasks[at..at + tasks[at..].find('>').expect("tag end")])
                .collect();
            assert_eq!(boxes.len(), 2, "both task markers are check boxes: {tasks}");
            assert!(
                boxes
                    .iter()
                    .all(|input| input.contains("type=\"checkbox\""))
            );
            assert!(boxes[0].contains("checked") && !boxes[1].contains("checked"));
            assert!(
                html.contains("li input[type=\"checkbox\"] { vertical-align: middle;"),
                "the checkbox shares the list marker's line"
            );
            assert!(tasks.contains("<strong>已完成</strong></li>") && tasks.contains("待办</li>"));
            assert!(!html.contains("[x]") && !html.contains("[ ]"));
            assert_eq!(
                cell("有序"),
                "<ol start=\"3\"><li>第三项</li><li>第四项</li></ol>"
            );
            assert_eq!(cell("单项"), "<ul><li>单个项目</li></ul>");
            assert_eq!(
                cell("混合"),
                "<p>说明</p><ul><li>第一项</li><li>第二项</li></ul><p>结尾</p>"
            );
            assert_eq!(
                cell("星号"),
                "<ul><li><strong>第一项</strong></li><li><code>第二项</code></li></ul>"
            );
            assert_eq!(
                cell("嵌套"),
                "<ul><li><p>父项</p><ul><li>子项</li></ul></li></ul>"
            );
            assert_eq!(
                cell("原样"),
                "<code>* literal&lt;br&gt;</code>与<code>&lt;br&gt;</code>"
            );
            assert_eq!(cell("换行"), "<p>第一行<br>第二行</p>");
        });
    }
    #[test]
    fn html_preserves_semantics_and_blocks_active_content() {
        futures_lite::future::block_on(async {
            let input = "# 标题 `code`\n\n正文 $x^2$ 与 $\\ce{H2O}$\n\n> 引用\n\n- [x] 已完成\n\n| 列 | 值 |\n|---|---|\n| **粗体** | `code`<br>下一行 |\n\n```rust\nfn main() { println!(\"你好\"); }\n```\n\n<script>alert(1)</script><p onclick=\"evil()\">安全正文</p>\n\n[恶意链接](javascript:alert(1))\n\n```mermaid\nflowchart LR\n  A[开始] --> B[结束]\n```";
            let directory = tempfile::tempdir().unwrap();
            let html = html(&snapshot(input, directory.path()), crate::network::client())
                .await
                .unwrap();
            assert!(
                html.contains("data:image/svg+xml;base64,"),
                "equations and diagrams are embedded vectors"
            );
            assert!(html.contains("code-language"));
            assert!(html.contains("<div class=\"code-language\"><img class=\"language-logo\""));
            assert!(
                html.contains("<span style="),
                "syntax highlight spans retained"
            );
            assert!(html.contains("<strong>粗体</strong>"));
            assert!(html.contains("<br>"));
            assert!(!html.contains("<script"));
            assert!(!html.contains("onclick"));
            assert!(!html.contains("href=\"javascript:"));
            assert!(
                !html.contains("data-pecan-export"),
                "all placeholders are expanded"
            );
        });
    }

    #[test]
    fn html_keeps_fragment_links_and_footnotes_but_denies_relative_resources() {
        futures_lite::future::block_on(async {
            let input = "# 标题 {#section}\n\n[跳转](#section) 与注释[^note]\n\n[^note]: 脚注正文 [返回标题](#section)\n\n[文件](other.md) [根路径](/relative) [协议相对](//example.com/file) [恶意](javascript:alert(1)) [外链](https://example.com)\n\n<a id=\"backlink\" href=\"#section\">返回</a>";
            let directory = tempfile::tempdir().unwrap();
            let html = html(&snapshot(input, directory.path()), crate::network::client())
                .await
                .unwrap();
            for expected in [
                "id=\"section\"",
                "href=\"#section\"",
                "id=\"note\"",
                "href=\"#note\"",
                "id=\"backlink\"",
                "href=\"https://example.com\"",
            ] {
                assert!(html.contains(expected), "HTML lacks {expected}");
            }
            for denied in ["other.md", "/relative", "//example.com/file", "javascript:"] {
                assert!(
                    !html.contains(&format!("href=\"{denied}")),
                    "HTML retains forbidden destination {denied}"
                );
            }
        });
    }
    #[test]
    fn markdown_copies_reference_and_html_images_without_touching_code() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let image = directory.path().join("图 片.png");
            let bytes = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9ZcXcAAAAASUVORK5CYII=").unwrap();
            std::fs::write(&image, &bytes).unwrap();
            let input = "# Images\n\n![inline](<图 片.png> \"title\")\n\n![reference][img]\n\n[img]: <图 片.png>\n\n<img src=\"图%20片.png\" width=\"100\" alt=\"HTML 图\">\n\n```md\n![not an image](missing.png)\n```\n";
            let path = directory.path().join("saved.md");
            markdown(
                &snapshot(input, directory.path()),
                &path,
                crate::network::client(),
            )
            .await
            .unwrap();
            let output = std::fs::read_to_string(&path).unwrap();
            assert!(output.contains("```md\n![not an image](missing.png)\n```"));
            assert!(output.contains("![inline](<pecan-assets-"));
            assert!(output.contains("![reference](<pecan-assets-"));
            let parsed = Parser::new_ext(&output, options());
            let image_paths: Vec<_> = parsed
                .filter_map(|event| match event {
                    Event::Start(Tag::Image { dest_url, .. }) => Some(dest_url.to_string()),
                    _ => None,
                })
                .collect();
            assert_eq!(image_paths.len(), 2);
            for file in image_paths {
                assert_eq!(std::fs::read(directory.path().join(file)).unwrap(), bytes);
            }
            assert!(output.contains("width=\"100\""));
            let portable = html(
                &snapshot(&output, directory.path()),
                crate::network::client(),
            )
            .await
            .unwrap();
            assert!(portable.contains("data:image/png;base64,"));
        });
    }
    #[test]
    fn failed_resource_export_preserves_destination_and_removes_temporary_assets() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("existing.md");
            std::fs::write(&path, "old").unwrap();
            let result = markdown(
                &snapshot("![missing](missing.png)", directory.path()),
                &path,
                crate::network::client(),
            )
            .await;
            assert!(result.is_err());
            assert_eq!(std::fs::read_to_string(path).unwrap(), "old");
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        });
    }
    #[test]
    #[ignore = "Exports an external local regression document through the Rust exporters"]
    fn real_document_exports() {
        let input = std::env::var_os("PECAN_EXPORT_VERIFY")
            .map(std::path::PathBuf::from)
            .expect("set PECAN_EXPORT_VERIFY");
        let output = std::env::var_os("PECAN_EXPORT_OUTPUT")
            .map(std::path::PathBuf::from)
            .expect("set PECAN_EXPORT_OUTPUT");
        std::fs::create_dir_all(&output).unwrap();
        let source = std::fs::read_to_string(&input).unwrap();
        let client = crate::network::client();
        let directory = input.parent().unwrap();
        for format in [
            super::super::DocumentFormat::Html,
            super::super::DocumentFormat::Markdown,
            super::super::DocumentFormat::Word,
            super::super::DocumentFormat::Pdf,
        ] {
            if matches!(format, super::super::DocumentFormat::Pdf)
                && std::env::var_os("PECAN_EXPORT_SKIP_PDF").is_some()
            {
                continue;
            }
            let started = std::time::Instant::now();
            let snapshot = snapshot(&source, directory);
            let path = output.join(format!("export.{}", format.extension()));
            futures_lite::future::block_on(super::super::write_document(
                snapshot,
                format,
                &path,
                client.clone(),
            ))
            .unwrap();
            eprintln!(
                "{format:?}: {:?}, {} bytes",
                started.elapsed(),
                path.metadata().unwrap().len()
            );
        }
    }
}
