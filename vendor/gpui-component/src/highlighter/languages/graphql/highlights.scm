; Queries over the upstream GraphQL grammar, shared by reading and export.
(name) @variable
(field (name) @property)
(field_definition (name) @property)
(named_type (name) @type)
(variable (name) @variable.parameter)
(directive (name) @function)
(fragment_name) @type
(string_value) @string
(int_value) @number
(float_value) @number
(boolean_value) @boolean
(null_value) @constant.builtin
(comment) @comment
(operation_type) @keyword
[ "type" "input" "interface" "enum" "scalar" "union" "schema" "extend" "fragment" "on" "implements" "directive" ] @keyword
[ "{" "}" "(" ")" "[" "]" ] @punctuation.bracket
[ ":" "!" "=" "@" "$" "|" "&" "..." ] @operator
