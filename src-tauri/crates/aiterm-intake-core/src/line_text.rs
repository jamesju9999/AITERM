//! LINE 文字訊息不支援 Markdown，符號會原樣顯示。送出前把常見的 Markdown 轉成好讀的純文字。
use regex::Regex;
use std::sync::OnceLock;

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

/// 給 AI 的提示：回覆會以 LINE 純文字送出。
pub const PLAIN_NOTE: &str = "\n\n這段回答會以 LINE 純文字送出，LINE 不會渲染 Markdown：請不要使用 **粗體**、# 標題、表格、程式碼圍欄，\
     用換行與「・」條列即可。";

pub fn to_plain(md: &str) -> String {
    static LINK: OnceLock<Regex> = OnceLock::new();
    static BOLD: OnceLock<Regex> = OnceLock::new();
    static BOLD_U: OnceLock<Regex> = OnceLock::new();
    static ITAL: OnceLock<Regex> = OnceLock::new();
    static CODE: OnceLock<Regex> = OnceLock::new();
    static HEAD: OnceLock<Regex> = OnceLock::new();
    static BULLET: OnceLock<Regex> = OnceLock::new();
    static SEP: OnceLock<Regex> = OnceLock::new();
    static BLANKS: OnceLock<Regex> = OnceLock::new();

    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in md.replace("\r\n", "\n").lines() {
        let t = line.trim_start();
        if t.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            out.push(line.to_string());
            continue;
        }
        // 表格分隔列（|---|---|）整行丟掉。
        if re(&SEP, r"^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$").is_match(line) && line.contains('-') && (line.contains('|') || t.starts_with("---")) {
            if t.starts_with("---") && !line.contains('|') {
                continue; // 水平線
            }
            continue;
        }
        let mut s = line.to_string();
        // 表格列：| a | b | → a｜b
        let tt = s.trim();
        if tt.starts_with('|') && tt.ends_with('|') && tt.len() > 1 {
            let cells: Vec<&str> = tt.trim_matches('|').split('|').map(|c| c.trim()).collect();
            s = cells.join("｜");
        }
        if let Some(c) = re(&HEAD, r"^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$").captures(&s) {
            s = format!("【{}】", &c[1]);
        }
        s = re(&BULLET, r"^(\s*)[-*+]\s+").replace(&s, "${1}・").into_owned();
        s = re(&LINK, r"\[([^\]]+)\]\((https?://[^)\s]+)\)").replace_all(&s, "$1 $2").into_owned();
        s = re(&BOLD, r"\*\*(.+?)\*\*").replace_all(&s, "$1").into_owned();
        s = re(&BOLD_U, r"__(.+?)__").replace_all(&s, "$1").into_owned();
        s = re(&ITAL, r"(^|[^*\w])\*([^*\s][^*]*?)\*($|[^*\w])").replace_all(&s, "$1$2$3").into_owned();
        s = re(&CODE, r"`([^`]+)`").replace_all(&s, "$1").into_owned();
        out.push(s);
    }
    let joined = out.join("\n");
    re(&BLANKS, r"\n{3,}").replace_all(&joined, "\n\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_bold_italic_and_inline_code() {
        assert_eq!(to_plain("請按 **確定**，再執行 `ipconfig`，*謝謝*"), "請按 確定，再執行 ipconfig，謝謝");
    }

    #[test]
    fn headings_and_bullets() {
        assert_eq!(to_plain("## 步驟\n- 開啟設定\n* 重新啟動\n  - 子項"), "【步驟】\n・開啟設定\n・重新啟動\n  ・子項");
    }

    #[test]
    fn code_fence_is_dropped_but_content_kept_verbatim() {
        assert_eq!(to_plain("執行：\n```bash\nls **x**\n```\n完成"), "執行：\nls **x**\n完成");
    }

    #[test]
    fn tables_become_lines() {
        assert_eq!(to_plain("| 欄位 | 值 |\n|---|---|\n| a | b |"), "欄位｜值\na｜b");
    }

    #[test]
    fn links_keep_text_and_url() {
        assert_eq!(to_plain("看[說明](https://example.com/a)"), "看說明 https://example.com/a");
    }

    #[test]
    fn plain_text_and_math_are_untouched() {
        let s = "價格 2 * 3 = 6，檔案 a_b_c.txt，網址 https://example.com/a_b";
        assert_eq!(to_plain(s), s);
    }

    #[test]
    fn horizontal_rule_and_extra_blank_lines_collapse() {
        assert_eq!(to_plain("a\n\n---\n\n\n\nb"), "a\n\nb");
    }
}
