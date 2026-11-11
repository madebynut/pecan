# Pecan Tree-sitter query attribution

Queries are bundled with their parser crate licenses. Swift and C# reference the published crate queries directly. GraphQL queries and HTML injection adjustments are Pecan configuration over the upstream grammar.

C++ combines tree-sitter-c 0.24.2 and tree-sitter-cpp 0.23.4 highlight queries; both parser licenses are included. JSX is combined with the existing JavaScript and TypeScript base queries. Proto adds syntax version literals. SQL normalizes capture names and numeric regexes; CMake converts the Lua pattern into a regex understood by this highlighter.

- jsx: tree-sitter-javascript-0.23.1/queries/highlights-jsx.scm
  Original SHA-256: 0cad1712acaf40804bc5dc7c8476858752c1f549cdf7d8c63c747317be0f8fa6
  Bundled SHA-256: 0cad1712acaf40804bc5dc7c8476858752c1f549cdf7d8c63c747317be0f8fa6
- proto: tree-sitter-proto-0.2.0/queries/highlights.scm
  Original SHA-256: 07eb14b859bbadaa94cac1eae48b77a3bb4a73ef0f3e813dd6c28b5c513c45a0
  Bundled SHA-256: 690dc9c2c2da36a034af7c1e185ce60c11f44981e616329475bca6d7575e8957
- sql: tree-sitter-sequel-0.3.11/queries/highlights.scm
  Original SHA-256: 80e726903b67fbe471bf1272a447ab6a6758e95654f9e9c8f5c2e4af729af8de
  Bundled SHA-256: 1ff363e2109cd4034b620dbb4a8ee2fad05b4730ad8f72724cf7f20c1b464333
- cmake: tree-sitter-cmake-0.7.6/queries/highlights.scm
  Original SHA-256: 935161098bf9e335d4984496626a46d13a650df9145871a9578fe0e25a3e9a6c
  Bundled SHA-256: 37529ff69c18d03138e6c5b0ecdcc6607eaaabbc3564dabd398addc8927c70ba
