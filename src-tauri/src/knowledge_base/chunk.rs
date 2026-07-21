const TARGET_CHUNK_CHARS: usize = 3200;
const OVERLAP_CHARS: usize = 600;

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub text: String,
    pub location_hint: Option<String>,
}

/// 將轉換後的 markdown 切成帶重疊的片段，優先沿標題邊界累積，
/// 超過 TARGET_CHUNK_CHARS 就切一刀，並保留 OVERLAP_CHARS 字元的重疊
/// 給下一個 chunk 以維持上下文連續性。
pub fn chunk_markdown(markdown: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut buffer = String::new();
    let mut buffer_heading: Option<String> = None;

    for line in markdown.lines() {
        if let Some(heading) = parse_heading(line) {
            current_heading = Some(heading);
            buffer_heading = current_heading.clone();
        }
        if buffer.is_empty() {
            buffer_heading = current_heading.clone();
        }
        buffer.push_str(line);
        buffer.push('\n');

        if buffer.chars().count() >= TARGET_CHUNK_CHARS {
            chunks.push(Chunk {
                text: buffer.trim_end().to_string(),
                location_hint: buffer_heading.clone(),
            });
            buffer = tail_chars(&buffer, OVERLAP_CHARS);
            buffer_heading = current_heading.clone();
        }
    }

    if !buffer.trim().is_empty() {
        chunks.push(Chunk {
            text: buffer.trim_end().to_string(),
            location_hint: buffer_heading,
        });
    }

    chunks
}

fn parse_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        let text = trimmed.trim_start_matches('#').trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    None
}

fn tail_chars(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        s.to_string()
    } else {
        chars[chars.len() - n..].iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_markdown_becomes_single_chunk() {
        let md = "# Title\n\nSome short content.";
        let chunks = chunk_markdown(md);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].location_hint.as_deref(), Some("Title"));
        assert!(chunks[0].text.contains("Some short content."));
    }

    #[test]
    fn empty_markdown_returns_no_chunks() {
        let chunks = chunk_markdown("   \n  ");
        assert!(chunks.is_empty());
    }

    #[test]
    fn long_markdown_splits_with_overlap() {
        // 產生遠超過 TARGET_CHUNK_CHARS 的內容
        let paragraph = "這是一段測試內容，用來確保切片邏輯能正確處理長文件。";
        let mut md = String::from("# 第一章\n\n");
        for _ in 0..200 {
            md.push_str(paragraph);
            md.push('\n');
        }

        let chunks = chunk_markdown(&md);
        assert!(chunks.len() > 1, "long content should split into multiple chunks");

        // 每個 chunk 都應該標記正確的最近標題
        for c in &chunks {
            assert_eq!(c.location_hint.as_deref(), Some("第一章"));
        }

        // 檢查有 overlap：後一個 chunk 的開頭應該與前一個 chunk 的結尾有重疊字元
        let first_tail: String = chunks[0].text.chars().rev().take(50).collect();
        let first_tail_reversed: String = first_tail.chars().rev().collect();
        let overlap_sample: String = first_tail_reversed.chars().take(20).collect();
        assert!(
            chunks[1].text.contains(&overlap_sample),
            "expected overlap between consecutive chunks"
        );
    }

    #[test]
    fn location_hint_tracks_nearest_heading() {
        let md = "# A\n\ncontent under A\n\n## B\n\ncontent under B";
        let chunks = chunk_markdown(md);
        // 內容量小，只會產生一個 chunk，但驗證切片函式至少能正確解析最後一個標題
        assert_eq!(chunks.last().unwrap().location_hint.as_deref(), Some("B"));
    }
}
