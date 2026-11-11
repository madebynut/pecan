; Explicit TypeScript script blocks (including Vue host highlighting).
((script_element
  (start_tag) @_tag
  (raw_text) @injection.content)
 (#match? @_tag "(?i)lang[ \t]*=[ \t]*['\"](?:ts|typescript)['\"]")
 (#set! injection.language "typescript"))

((script_element
  (start_tag) @_tag
  (raw_text) @injection.content)
 (#not-match? @_tag "(?i)lang[ \t]*=[ \t]*['\"](?:ts|typescript)['\"]")
 (#set! injection.language "javascript"))

((style_element
  (raw_text) @injection.content)
 (#set! injection.language "css"))
