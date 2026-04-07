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

        // Check if followed by a continuation keyword.
        // Must match the FULL keyword and verify the next char is not
        // alphanumeric (otherwise "finally" would swallow "find", etc.).
        // Since mark_end was called above, these advances are pure lookahead.
        {
            // Collect up to 8 chars of the next token (longest keyword: "finally" = 7)
            char buf[9];
            int len = 0;
            while (len < 8 && ((lexer->lookahead >= 'a' && lexer->lookahead <= 'z') ||
                               (lexer->lookahead >= 'A' && lexer->lookahead <= 'Z') ||
                               (lexer->lookahead >= '0' && lexer->lookahead <= '9') ||
                               lexer->lookahead == '_')) {
                buf[len++] = (char)lexer->lookahead;
                lexer->advance(lexer, false);
            }
            buf[len] = '\0';

            // The keyword must be an exact match (not a prefix of a longer identifier).
            // After reading the keyword chars, lookahead is already the next char.
            // If len matches exactly, the word boundary is guaranteed because we
            // stopped at a non-alnum char.
            if ((len == 4 && (memcmp(buf, "else", 4) == 0 || memcmp(buf, "elif", 4) == 0)) ||
                (len == 6 && memcmp(buf, "except", 6) == 0) ||
                (len == 7 && memcmp(buf, "finally", 7) == 0)) {
                return false;
            }
        }

        // Not a continuation keyword - this is a significant newline
        return true;
    }

    return false;
}
