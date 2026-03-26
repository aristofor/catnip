; Boolean literals
(true) @constant.builtin
(false) @constant.builtin
(none) @constant.builtin

; Numbers
(integer) @number
(float) @number
(decimal) @number
(imaginary) @number

; Strings
(string) @string
(fstring) @string
(escape_sequence) @string.escape

; Comments
(comment) @comment

; Identifiers
(identifier) @variable

; Function calls
(call
  (identifier) @function.call)
(callattr
  method: (identifier) @function.method.call)

; Function parameters
(lambda_param
  name: (identifier) @variable.parameter)
(variadic_param
  name: (identifier) @variable.parameter)

; Control flow statements (highlight the whole statement as keyword context)
(break_stmt) @keyword
(continue_stmt) @keyword
(return_stmt) @keyword
(while_stmt) @keyword
(for_stmt) @keyword
(if_expr) @keyword
(elif_clause) @keyword
(else_clause) @keyword
(match_expr) @keyword
(pragma_stmt) @keyword

; Pattern matching
(pattern_wildcard) @variable.builtin
(pattern_var
  (identifier) @variable)
(pattern_star
  (identifier) @variable)

; Attributes
(getattr
  attribute: (identifier) @property)

; Operators (via op nodes)
(add_sub_op) @operator
(mul_div_op) @operator
(comp_op) @operator
(shift_op) @operator
(unary_op) @operator
(bcast_op) @operator
(bcast_unary_op) @operator
(operator_symbol) @operator
