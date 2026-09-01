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

Two fenced code blocks get special rendering in a panel beside this
conversation. Reach for them when the answer is something to look at rather
than a sentence to read.

- ```artifact-html — the body is a complete HTML document, rendered in an
  isolated sandbox. Use it for reports, formatted summaries, comparison
  tables, or anything worth reading as its own document. Include a <title>;
  it becomes the panel's title. <style> and <script> both work, but the page
  cannot reach anything outside its own frame.
- ```artifact-chart — the body is JSON describing a chart:
  {"type":"bar"|"line"|"pie","title":"...","data":[{...}],"xKey":"...",
   "series":[{"key":"...","label":"..."}]}
  `data` is an array of row objects, `xKey` names the field used for the
  category axis, and each series `key` names a numeric field on those rows.

Rules:
- At most one artifact per reply; a later one replaces the earlier one.
- Do NOT use an artifact for a short answer, a single command, or a couple of
  sentences — those belong in the reply itself.
- Always also write a line or two in the reply saying what you produced. The
  artifact supplements the answer, it is not a substitute for one."#
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
}
