# Pecan component patches

Based on gpui-component 0.9.0 (Apache-2.0).

- Settings page labels toggle child groups; sidebar debug selectors support click regressions.
- Tree-sitter configuration supplies missing Swift, C#, GraphQL, Proto and CMake queries.
- C++ inherits its C queries; TSX inherits TypeScript/JavaScript plus JSX queries. JSX tags/attributes are also added to JavaScript.
- SQL query captures use standard styles and regex syntax; numeric literals are disjoint from string captures.
- TSX and EJS/ERB enable their embedded grammars; ERB uses Ruby injections. HTML selects TypeScript for explicit lang=ts script blocks, preserving JavaScript otherwise.

Query provenance and dependency licenses are recorded in licenses/HIGHLIGHTING-0.7.1.md and licenses/NOTICE-highlighting.md. Grammars remain supplied by published Tree-sitter crates. Pecan does not implement a second programming-language lexer.
