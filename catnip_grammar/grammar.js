// FILE: catnip_grammar/grammar.js
/**
 * Tree-sitter grammar for Catnip
 */

const PREC = {
  OR: 1,
  AND: 2,
  NOT: 3,
  COMPARE: 4,
  BIT_OR: 5,
  BIT_XOR: 6,
  BIT_AND: 7,
  SHIFT: 8,
  ADD: 9,
  MUL: 10,
  EXP: 11,
  UNARY: 12,
  CALL: 13,
  MEMBER: 14,
  KEYWORD: 15,  // Force keyword recognition over identifier
};

module.exports = grammar({
  name: 'catnip',

  word: $ => $.identifier,

  externals: $ => [
    $._newline,
  ],

  // Whitespace and comments
  // Newlines are in extras (can be skipped) but also handled by external scanner
  // When external scanner matches, newline is significant; otherwise it's whitespace
  extras: $ => [
    /[ \t\r\n]/,  // Whitespace including newlines
    $.comment,
  ],

  // GLR conflicts - minimal set required for lambda/unpack ambiguity
  conflicts: $ => [
    [$.lambda_param, $._unpack_item],
    [$._atom, $.lambda_param, $._unpack_item],
    [$._atom, $.lambda_param],
    [$._atom, $._unpack_item],
    [$.lambda_params, $._unpack_item],
    [$.args, $.group],  // @>(expr) ambiguity: args vs grouped expression
    [$.kwarg, $.lambda_param],  // @@(x=val) ambiguity: kwarg vs lambda default
    [$.arguments, $.lambda_expr],  // @@() ambiguity: empty args vs lambda start
  ],

  rules: {
    source_file: $ => seq(
      repeat($._newline),  // Allow leading newlines
      optional($._statements),
    ),

    // Statements - separated by semicolons or newlines
    _statements: $ => seq(
      $.statement,
      repeat(seq(repeat1($._statement_separator), $.statement)),
      repeat($._statement_separator),
    ),

    _statement_separator: $ => choice(';', $._newline),

    statement: $ => choice(
      $.assignment,
      $.while_stmt,
      $.for_stmt,
      $.return_stmt,
      $.break_stmt,
      $.continue_stmt,
      $.pragma_stmt,
      $.struct_stmt,
      $.trait_stmt,
      $._expression,
    ),

    pragma_stmt: $ => seq(
      'pragma',
      '(',
      $.pragma_arg,
      repeat(seq(',', $.pragma_arg)),
      ')',
    ),

    pragma_arg: $ => choice($.string, $.true, $.false),

    struct_stmt: $ => seq(
      'struct',
      field('name', $.identifier),
      optional(choice(
        seq($.struct_implements, optional($.struct_extends)),
        seq($.struct_extends, optional($.struct_implements)),
      )),
      '{',
      optional($.struct_fields),
      repeat($.struct_method),
      '}',
    ),

    struct_implements: $ => seq(
      'implements',
      '(',
      $.identifier,
      repeat(seq(',', $.identifier)),
      ')',
    ),

    struct_extends: $ => seq(
      'extends',
      '(',
      field('base', $.identifier),
      ')',
    ),

    struct_fields: $ => prec.right(seq(
      $.struct_field,
      repeat(seq(',', $.struct_field)),
      optional(','),
    )),

    struct_field: $ => seq(
      field('name', $.identifier),
      optional(seq('=', field('default', $._expression))),
    ),

    struct_method: $ => seq(
      field('method_name', $.identifier),
      '(',
      optional($.lambda_params),
      ')',
      '=>',
      $.block,
    ),

    // Trait definition
    trait_stmt: $ => seq(
      'trait',
      field('name', $.identifier),
      optional($.trait_extends),
      '{',
      optional($.struct_fields),
      repeat($.struct_method),
      '}',
    ),

    trait_extends: $ => seq(
      'extends',
      '(',
      $.identifier,
      repeat(seq(',', $.identifier)),
      ')',
    ),

    // Decorator: @identifier (for @jit, @pure, etc.)
    decorator: $ => seq('@', $.identifier),

    assignment: $ => seq(
      repeat($.decorator),  // Zero or more decorators
      $.lvalue,
      repeat(seq('=', $.lvalue)),
      '=',
      $._expression,
    ),

    while_stmt: $ => seq('while', $._expression, $.block),

    for_stmt: $ => seq('for', $.unpack_target, 'in', $._expression, $.block),

    return_stmt: $ => prec.right(seq('return', optional($._expression))),

    break_stmt: $ => 'break',

    continue_stmt: $ => 'continue',

    if_expr: $ => prec.right(seq(
      'if',
      field('condition', $._expression),
      field('consequence', $.block),
      repeat($.elif_clause),
      optional($.else_clause),
    )),

    elif_clause: $ => seq(
      'elif',
      field('condition', $._expression),
      field('consequence', $.block),
    ),

    else_clause: $ => seq('else', field('body', $.block)),

    match_expr: $ => seq(
      'match',
      field('value', $._expression),
      '{',
      repeat1($.match_case),
      '}',
    ),

    match_case: $ => seq(
      $.pattern,
      optional(seq('if', field('guard', $._expression))),
      '=>',
      $.block,
    ),

    pattern: $ => $.pattern_or,

    pattern_or: $ => seq(
      $._pattern_primary,
      repeat(seq('|', $._pattern_primary)),
    ),

    _pattern_primary: $ => choice(
      $.pattern_struct,
      $.pattern_tuple,
      $.pattern_literal,
      $.pattern_var,
      $.pattern_wildcard,
    ),

    pattern_struct: $ => seq(
      field('struct_name', $.identifier),
      '{',
      field('fields', $.identifier),
      repeat(seq(',', field('fields', $.identifier))),
      optional(','),
      '}',
    ),

    pattern_tuple: $ => seq('(', $.pattern_items, ')'),

    pattern_items: $ => seq(
      $._pattern_item,
      repeat(seq(',', $._pattern_item)),
      optional(','),
    ),

    _pattern_item: $ => choice(
      $.pattern_tuple,
      $.pattern_star,
      $.pattern_literal,
      $.pattern_var,
      $.pattern_wildcard,
    ),

    pattern_literal: $ => $.literal,
    pattern_var: $ => $.identifier,
    pattern_wildcard: $ => '_',
    pattern_star: $ => seq('*', $.identifier),

    block: $ => seq(
      '{',
      repeat($._newline),
      optional($._statements),
      '}',
    ),

    // Expressions
    _expression: $ => $._bool_or,

    _bool_or: $ => choice($.bool_or, $._bool_and),
    bool_or: $ => prec.left(PREC.OR, seq($._bool_or, 'or', $._bool_and)),

    _bool_and: $ => choice($.bool_and, $._bool_not),
    bool_and: $ => prec.left(PREC.AND, seq($._bool_and, 'and', $._bool_not)),

    _bool_not: $ => choice($.bool_not, $._comparison),
    bool_not: $ => prec(PREC.NOT, seq('not', $._bool_not)),

    _comparison: $ => choice($.comparison, $._bit_or),
    comparison: $ => prec.left(PREC.COMPARE,
      seq($._bit_or, repeat1(seq($.comp_op, $._bit_or)))
    ),
    comp_op: $ => choice('<=', '<', '>=', '>', token('!='), '=='),

    _bit_or: $ => choice($.bit_or, $._bit_xor),
    bit_or: $ => prec.left(PREC.BIT_OR, seq($._bit_or, '|', $._bit_xor)),

    _bit_xor: $ => choice($.bit_xor, $._bit_and),
    bit_xor: $ => prec.left(PREC.BIT_XOR, seq($._bit_xor, '^', $._bit_and)),

    _bit_and: $ => choice($.bit_and, $._shift),
    bit_and: $ => prec.left(PREC.BIT_AND, seq($._bit_and, '&', $._shift)),

    _shift: $ => choice($.shift, $._additive),
    shift: $ => prec.left(PREC.SHIFT, seq($._shift, $.shift_op, $._additive)),
    shift_op: $ => choice('<<', '>>'),

    _additive: $ => choice($.additive, $._multiplicative),
    additive: $ => prec.left(PREC.ADD, seq($._additive, $.add_sub_op, $._multiplicative)),
    add_sub_op: $ => choice('+', '-'),

    _multiplicative: $ => choice($.multiplicative, $._exponent),
    multiplicative: $ => prec.left(PREC.MUL, seq($._multiplicative, $.mul_div_op, $._exponent)),
    mul_div_op: $ => choice('*', '/', '//', '%'),

    _exponent: $ => choice($.exponent, $._unary),
    exponent: $ => prec.right(PREC.EXP, seq($._unary, '**', $._exponent)),

    _unary: $ => choice($.unary, $.literal, $._primary),
    unary: $ => prec(PREC.UNARY, seq($.unary_op, $._unary)),
    unary_op: $ => choice('-', '+', '~'),

    _primary: $ => choice(
      $.lambda_expr,
      $.nd_recursion,
      $.nd_map,
      $.chained,
      $.list_literal,
      $.tuple_literal,
      $.set_literal,
      $.dict_literal,
      $.call,
      $._atom,
    ),

    // ND-recursion: @@(seed, lambda) or @@ lambda
    nd_recursion: $ => choice(
      prec(2, seq('@@', $.arguments)),    // @@(seed, lambda) - combinator call (prioritaire)
      prec(1, seq('@@', $.lambda_expr)),  // @@ lambda - declaration form
    ),

    // ND-map: @>(data, f) or @> f
    nd_map: $ => choice(
      prec(2, seq('@>', $.arguments)),  // @>(data, f) - applicative form (prioritaire)
      prec(1, seq('@>', $._primary)),   // @> f - lift form
    ),

    chained: $ => prec.left(PREC.MEMBER, seq(
      choice(
        $.list_literal,
        $.tuple_literal,
        $.set_literal,
        $.dict_literal,
        $.call,
        $._atom,
        $.literal,
      ),
      repeat1($._member),
    )),

    call: $ => prec(PREC.CALL, seq($._atom, $.arguments)),

    _member: $ => choice(
      $.getattr,
      $.callattr,
      $.call_member,
      $.broadcast,
      $.index,
      $.fullslice,
    ),

    getattr: $ => prec(1, seq('.', field('attribute', $.identifier))),
    callattr: $ => prec(2, seq('.', field('method', $.identifier), $.arguments)),
    call_member: $ => $.arguments,
    broadcast: $ => seq('.[', $.broadcast_op, ']'),
    index: $ => seq('[', $._slice_expr, ']'),
    fullslice: $ => seq('.[', $.slice_range, ']'),

    _slice_expr: $ => choice($._expression, $.slice_range),

    slice_range: $ => seq(
      optional($._expression),
      ':',
      optional($._expression),
      optional(seq(':', optional($._expression))),
    ),

    broadcast_op: $ => choice(
      $.broadcast_if,
      $.broadcast_nd_recursion,
      $.broadcast_nd_map,
      $.broadcast_binary,
      $.broadcast_unary,
      $.broadcast,
      $._expression,
    ),

    // data.[@@ lambda] - broadcast ND-recursion (higher prec than nd_recursion)
    broadcast_nd_recursion: $ => prec(3, seq('@@', choice($.lambda_expr, $._expression))),

    // data.[@> f] - broadcast ND-map (higher prec than nd_map)
    broadcast_nd_map: $ => prec(3, seq('@>', $._expression)),

    broadcast_if: $ => seq(
      'if',
      choice($.broadcast_binary, $.broadcast_unary, $._expression),
    ),

    broadcast_binary: $ => seq($.bcast_op, $._expression),

    bcast_op: $ => prec(PREC.UNARY + 1, choice(
      '+', '-', '*', '/', '//', '%', '**',
      '<', '<=', '>', '>=', '==', '!=',
      '&', '|', '^', '<<', '>>',
      'and', 'or',
    )),

    broadcast_unary: $ => $.bcast_unary_op,
    bcast_unary_op: $ => choice('abs', 'not', '-', '+', '~'),

    arguments: $ => seq('(', optional($._params), ')'),

    _params: $ => choice($.args_kwargs, $.args, $.kwargs),

    args_kwargs: $ => seq(
      $._expression,
      repeat(seq(',', $._expression)),
      ',',
      $.kwarg,
      repeat(seq(',', $.kwarg)),
      optional(','),
    ),

    args: $ => seq(
      $._expression,
      repeat(seq(',', $._expression)),
      optional(','),
    ),

    kwargs: $ => seq(
      $.kwarg,
      repeat(seq(',', $.kwarg)),
      optional(','),
    ),

    kwarg: $ => seq(field('key', $.identifier), '=', field('value', $._expression)),

    _atom: $ => choice(
      $.identifier,
      $.group,
      $.block,
      $.match_expr,
      $.if_expr,
    ),

    group: $ => seq('(', $._expression, ')'),

    lambda_expr: $ => seq(
      '(',
      optional($.lambda_params),
      ')',
      '=>',
      $.block,
    ),

    lambda_params: $ => choice(
      seq(
        $.lambda_param,
        repeat(seq(',', $.lambda_param)),
        ',',
        $.variadic_param,
      ),
      seq(
        $.lambda_param,
        repeat(seq(',', $.lambda_param)),
      ),
      $.variadic_param,
    ),

    lambda_param: $ => seq(
      field('name', $.identifier),
      optional(seq('=', field('default', $._expression))),
    ),

    variadic_param: $ => seq('*', field('name', $.identifier)),

    unpack_target: $ => choice(
      $.unpack_tuple,
      $.unpack_sequence,
      $.identifier,
    ),

    unpack_tuple: $ => seq('(', $.unpack_items, ')'),

    unpack_sequence: $ => seq(
      $.identifier,
      ',',
      $._unpack_item,
      repeat(seq(',', $._unpack_item)),
      optional(','),
    ),

    unpack_items: $ => seq(
      $._unpack_item,
      repeat(seq(',', $._unpack_item)),
      optional(','),
    ),

    _unpack_item: $ => choice(
      $.unpack_tuple,
      $.variadic_param,
      $.identifier,
    ),

    lvalue: $ => choice($.setattr, $.unpack_target),

    setattr: $ => seq($._atom, repeat1($._member)),

    list_literal: $ => seq('list', '(', optional($.collection_items), ')'),
    tuple_literal: $ => seq('tuple', '(', optional($.collection_items), ')'),
    set_literal: $ => seq('set', '(', optional($.collection_items), ')'),

    collection_items: $ => seq(
      $._expression,
      repeat(seq(',', $._expression)),
      optional(','),
    ),

    dict_literal: $ => seq('dict', '(', optional($.dict_items), ')'),

    dict_items: $ => seq(
      $._dict_entry,
      repeat(seq(',', $._dict_entry)),
      optional(','),
    ),

    _dict_entry: $ => choice($.dict_pair, $.dict_kwarg),

    dict_pair: $ => seq(
      '(',
      field('key', $._expression),
      ',',
      field('value', $._expression),
      ')',
    ),

    dict_kwarg: $ => seq(
      field('key', $.identifier),
      '=',
      field('value', $._expression),
    ),

    literal: $ => choice(
      $.none,
      $.true,
      $.false,
      $.nd_empty_topos,
      $.string,
      $.fstring,
      $.bstring,
      $.number,
    ),

    nd_empty_topos: $ => '@[]',

    none: $ => token(prec(PREC.KEYWORD, 'None')),
    true: $ => token(prec(PREC.KEYWORD, 'True')),
    false: $ => token(prec(PREC.KEYWORD, 'False')),

    number: $ => choice($.float, $.integer),

    float: $ => token(choice(
      /\d+\.\d+([eE][+-]?\d+)?/,
      /\.\d+([eE][+-]?\d+)?/,
      /\d+[eE][+-]?\d+/,
    )),

    integer: $ => token(choice(
      /0[xX][0-9a-fA-F]+/,
      /0[bB][01]+/,
      /0[oO][0-7]+/,
      /\d+/,
    )),

    string: $ => choice(
      $._double_string,
      $._single_string,
      $._long_double_string,
      $._long_single_string,
    ),

    // Simple strings as tokens to prevent extras (comments) from being inserted mid-string
    _double_string: $ => token(seq('"', repeat(choice(/\\['"\\nrtbfv0]|\\x[0-9a-fA-F]{2}|\\u[0-9a-fA-F]{4}|\\U[0-9a-fA-F]{8}/, /[^"\\]+/)), '"')),
    _single_string: $ => token(seq("'", repeat(choice(/\\['"\\nrtbfv0]|\\x[0-9a-fA-F]{2}|\\u[0-9a-fA-F]{4}|\\U[0-9a-fA-F]{8}/, /[^'\\]+/)), "'")),
    // Long strings - handled as single tokens to avoid lookahead
    _long_double_string: $ => token(seq('"""', /([^"\\]|\\.|"[^"]|""[^"])*/, '"""')),
    _long_single_string: $ => token(seq("'''", /([^'\\]|\\.|'[^']|''[^'])*/, "'''")),

    escape_sequence: $ => token.immediate(seq(
      '\\',
      choice(
        /['"\\nrtbfv0]/,
        /x[0-9a-fA-F]{2}/,
        /u[0-9a-fA-F]{4}/,
        /U[0-9a-fA-F]{8}/,
      ),
    )),

    fstring: $ => choice(
      $._fstring_double,
      $._fstring_single,
      $._long_fstring_double,
      $._long_fstring_single,
    ),

    _fstring_double: $ => seq(/[fF]"/, repeat(choice($.escape_sequence, $.interpolation, $.fstring_text_double)), '"'),
    _fstring_single: $ => seq(/[fF]'/, repeat(choice($.escape_sequence, $.interpolation, $.fstring_text_single)), "'"),
    // Long f-strings with interpolation support
    _long_fstring_double: $ => seq(
      /[fF]"""/,
      repeat(choice(
        $.escape_sequence,
        $.interpolation,
        $.fstring_text_long_double,
      )),
      '"""'
    ),
    _long_fstring_single: $ => seq(
      /[fF]'''/,
      repeat(choice(
        $.escape_sequence,
        $.interpolation,
        $.fstring_text_long_single,
      )),
      "'''"
    ),

    // F-string literal text nodes (named so they appear in AST)
    // Use choice to handle ! separately to avoid conflict with != operator
    fstring_text_double: $ => token(prec(100, choice(
      /[^"\\{!]+/,       // Regular text without !
      /!/,               // Single ! character
    ))),
    fstring_text_single: $ => token(prec(100, choice(
      /[^'\\{!]+/,       // Regular text without !
      /!/,               // Single ! character
    ))),
    fstring_text_long_double: $ => token(prec(100, choice(
      /[^"\\{!]+/,       // Non-special chars (excluding !)
      /!/,               // Single ! character
      /"[^"]/,           // Single quote not followed by another
      /""[^"]/,          // Two quotes not followed by third
    ))),
    fstring_text_long_single: $ => token(prec(100, choice(
      /[^'\\{!]+/,
      /!/,               // Single ! character
      /'[^']/,
      /''[^']/,
    ))),

    interpolation: $ => seq(
      '{',
      $._expression,
      optional($.fstring_debug),
      optional(seq('!', $.fstring_conversion)),
      optional(seq(':', $.format_spec)),
      '}'
    ),

    // Debug flag: f"{x=}" shows "x=42"
    fstring_debug: $ => '=',

    // Conversion flag: !r (repr), !s (str), !a (ascii)
    fstring_conversion: $ => /[rsa]/,

    // Format specification for f-string interpolations
    // token.immediate prevents extras (comments) from matching inside the spec
    // Examples: .2f, >10, ^20, 0>8, #x
    format_spec: $ => token.immediate(prec(200, /[^}]+/)),

    bstring: $ => choice(
      $._bstring_double,
      $._bstring_single,
      $._long_bstring_double,
      $._long_bstring_single,
    ),

    _bstring_double: $ => seq(/[bB]"/, repeat(choice($.escape_sequence, /[^"\\]+/)), '"'),
    _bstring_single: $ => seq(/[bB]'/, repeat(choice($.escape_sequence, /[^'\\]+/)), "'"),
    _long_bstring_double: $ => token(seq(/[bB]"""/, /([^"\\]|\\.|"[^"]|""[^"])*/, '"""')),
    _long_bstring_single: $ => token(seq(/[bB]'''/, /([^'\\]|\\.|'[^']|''[^'])*/, "'''")),

    identifier: $ => /[a-zA-Z_]\w*/,

    comment: $ => token(seq('#', /.*/)),
  },
});
