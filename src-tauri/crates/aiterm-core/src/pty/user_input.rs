//! 辨認「使用者在這個終端機裡跟 AI 對話」：統計使用者（不是程式）送出的、夠長的整行輸入。
//!
//! 派工的 Agent 卡預設自動結案（Claude 回完一輪就響 bell）。使用者進去分頁打字討論時，
//! 第一輪回完卡片就會被判成完成、工作資料夾被自動 commit，使用者的對話卻還在進行。
//! 監看靠這裡的計數判斷「使用者接手了」，之後改成互動卡的規則（不靠 bell 與逾時結案）。
//!
//! 只算「夠長的一行」：回答權限詢問只要按一個數字、`y` 或 Enter，那不是對話，不能因此接手。

/// 一行至少要有幾個可見字元才算對話（一個中文字算一個）。
pub const MIN_LINE_CHARS: usize = 4;

#[derive(Debug, Default)]
pub struct UserInputTracker {
    pending: usize,
    lines: u64,
    escape: Escape,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Escape {
    #[default]
    None,
    /// 剛收到 ESC。
    Esc,
    /// CSI（`ESC [`）：一路吃到最後一個位元組（0x40–0x7e）。方向鍵、滑鼠、括號貼上標記都是這種。
    Csi,
    /// SS3（`ESC O`）：再吃一個位元組。
    Ss3,
}

impl UserInputTracker {
    /// 使用者送出的位元組。換行（Enter）時，這一行夠長就算一行對話。
    pub fn feed(&mut self, data: &[u8]) {
        for &b in data {
            match self.escape {
                Escape::Esc => {
                    self.escape = match b {
                        b'[' => Escape::Csi,
                        b'O' => Escape::Ss3,
                        _ => Escape::None,
                    };
                    continue;
                }
                Escape::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        self.escape = Escape::None;
                    }
                    continue;
                }
                Escape::Ss3 => {
                    self.escape = Escape::None;
                    continue;
                }
                Escape::None => {}
            }
            match b {
                0x1b => self.escape = Escape::Esc,
                b'\r' | b'\n' => {
                    if self.pending >= MIN_LINE_CHARS {
                        self.lines += 1;
                    }
                    self.pending = 0;
                }
                0x7f | 0x08 => self.pending = self.pending.saturating_sub(1),
                // Ctrl+C／Ctrl+U：這一行作廢。
                0x03 | 0x15 => self.pending = 0,
                b if b < 0x20 => {}
                // UTF-8 的延續位元組不算新字元。
                b if b & 0xC0 == 0x80 => {}
                _ => self.pending += 1,
            }
        }
    }

    /// 到目前為止，使用者送出了幾行夠長的輸入。
    pub fn lines(&self) -> u64 {
        self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_after(inputs: &[&str]) -> u64 {
        let mut t = UserInputTracker::default();
        for i in inputs {
            t.feed(i.as_bytes());
        }
        t.lines()
    }

    #[test]
    fn a_typed_message_with_enter_counts_as_one_line() {
        assert_eq!(lines_after(&["請幫我改一下這裡的邏輯\r"]), 1);
        assert_eq!(lines_after(&["fix the", " bug\r", "and more text\r"]), 2);
    }

    #[test]
    fn answering_a_prompt_with_a_digit_or_enter_is_not_a_conversation() {
        assert_eq!(lines_after(&["1\r"]), 0);
        assert_eq!(lines_after(&["y\r"]), 0);
        assert_eq!(lines_after(&["\r"]), 0);
        assert_eq!(lines_after(&["\r\r\r"]), 0);
    }

    #[test]
    fn a_line_without_enter_is_not_counted_yet() {
        assert_eq!(lines_after(&["請幫我改一下這裡的邏輯"]), 0);
    }

    #[test]
    fn chinese_characters_count_once_each_not_per_byte() {
        // 3 個中文字 = 9 個位元組，但只有 3 個字元：不夠長。
        assert_eq!(lines_after(&["繼續做\r"]), 0);
        assert_eq!(lines_after(&["繼續做吧\r"]), 1);
    }

    #[test]
    fn arrow_keys_mouse_and_paste_markers_are_not_text() {
        // 方向鍵、滑鼠回報、括號貼上標記都不能當成輸入的字元。
        assert_eq!(lines_after(&["\x1b[A\x1b[B\x1b[C\x1b[D\r"]), 0);
        assert_eq!(lines_after(&["\x1b[<0;10;5M\x1b[<0;10;5m\r"]), 0);
        assert_eq!(lines_after(&["\x1bOP\x1bOQ\r"]), 0);
        // 括號貼上：標記不算，裡面的文字算。
        assert_eq!(lines_after(&["\x1b[200~請幫我看一下這段\x1b[201~\r"]), 1);
    }

    #[test]
    fn backspace_and_cancel_shorten_or_discard_the_line() {
        assert_eq!(lines_after(&["abcd\x7f\x7f\r"]), 0);
        assert_eq!(lines_after(&["abcdef\x03\r"]), 0);
        assert_eq!(lines_after(&["abcdef\x15\r"]), 0);
        assert_eq!(lines_after(&["abcdef\x7f\r"]), 1);
    }

    #[test]
    fn an_escape_sequence_split_across_writes_is_still_ignored() {
        assert_eq!(lines_after(&["\x1b", "[", "A", "\r"]), 0);
        assert_eq!(lines_after(&["\x1b[20", "0~hello world\x1b[2", "01~\r"]), 1);
    }
}
