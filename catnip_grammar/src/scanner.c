/* FILE: catnip_grammar/src/scanner.c */
// External scanner for Catnip grammar
// Handles significant newlines as statement separators
// Newlines inside parentheses/brackets/braces are NOT significant (like Python)
// Newlines before 'else' or 'elif' are NOT significant (continuation)

#include <tree_sitter/parser.h>
#include <wctype.h>
#include <stdbool.h>
#include <string.h>

enum TokenType {
    NEWLINE,
};

void *tree_sitter_catnip_external_scanner_create() { return NULL; }
void tree_sitter_catnip_external_scanner_destroy(void *p) { (void)p; }
void tree_sitter_catnip_external_scanner_reset(void *p) { (void)p; }
unsigned tree_sitter_catnip_external_scanner_serialize(void *p, char *buffer) { (void)p; (void)buffer; return 0; }
void tree_sitter_catnip_external_scanner_deserialize(void *p, const char *b, unsigned n) { (void)p; (void)b; (void)n; }

// Check if we're at a significant newline (separates statements)
bool tree_sitter_catnip_external_scanner_scan(void *payload, TSLexer *lexer,
                                                const bool *valid_symbols) {
    (void)payload;

    // Only try to match NEWLINE when it's valid
    if (!valid_symbols[NEWLINE]) {
        return false;
    }

    // Skip horizontal whitespace only
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
        lexer->advance(lexer, true);
    }

    // Check if we have a newline
    if (lexer->lookahead == '\n' || lexer->lookahead == '\r') {
        lexer->advance(lexer, false);

        // Handle \r\n
        if (lexer->lookahead == '\n') {
            lexer->advance(lexer, false);
        }

        // IMPORTANT: Mark end BEFORE lookahead for else/elif
        // This ensures we don't consume too much
        lexer->result_symbol = NEWLINE;
        lexer->mark_end(lexer);

        // Now check if followed by else/elif (optional lookahead, limited)
        // Skip whitespace after newline
        int space_count = 0;
        while ((lexer->lookahead == ' ' || lexer->lookahead == '\t') && space_count < 50) {
            lexer->advance(lexer, true);
            space_count++;
        }

        // Check for continuation tokens: line starting with '.' chains
        // the previous expression (method call, broadcast, attribute access)
        if (lexer->lookahead == '.') {
            return false;
        }

        // Check if followed by 'else' or 'elif' (lightweight lookahead)
        // We check first 3-4 chars to distinguish from regular identifiers starting with 'e'
        if (lexer->lookahead == 'e') {
            lexer->advance(lexer, false);
            if (lexer->lookahead == 'l') {
                lexer->advance(lexer, false);
                if (lexer->lookahead == 's' || lexer->lookahead == 'i') {
                    // Looks like 'els...' or 'eli...' - probably else/elif
                    // Don't treat newline as significant
                    return false;
                }
            }
            // Just 'e' followed by something else - not else/elif
            // Fall through to return true
        }

        // Not followed by else/elif - this is a significant newline
        return true;
    }

    return false;
}
