# Pecan Tree-sitter query attribution

Queries are bundled with their parser crate licenses. Swift and C# reference the published crate queries directly. GraphQL queries and HTML injection adjustments are Pecan configuration over the upstream grammar.

C++ combines tree-sitter-c 0.24.2 and tree-sitter-cpp 0.23.4 highlight queries; both parser licenses are included. JSX is combined with the existing JavaScript and TypeScript base queries. Proto adds syntax version literals. SQL normalizes capture names and keeps numeric literals apart from strings; CMake converts the Lua pattern into a regex understood by this highlighter.

- jsx: tree-sitter-javascript-0.23.1/queries/highlights-jsx.scm
  Original SHA-256: 0cad1712acaf40804bc5dc7c8476858752c1f549cdf7d8c63c747317be0f8fa6
  Bundled SHA-256: 0cad1712acaf40804bc5dc7c8476858752c1f549cdf7d8c63c747317be0f8fa6
- proto: tree-sitter-proto-0.2.0/queries/highlights.scm
  Original SHA-256: 07eb14b859bbadaa94cac1eae48b77a3bb4a73ef0f3e813dd6c28b5c513c45a0
  Bundled SHA-256: 690dc9c2c2da36a034af7c1e185ce60c11f44981e616329475bca6d7575e8957
- sql: tree-sitter-sequel-0.3.13/queries/highlights.scm
  Original SHA-256: ae19313e5df8d7f8f38482b09de63fc3ffb1fbc32f6d5c0b7abe48dc2c894b38
  Bundled SHA-256: 9b85a186354f94ab582b03425d3664c19c01a6f0ed6296bd6d5f126d6fab43b7
- cmake: tree-sitter-cmake-0.7.6/queries/highlights.scm
  Original SHA-256: 935161098bf9e335d4984496626a46d13a650df9145871a9578fe0e25a3e9a6c
  Bundled SHA-256: 37529ff69c18d03138e6c5b0ecdcc6607eaaabbc3564dabd398addc8927c70ba
