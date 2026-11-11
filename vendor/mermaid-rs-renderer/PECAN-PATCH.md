# Mermaid layout and connector patch

Upstream: mermaid-rs-renderer 0.3.1 (MIT), copied from the Cargo registry release. Runtime patches are recorded below and in PECAN-PATCH.diff; untouched runtime sources remain upstream code. Cargo.toml retains runtime dependencies and features, omitting development benchmarks and release-profile settings owned by the workspace.

PECAN-PATCH.diff records the changes:

- Align generic, state and class filled arrow tips with their anchors and omit outer triangle strokes; orient state start markers outward. Correct class inheritance triangle direction and include its half-unit outline in the marker reference, removing obsolete class endpoint insets. C4 uses exact triangle references; block and architecture connectors share the corrected generic markers. Flowchart overlay tips use fill only; shorten shafts into the arrow body while keeping tips at the original endpoint, so rectangular line ink cannot protrude through the tip.
- Restore upstream obstacle routing and tiny-graph shortcuts. Simplify unobstructed forward flowchart connections to centered ports, straight lines or a single cubic curve. Only smooth monotonic four-point routes with empty control hulls; subgraphs, obstacles, loops, parallel relationships and semantic decorations retain their routed topology. Other routed bends use 10-unit corner rounding. Existing label collision resolution remains in use. Other specialized layouts retain their original curve semantics.
- Paint frame outlines, dividers, label plates and their text after lifelines and diagram contents, preserving opaque fill so alt/loop/opt tags remain above vertical lines. Use 6/4-unit short dashes on sequence frame outlines and dividers.
- Escape normalized font-family names when serializing SVG attributes; the application selects independent Latin/CJK spans and font-aware cache identity in pecan-mermaid.
- Align sequence end/start marker tips with the message endpoints (`refX=10` / `refX=0`) and omit the triangle outline so painted tips do not extend past the lifeline. This marker geometry is also shared by ZenUML.
- Keep sequence lifelines at 1.5 SVG units rather than 0.5, so fitted light diagrams retain visible coverage. Application palette roles remain in pecan-mermaid/src/renderer.rs.
- Let sequence labels wrap at the normal configured width (up to 28 characters), and size actor boxes from their measured labels.
- Reserve the previous self-loop tail, message label height, and separate first/else header bands in the timeline.
- Keep self-loops short with a font-scaled fixed width and curved rendering, independent of caption length. Place captions separately and include both captions and loop return segments in branch frames.
- Use uniform filled sequence-number badges centered on actual outgoing points. One- and two-digit values both use 12-unit bold type. Compute a shared radius once from the selected font's widest digit pair, with a compact double-digit inset and a 22-unit baseline diameter. Three or more digits scale down within the same circle. Reserve a single radius in rows, frames, collision checks and output bounds.
- Place condition text below the frame tag, and reserve space above each badge for message text.
- Exclude notes after a frame from that frame and leave room after a self-loop before a note.
- Use deterministic measured center-label slots for sequence diagrams; retain upstream placement for ZenUML.

No parser, font engine, raster limits, or cache budgets are replaced. Tests live in crates/pecan-mermaid/tests/sequence_layout.rs and the existing renderer/GPUI tests. When upstream includes equivalent fixes, switch the workspace dependency back to the registry release and remove this vendor copy.

Sequence message shafts stop 8 SVG units inside filled arrowheads. A separate unpainted path carries native SVG markers at the unchanged original endpoints. Self-call cubic routing is retained. Regression checks marker endpoint geometry and 800% tip raster pixels in both themes.

Sequence end triangles have equal tail x coordinates, keeping their base perpendicular to horizontal messages. Transformed SVG regression covers forward, reverse, self-call and start arrows without changing endpoint anchors.
