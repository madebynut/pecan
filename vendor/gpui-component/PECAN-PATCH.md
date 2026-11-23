# Pecan component patches

Based on gpui-component 0.10.0 (Apache-2.0).

- Sidebar items and settings pages carry debug selectors for click regressions. Folding a settings page by its label is upstream behavior since 0.10.0.
- Tree-sitter configuration supplies missing Swift, C#, GraphQL, Proto and CMake queries.
- C++ inherits its C queries; TSX inherits TypeScript/JavaScript plus JSX queries. JSX tags/attributes are also added to JavaScript.
- The SQL query is the tree-sitter-sequel 0.3.13 one with capture names this highlighter styles; numeric literals are disjoint from string captures.
- Markdown reference links use supported theme captures; task markers, table-cell markup and inline strikethrough are highlighted. Syntax themes accept an optional strikethrough decoration in both native and fallback backends.
- TSX and EJS/ERB enable their embedded grammars; ERB uses Ruby injections. HTML selects TypeScript for explicit lang=ts script blocks, preserving JavaScript otherwise.

Query provenance and dependency licenses are recorded in licenses/HIGHLIGHTING-0.7.1.md and licenses/NOTICE-highlighting.md. Grammars remain supplied by published Tree-sitter crates. Pecan does not implement a second programming-language lexer.
