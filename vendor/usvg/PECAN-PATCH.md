# Mixed-script span font fallback patch

Upstream: usvg 0.46.0 from the Cargo registry release, under Apache-2.0 OR MIT.
Both licenses and the runtime sources are preserved. The workspace selects this
copy through a crates.io patch; upstream dependencies and features are unchanged.

The only runtime change is in `src/text/layout.rs`. Each span still shapes the
complete text chunk to preserve bidi ordering and glyph clusters. Missing-glyph
fallback and warnings now consider only glyphs within that span's byte range.
Missing Chinese glyphs belonging to a neighboring span must not cause a valid
English face to be replaced by a font covering the entire mixed-script chunk.
Fallback for missing glyphs inside the current span keeps the upstream behavior.

The change adds no allocation, font discovery, cache, thread, or rendering engine.
Mermaid retains independent Latin/CJK SVG spans and the existing shared fontdb.
Regression tests inspect the font of each actual positioned glyph, including
mixed labels, multiple lines, bold numbers, flowcharts and sequence diagrams in
both themes, using platform defaults and installed custom families.

`PECAN-PATCH.diff` records the runtime change and is checked against the registry
source. Remove this vendor copy when an equivalent fix is available upstream.


SVGZ decompression stops after 16 MiB + 1 and returns ElementsLimitReached rather than allocating an unbounded expanded XML buffer. Ordinary SVGZ parsing and uncompressed SVG behavior remain unchanged. The application regression fixture contains a 16 KiB gzip stream expanding just past the budget; it is used only in tests.
