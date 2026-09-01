//! Artifact 協定的說明文字。三個 prompt builder（`commands/ai.rs` 的
//! `build_chat_prompt`、`code_assistant` 與 `knowledge_base` 各自的
//! `build_system_prompt`）共用同一份，避免同樣的說明散在各處各自漂移。
//!
//! 文字維持英文，跟這三個 prompt 的既有慣例一致——規則本文一律英文，輸出語言
//! 另外由各自的 "Respond in {language}" 規則管。

/// 教模型怎麼輸出會被 artifact 面板渲染的 fenced code block。
/// 呼叫端自行決定要不要接上去（`build_chat_prompt` 有旗標，另外兩個無條件接）。
pub fn artifact_protocol_section() -> &'static str {
    r#"
## Rendering documents and charts

`artifact-html` and `artifact-chart` are NOT tools. Never call them as a tool
and never put them in a tool call — there is no such tool and the call will
fail. They are markdown code-fence languages that you type directly into the
text of your reply, exactly like writing a ```json block. The app watches your
reply text for these two fences and renders them in a panel beside the
conversation.

Write a document like this, as literal text in your reply:

```artifact-html
<!DOCTYPE html>
<html><head><title>Quarterly Summary</title></head>
<body><h1>Quarterly Summary</h1><p>…</p></body></html>
```

The HTML is rendered in an isolated sandbox. Use it for reports, formatted
summaries, comparison tables, or anything worth reading as its own document.
The <title> becomes the panel's title. <style> and <script> both work, but the
page cannot reach anything outside its own frame.

Write a chart like this, again as literal text in your reply:

```artifact-chart
{"type":"bar","title":"Sales by month",
 "data":[{"month":"Jan","sales":120},{"month":"Feb","sales":150}],
 "xKey":"month","series":[{"key":"sales","label":"Sales"}]}
```

`type` is "bar", "line" or "pie"; `data` is an array of row objects; `xKey`
names the field used for the category axis; each series `key` names a numeric
field on those rows.

Rules:
- At most one artifact per reply; a later one replaces the earlier one.
- Do NOT use an artifact for a short answer, a single command, or a couple of
  sentences — those belong in the reply itself.
- Always also write a line or two in the reply saying what you produced. The
  artifact supplements the answer, it is not a substitute for one.
- Write the whole fence in one go, and only claim the document is ready after
  you have actually written it — not before.
- Never build a document by shelling out — no `cat > file.html <<EOF`, no
  writing HTML through a terminal command. Write the fence in your reply
  instead; the app renders and offers to save it.
- If you are working through a multi-step task, keep going until the work is
  done. The artifact is how you present the finished result, not a reason to
  stop early."#
}

#[cfg(test)]
mod tests {
    use super::artifact_protocol_section;

    /// 兩個 fence 名稱是協定本身，前端 `src/lib/markdown.tsx` 的 code renderer
    /// 就是比對這兩個字串。這裡釘住它們，避免哪天有人「順手」改了措辭卻讓模型
    /// 學到一個前端根本不認得的名字。
    #[test]
    fn mentions_both_fence_languages() {
        let s = artifact_protocol_section();
        assert!(s.contains("artifact-html"), "must teach the artifact-html fence");
        assert!(s.contains("artifact-chart"), "must teach the artifact-chart fence");
    }

    /// 圖表 JSON 的欄位名也是協定：前端 `ArtifactChart.tsx` 的 `ChartSpec`
    /// 就是照這些名字解析的。
    #[test]
    fn documents_the_chart_spec_fields() {
        let s = artifact_protocol_section();
        for field in ["\"type\"", "\"data\"", "\"xKey\"", "\"series\""] {
            assert!(s.contains(field), "chart spec must document {field}");
        }
        for kind in ["\"bar\"", "\"line\"", "\"pie\""] {
            assert!(s.contains(kind), "chart spec must list the {kind} chart type");
        }
    }

    /// 沒有這條，模型很容易把每個回答都包成 artifact。
    #[test]
    fn warns_against_using_artifacts_for_short_answers() {
        assert!(artifact_protocol_section().contains("Do NOT use an artifact for a short answer"));
    }

    /// 知識庫與 Code Assistant 是 native tool-calling 的迴圈。實測發現模型會把
    /// 這段散文描述的「能力」類推成又一個工具，直接發出名為 artifact-html 的
    /// tool call，掉進 knowledge_base/tools.rs 的 `Unknown tool` fallback——
    /// 使用者看到「報告已完成」卻什麼都沒出現。這條釘住那句否定，別讓它被
    /// 「精簡」掉。
    /// 終端機 agent 模式實測：模型不知道有 artifact 時，會改用
    /// `cat > x.html <<'EOF'` 把 HTML 寫進檔案，而那個多行 heredoc 會讓終端機
    /// 卡在 heredoc> 永遠等不到結束標記。這條釘住那個禁令。
    #[test]
    fn forbids_writing_documents_through_the_shell() {
        let s = artifact_protocol_section();
        assert!(s.contains("Never build a document by shelling out"));
        assert!(s.contains("cat > file.html"), "name the exact anti-pattern");
    }

    #[test]
    fn states_plainly_that_these_are_not_tools() {
        let s = artifact_protocol_section();
        assert!(s.contains("NOT tools"), "must say they are not tools");
        assert!(s.contains("Never call them as a tool"));
    }
}
