# Pecan rich text refinements

Upstream: gpui-base 0.9.0, Apache-2.0. LICENSE-APACHE is preserved. The workspace uses a local crates.io patch. Cargo.toml omits upstream integration-test and benchmark targets not included in this runtime copy.

Inline code explicitly configured with self_center is vertically centered with its glyphs. The compact background and padding retain the native selection and copying paths.

CodeBox also retains an optical horizontal offset calculated from the shaped glyphs' real font bounds. This balances visible side bearings while preserving explicit padding, source whitespace, wrapping and intrinsic widths. Layout measures each distinct glyph once per span; paint and hit-testing share the cached origin without querying font metrics again.

Read-only task markers use 16px neutral checkboxes with a 1px subdued border, 3px corners and a soft checked fill. The check is a native SVG mask tinted by the reading foreground and rasterized at the window's actual scale. Both states center on the first text line without affecting selection, copying or Markdown task syntax.

TextViewStyle has opt-in paragraph justification. Body paragraphs use the patched GPUI TextLayout for shared painting, selection, and link coordinates. Headings and ordinary table cells retain their alignment; nowrap tables do not stretch. Mixed inline flows distribute available width among text fragments, keeping code and math atoms intact. Their width/typography cache includes alignment, so unchanged frames reuse wrapping and positioning.

Scrollable nested CodeBlock uses a native div border outside its scroll track, with fixed actions and masks/scrollbars only on opted-in axes. Top-level application fences use the native readonly Editor instead of synthetic paged Markdown. The fully expanded Editor intersects its text bounds with the ancestor content mask to shape only visible rows while retaining complete scroll geometry. Link hover submits one cursor request, choosing PointingHand for actual link glyphs and IBeam for selectable plain text. Existing table and text-selection behavior is preserved.

PECAN-PATCH.diff contains the text changes against the registry source. Application tests cover capsule centering, paragraph endings and copying, table vertical centering/explicit breaks/horizontal scrolling, and native code-border paint/clipping at 1x and 2x. These geometric tests do not substitute for actual desktop pixel verification.

MarkdownExtensions has opt-in inline_code_emphasis. It uses the existing Markdown parser at document preparation, retains a single selectable native code capsule, and composes rendered/source ranges. Code fences and unsupported inline code syntax remain literal. Native tests cover nested marks, whitespace, nonzero source offsets, selection and literal fences.

TextView has an opt-in on_first_line_ink observation during prepaint. Text and mixed inline fragments report their visible glyph extent in window coordinates after native shaping and wrapping. Pecan places note symbols in a gutter around that first row's ink center rather than shifting all text or using fixed font offsets. Views without observers do not query glyph bounds. The callback shares the existing layout and does not alter source indices, hit testing, line height or selection. Native layout and real Metal pixel tests cover Chinese/English, mixed marks, enlarged titles, inline code and wrapping.
