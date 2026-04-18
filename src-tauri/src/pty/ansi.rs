//! ANSI escape sequence stripper for terminal output.
//!
//! Used to clean PTY output before sending it to the AI as context.
//! Handles CSI, OSC, and other common escape sequences.

/// Strip ANSI/VT escape sequences from terminal output.
///
/// Handles:
/// - CSI sequences: `ESC [ ... final_byte` (colors, cursor movement, etc.)
/// - OSC sequences: `ESC ] ... BEL` or `ESC ] ... ESC \` (title, hyperlinks)
/// - Character set designations: `ESC ( X`, `ESC ) X`
/// - All other two-char ESC sequences
/// - Bare CR characters (collapsed into the surrounding newline structure)
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        match bytes[i] {
            // ESC — start of an escape sequence
            0x1b => {
                i += 1;
                if i >= len {
                    break;
                }
                match bytes[i] {
                    // CSI — ESC [ ... <final 0x40-0x7E>
                    b'[' => {
                        i += 1;
                        while i < len {
                            let b = bytes[i];
                            i += 1;
                            if (0x40..=0x7e).contains(&b) {
                                break; // final byte consumed
                            }
                        }
                    }
                    // OSC — ESC ] ... BEL  or  ESC ] ... ESC \
                    b']' => {
                        i += 1;
                        while i < len {
                            if bytes[i] == 0x07 {
                                i += 1;
                                break;
                            }
                            if bytes[i] == 0x1b && i + 1 < len && bytes[i + 1] == b'\\' {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    // DCS — ESC P ... ST  (device control string)
                    b'P' => {
                        i += 1;
                        while i < len {
                            if bytes[i] == 0x1b && i + 1 < len && bytes[i + 1] == b'\\' {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    // Character set designations: ESC ( X, ESC ) X, etc.
                    b'(' | b')' | b'*' | b'+' => {
                        i += 1; // skip the designator byte
                        if i < len {
                            i += 1; // skip the charset byte
                        }
                    }
                    // SS3 — ESC O <byte>  (function keys)
                    b'O' => {
                        i += 1;
                        if i < len {
                            i += 1;
                        }
                    }
                    // All other two-char ESC sequences: skip the second byte
                    _ => {
                        i += 1;
                    }
                }
            }
            // Bare CR: skip (the \n companion, if any, is kept)
            0x0d => {
                i += 1;
            }
            // Regular byte — may be the first byte of a multi-byte UTF-8 sequence
            _ => {
                // Safe to index into the str at byte boundary since we only land
                // here on non-ESC, non-CR bytes — either ASCII or first UTF-8 byte.
                let ch = match input[i..].chars().next() {
                    Some(c) => c,
                    None => break,
                };
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }

    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_unchanged() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }

    #[test]
    fn strips_sgr_color_codes() {
        // ESC [ 3 1 m = red foreground, ESC [ 0 m = reset
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn strips_cursor_movement() {
        // ESC [ H = cursor home, ESC [ 2 J = clear screen
        assert_eq!(strip_ansi("\x1b[H\x1b[2Jtext"), "text");
    }

    #[test]
    fn strips_osc_title() {
        // ESC ] 0 ; title BEL
        assert_eq!(strip_ansi("\x1b]0;My Terminal\x07text"), "text");
    }

    #[test]
    fn strips_osc_with_st_terminator() {
        assert_eq!(strip_ansi("\x1b]0;title\x1b\\text"), "text");
    }

    #[test]
    fn strips_bare_cr() {
        assert_eq!(strip_ansi("line1\r\nline2"), "line1\nline2");
    }

    #[test]
    fn strips_mixed_sequences() {
        let input = "\x1b[32m$\x1b[0m ls -la\r\n\x1b[1mfoo\x1b[0m  bar";
        let output = strip_ansi(input);
        assert_eq!(output, "$ ls -la\nfoo  bar");
    }

    #[test]
    fn preserves_unicode() {
        assert_eq!(strip_ansi("\x1b[31m你好\x1b[0m"), "你好");
    }

    #[test]
    fn empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn lone_esc_at_end() {
        // Should not panic on truncated escape at end of input
        let result = strip_ansi("text\x1b");
        assert_eq!(result, "text");
    }

    #[test]
    fn strips_ss3_function_key() {
        // ESC O A = cursor up (SS3 + A)
        assert_eq!(strip_ansi("\x1bOAtext"), "text");
    }

    #[test]
    fn strips_charset_designation() {
        // ESC ( B = US ASCII charset
        assert_eq!(strip_ansi("\x1b(Btext"), "text");
    }
}
