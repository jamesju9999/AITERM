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
        // 迴圈不變式：i 必須落在字元邊界，否則下面 `input[i..]` 會 panic。
        //
        // 下面每個 ESC 分支都是以原始位元組數推進的（例如 catch-all 的
        // `ESC <byte>` 固定跳兩個位元組），所以遇到「ESC 後面緊接一個多位元組
        // 字元」時，會把那個字元的首位元組當成 escape 的一部分吃掉，讓 i 停在
        // 接續位元組上。二進位內容經 String::from_utf8_lossy 轉換後滿是 U+FFFD
        // （EF BF BD），撞上的機率很高——實機上 agent 讀到一個 .msg 檔就讓整個
        // app abort 了。
        //
        // 首位元組已經被吞掉的那個字元本來就殘缺，剩下的接續位元組是垃圾，
        // 跳過即可。
        if !input.is_char_boundary(i) {
            i += 1;
            continue;
        }
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

/// Tracks the DEC private modes (`CSI ? Ps h` / `CSI ? Ps l`) a PTY stream has
/// switched on or off, so a replay that starts mid-stream can restore them.
///
/// **Why this exists.** Screen sharing primes a new viewer from
/// `PtySession::subscribe_with_history`, which replays the tail of a fixed-size
/// ring of raw bytes. Mode switches are *sticky state expressed once*: a
/// full-screen program emits `\x1b[?1049h` at startup and never again. Measured
/// against the real Claude Code CLI, that byte lands at offset 855 of the
/// session and one full 45x160 redraw costs ~2.2 KB — so after roughly a
/// hundred redraws it has been evicted from the ring and the viewer, having
/// never seen it, renders the alternate-screen content into its normal buffer.
/// The visible symptom was the remote tab's live pane staying three rows tall
/// instead of going full height. A bigger ring only moves the threshold; this
/// tracker is never evicted.
///
/// Every DEC private mode is tracked, not an allowlist: the value recorded is
/// exactly "what the host's own terminal was last told", which is precisely
/// what the viewer needs, and there is no list to keep in sync as programs
/// start using new modes.
#[derive(Default)]
pub struct DecModeTracker {
    /// Last value seen per mode number. `BTreeMap` for a deterministic
    /// prefix — the same state must always produce the same bytes.
    modes: std::collections::BTreeMap<u16, bool>,
    /// Parser state carried across chunk boundaries. The reader thread hands
    /// us 4 KB reads, which cut escape sequences in half regularly.
    state: DecParseState,
    /// Digits of the parameter currently being accumulated.
    digits: String,
    /// Parameters already collected from the current sequence (`?1000;1002;1006h`).
    params: Vec<u16>,
}

#[derive(Default)]
enum DecParseState {
    /// Not inside an escape sequence.
    #[default]
    Ground,
    /// Saw ESC.
    Esc,
    /// Saw `ESC [`, still deciding whether a `?` follows.
    Csi,
    /// Saw `ESC [ ?` — collecting parameters until the final byte.
    DecParams,
    /// Inside a CSI that is not a DEC private mode; skip to its final byte.
    CsiIgnore,
}

impl DecModeTracker {
    /// Feed one raw output chunk. Safe to split anywhere.
    pub fn feed(&mut self, chunk: &[u8]) {
        for &b in chunk {
            match self.state {
                DecParseState::Ground => {
                    if b == 0x1b {
                        self.state = DecParseState::Esc;
                    }
                }
                DecParseState::Esc => {
                    self.state = if b == b'[' { DecParseState::Csi } else { DecParseState::Ground };
                }
                DecParseState::Csi => {
                    if b == b'?' {
                        self.digits.clear();
                        self.params.clear();
                        self.state = DecParseState::DecParams;
                    } else if (0x40..=0x7e).contains(&b) {
                        // A complete CSI with no parameters at all.
                        self.state = DecParseState::Ground;
                    } else {
                        self.state = DecParseState::CsiIgnore;
                    }
                }
                DecParseState::CsiIgnore => {
                    if (0x40..=0x7e).contains(&b) {
                        self.state = DecParseState::Ground;
                    }
                }
                DecParseState::DecParams => {
                    if b.is_ascii_digit() {
                        self.digits.push(b as char);
                    } else if b == b';' {
                        self.push_param();
                    } else if b == b'h' || b == b'l' {
                        self.push_param();
                        let on = b == b'h';
                        for m in self.params.drain(..) {
                            self.modes.insert(m, on);
                        }
                        self.state = DecParseState::Ground;
                    } else {
                        // Some other DEC private sequence (`?25$p`, `?1;2r`,
                        // ...) — not a mode set/reset, drop it.
                        if (0x40..=0x7e).contains(&b) {
                            self.state = DecParseState::Ground;
                        }
                        self.digits.clear();
                        self.params.clear();
                    }
                }
            }
        }
    }

    fn push_param(&mut self) {
        // An empty or absurd parameter is not a mode; ignore it rather than
        // recording a bogus number.
        if let Ok(n) = self.digits.parse::<u16>() {
            self.params.push(n);
        }
        self.digits.clear();
    }

    /// The escape sequences that reproduce the tracked state on a fresh
    /// terminal. Empty when nothing has been seen.
    ///
    /// Modes that were explicitly reset are emitted too: several DEC modes
    /// (autowrap, cursor visibility) default to *on*, so "not emitted" is not
    /// the same as "off".
    pub fn prefix(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (mode, on) in &self.modes {
            out.extend_from_slice(format!("\x1b[?{mode}{}", if *on { 'h' } else { 'l' }).as_bytes());
        }
        out
    }
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

    /// 實機當掉過：agent 讀到二進位檔（Outlook .msg），輸出灌進 PTY 後這裡
    /// panic「byte index N is not a char boundary」，整個 app 直接 abort。
    ///
    /// 成因：所有 ESC 分支都以原始位元組數推進，完全不看字元邊界。catch-all
    /// 的 `ESC <byte>` 會把後面那個多位元組字元的「首位元組」當成 escape 的第二
    /// 個字元吃掉，i 就落在字元中間，下一圈 input[i..] 直接炸。二進位資料經
    /// lossy UTF-8 轉換後滿是 U+FFFD（EF BF BD），撞上的機率很高。
    #[test]
    fn does_not_panic_when_esc_is_followed_by_a_multibyte_char() {
        // ESC 之後緊接 U+FFFD：catch-all 分支吃掉 0xEF，i 落在 0xBF 上。
        assert_eq!(strip_ansi("\x1b\u{FFFD}tail"), "tail");
        // 同樣的形狀，換成一般的中文字元。
        assert_eq!(strip_ansi("\x1b中text"), "text");
        // 固定寬度的另外兩個分支也有同樣的問題。
        assert_eq!(strip_ansi("\x1bO\u{FFFD}x"), "x");
        assert_eq!(strip_ansi("\x1b(\u{FFFD}y"), "y");
    }

    /// 這是上面那個 panic 的真實來源形狀：二進位內容經 lossy 轉換後，散落的
    /// U+FFFD 與控制位元組混在一起。不要求輸出內容，只要求「不准 panic」。
    #[test]
    fn survives_lossy_converted_binary_garbage() {
        let raw: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        let lossy = String::from_utf8_lossy(&raw).into_owned();
        let _ = strip_ansi(&lossy); // must not panic
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

    // ── DecModeTracker ────────────────────────────────────────────────────

    fn tracked(chunks: &[&[u8]]) -> Vec<u8> {
        let mut t = DecModeTracker::default();
        for c in chunks {
            t.feed(c);
        }
        t.prefix()
    }

    #[test]
    fn dec_tracker_starts_empty() {
        assert_eq!(tracked(&[b"plain output\r\n"]), b"");
    }

    #[test]
    fn dec_tracker_records_alt_screen_entry() {
        assert_eq!(tracked(&[b"\x1b[?1049h"]), b"\x1b[?1049h");
    }

    /// The reader thread reads in 4 KB chunks, so a sequence split across two
    /// reads is routine, not a corner case.
    #[test]
    fn dec_tracker_survives_split_across_chunks() {
        assert_eq!(tracked(&[b"\x1b[?10", b"49h"]), b"\x1b[?1049h");
        assert_eq!(tracked(&[b"\x1b", b"[", b"?", b"1", b"0", b"4", b"9", b"h"]), b"\x1b[?1049h");
    }

    #[test]
    fn dec_tracker_keeps_the_last_value_for_a_mode() {
        assert_eq!(tracked(&[b"\x1b[?1049h", b"\x1b[?1049l"]), b"\x1b[?1049l");
        assert_eq!(tracked(&[b"\x1b[?1049l", b"\x1b[?1049h"]), b"\x1b[?1049h");
    }

    #[test]
    fn dec_tracker_splits_multi_parameter_sequences() {
        assert_eq!(
            tracked(&[b"\x1b[?1000;1002;1006h"]),
            b"\x1b[?1000h\x1b[?1002h\x1b[?1006h",
        );
    }

    /// Ordered by mode number so the same state always yields the same bytes.
    #[test]
    fn dec_tracker_prefix_is_deterministic() {
        assert_eq!(
            tracked(&[b"\x1b[?2004h\x1b[?25l\x1b[?1049h"]),
            b"\x1b[?25l\x1b[?1049h\x1b[?2004h",
        );
    }

    /// Non-DEC CSI sequences (SGR colour, erase, cursor moves) must not leak
    /// into the tracked state — Claude Code's output is mostly these.
    #[test]
    fn dec_tracker_ignores_ordinary_csi() {
        assert_eq!(tracked(&[b"\x1b[31mred\x1b[0m\x1b[2J\x1b[1;1H\x1b[38;2;255;193;7m"]), b"");
    }

    /// `CSI ? Ps $ p` (DECRQM) asks about a mode, it does not set one.
    #[test]
    fn dec_tracker_ignores_mode_queries() {
        assert_eq!(tracked(&[b"\x1b[?1049$p"]), b"");
    }

    /// The real thing: the bytes Claude Code CLI actually emits at startup,
    /// captured from a pty run.
    #[test]
    fn dec_tracker_captures_claude_code_startup_modes() {
        let prefix = tracked(&[b"\x1b[?25h\x1b[?25l\x1b[?2004h\x1b[?2031h\x1b[?1004h\x1b[?1049h\x1b[?1007h"]);
        let s = String::from_utf8(prefix).unwrap();
        assert!(s.contains("\x1b[?1049h"), "alt screen must be restored: {s:?}");
        assert!(s.contains("\x1b[?2004h"), "bracketed paste must be restored: {s:?}");
        assert!(s.contains("\x1b[?25l"), "hidden cursor must stay hidden: {s:?}");
    }
}
