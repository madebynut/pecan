# anydoc integration

Upstream: anydoc 0.2.5 from the Cargo registry release, MIT (LICENSE preserved),
release commit `0a6a4fd29e5c0315972de348fa63de8e767f632e` in firecrawl/anydoc.
The sources are the published ones; `PECAN-PATCH.diff` is the whole difference and
is checked against the registry source.

Two changes remain:

- `src/lib.rs` adds a public `model_to_markdown` wrapper around the existing
  serializer. Pecan rewrites embedded image sources to its private temporary
  assets before serialization, so images survive reflow without parsing the
  document twice. Upstream exposes the model but not its serializer.
- `src/package/limits.rs` tightens the desktop resource caps: entries 32 MiB,
  archive reads 64 MiB, 10,000 entries, 200,000 XML nodes, 100,000 grid/expansion
  positions, 32 MiB expansion text/assets, and one million legacy records. Parsing
  still returns explicit ResourceLimit errors. Upstream keeps these constants
  fixed on purpose, so they cannot be set from outside the crate.

`Cargo.toml` and the example are the published ones. `rustfmt.toml` is the only
added file: it repeats the upstream repository setting, which the published crate
omits, so `cargo fmt --all` leaves the upstream sources as published.

Remove this vendor copy when upstream exposes the serializer and Pecan either
accepts the upstream caps or can configure them.
