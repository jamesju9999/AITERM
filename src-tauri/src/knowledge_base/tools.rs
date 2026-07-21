use sqlx::SqlitePool;
use crate::db::knowledge_base;
use crate::knowledge_base::embedding::Embedder;
use crate::ai::McpToolDefinition;

const MAX_READ_DOCUMENT_BYTES: usize = 100 * 1024;

pub fn tool_definitions() -> Vec<McpToolDefinition> {
    vec![
        McpToolDefinition {
            name: "search_documents".into(),
            description: "Semantic search over the notebook's indexed documents. Returns the most relevant text chunks, each tagged with its source file path, location hint, and similarity score. This is your primary tool — call it first for any question.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural-language description of what you're looking for — not just keywords." },
                    "top_k": { "type": "integer", "description": "Number of results to return (default 8, max 20)." }
                },
                "required": ["query"]
            }),
        },
        McpToolDefinition {
            name: "read_document".into(),
            description: "Read a document's full converted content by its exact path (as shown in search_documents results). Use when a single chunk doesn't give enough context.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Document rel_path exactly as returned by search_documents." }
                },
                "required": ["path"]
            }),
        },
    ]
}

/// 找不到剛好落在 max_bytes 的 UTF-8 字元邊界時往前找最近的合法邊界，
/// 避免在多位元組字元（中文）中間切斷造成 panic。
fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub async fn dispatch_tool(
    pool: &SqlitePool,
    notebook_id: &str,
    embedder: &dyn Embedder,
    name: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    match name {
        "search_documents" => {
            let query = args["query"].as_str().unwrap_or("").trim().to_owned();
            let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(8).clamp(1, 20) as usize;

            if query.is_empty() {
                return ("Error: query is empty".into(), false);
            }

            let mut vectors = match embedder.embed(&[query]).await {
                Ok(v) => v,
                Err(e) => return (format!("Error: {e}"), false),
            };
            let query_embedding = match vectors.pop() {
                Some(v) => v,
                None => return ("Error: embedding provider returned no vector".into(), false),
            };

            match knowledge_base::search_similar_chunks(pool, notebook_id, &query_embedding, top_k).await {
                Ok(hits) if hits.is_empty() => ("No matching content found.".into(), false),
                Ok(hits) => {
                    let formatted = hits.iter().enumerate().map(|(i, h)| {
                        let loc = h.location_hint.as_deref().unwrap_or("(no section title)");
                        format!(
                            "[{}] {} — {} (score {:.2})\n{}",
                            i + 1, h.rel_path, loc, h.score, h.text
                        )
                    }).collect::<Vec<_>>().join("\n\n---\n\n");
                    (formatted, false)
                }
                Err(e) => (format!("Error: {e}"), false),
            }
        }
        "read_document" => {
            let path = args["path"].as_str().unwrap_or("").to_owned();
            match knowledge_base::get_document_by_path(pool, notebook_id, &path).await {
                Ok(Some(doc)) if doc.status == "ok" => {
                    let content = doc.markdown_cache.unwrap_or_default();
                    let truncated = content.len() > MAX_READ_DOCUMENT_BYTES;
                    let content = if truncated {
                        format!(
                            "{}\n\n[TRUNCATED: document exceeds size limit]",
                            safe_truncate(&content, MAX_READ_DOCUMENT_BYTES)
                        )
                    } else {
                        content
                    };
                    (content, truncated)
                }
                Ok(Some(doc)) => (
                    format!("Error: document has status '{}': {}", doc.status, doc.error_message.unwrap_or_default()),
                    false,
                ),
                Ok(None) => (format!("Error: no document found at path '{path}' in this notebook"), false),
                Err(e) => (format!("Error: {e}"), false),
            }
        }
        _ => (format!("Unknown tool: {name}"), false),
    }
}
