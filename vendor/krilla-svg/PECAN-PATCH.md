# SVG PDF dependency alignment

Upstream: krilla-svg 0.7.0, MIT OR Apache-2.0, release commit
`a125370b954f810c37b186a080889cbe5f6b30fc` in LaurenzV/krilla.
Runtime Rust sources are unchanged. Licenses and upstream NOTICE are distributed
in the project's `licenses/` directory.

The normalized Cargo manifest aligns resvg/usvg with Pecan's existing 0.46
rendering stack and mixed-script font fallback patch. Text support stays enabled;
unused default rendering features are disabled. flate2 uses its Rust backend.
The omitted upstream example is not shipped in this trimmed vendor copy.

`PECAN-PATCH.diff` reconstructs the manifest against the published Cargo release.
PDF export tests and real-document visual checks verify SVG text, mathematics,
diagrams and images after this dependency alignment. Remove this vendor copy
when upstream supports the same rendering versions.
