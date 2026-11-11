# anydoc integration

Upstream: https://github.com/firecrawl/anydoc
Version: 0.2.4
Commit: e7178e099b6bf9171e2c52b832cbdb4134b69112
License: MIT (LICENSE preserved)

The source is adapted with a public `model_to_markdown` wrapper around the existing serializer. Moye rewrites embedded image sources to its private temporary assets before serialization, so images survive reflow without parsing the document twice. Cargo.toml excludes upstream bindings and development targets; all runtime dependencies are retained.

Desktop resource caps are tightened in package/limits.rs: entries 32 MiB, archive reads 64 MiB, 10,000 entries, 200,000 XML nodes, 100,000 grid/expansion positions, 32 MiB expansion text/assets, and one million legacy records. Parsing still returns explicit ResourceLimit errors.


The ZIP dependency is upgraded to 9.x for the fixed extraction advisories (RUSTSEC-2026-0367/0368/0369/0429). Pecan continues to use bounded part reads, never ZipArchive::extract; package limits and serialization remain unchanged.
