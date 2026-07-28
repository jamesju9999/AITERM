use std::sync::Arc;
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::ai::{
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest,
    GenerateWithToolsResult, Locale, QueryMode,
};
use crate::db::knowledge_base::NotebookRow;
use crate::knowledge_base::embedding::Embedder;
use crate::knowledge_base::tools::{dispatch_tool, tool_definitions};

const MAX_TOOL_ROUNDS: usize = 20;
const TOKEN_ESTIMATE_LIMIT: usize = 50_000;
const CHECKPOINT_THRESHOLD: usize = 30_000;
const MAX_CHECKPOINTS: usize = 2;

const KB_CHAT_EVENT: &str = "kb-chat-event";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KbChatEvent {
    ToolCall {
        session_id: String,
        call_id: String,
        tool: String,
        args: serde_json::Value,
    },
    ToolResult {
        session_id: String,
        call_id: String,
        content: String,
        truncated: bool,
    },
    TextDelta {
        session_id: String,
        delta: String,
    },
    Checkpoint {
        session_id: String,
        number: usize,
    },
    ClearContent {
        session_id: String,
    },
    Done {
        session_id: String,
    },
    Error {
        session_id: String,
        message: String,
    },
    FallbackMode {
        session_id: String,
    },
    TokenCount {
        session_id: String,
        count: usize,
        limit: usize,
    },
}

#[derive(Debug, Clone, Serialize)]
struct PersistedToolCall {
    tool: String,
    args: serde_json::Value,
    result: String,
}

async fn save_chat_turn(
    pool: &SqlitePool,
    chat_session_id: &str,
    user_text: &str,
    assistant_text: &str,
    tool_calls: &[PersistedToolCall],
) {
    let _ = crate::db::kb_chat_sessions::create_chat_message(
        pool, chat_session_id, "user", user_text, None,
    ).await;

    let tool_calls_json = if tool_calls.is_empty() {
        None
    } else {
        serde_json::to_string(tool_calls).ok()
    };

    let _ = crate::db::kb_chat_sessions::create_chat_message(
        pool, chat_session_id, "assistant", assistant_text, tool_calls_json.as_deref(),
    ).await;
}

fn build_system_prompt(notebook_name: &str, locale: Locale) -> String {
    let language = crate::ai::language_name(locale);
    format!(
r#"You are a research assistant answering questions strictly from the documents in the notebook "{notebook_name}".

## Reply Language — MANDATORY
Write your ENTIRE reply in {language}: every sentence, every citation label, and every text label inside Mermaid diagrams. The documents and tool output stay in their original language, but that must NOT change the language you write in. Do not default to English. This rule overrides everything else in this prompt.

## Tools

- search_documents(query, top_k?): semantic search over indexed document chunks. Returns the most relevant chunks with source file path, location hint, and a similarity score. This is your primary tool — call it first for any question.
- read_document(path): read a document's full converted content by its exact path (as shown in search_documents results). Use when a single chunk doesn't give enough context.

## Search Strategy

0. EVERY new user message is a fresh question and requires its own tool call(s) in THIS turn — even if an earlier turn in this conversation already searched or read a document. Never answer a new question using only documents retrieved for a DIFFERENT, earlier question. A new topic almost always needs its own search, even within the same notebook.
1. Call search_documents with a natural-language description of what you need — not just keywords.
2. If the returned chunks don't fully answer the question, call read_document on the most promising source for full context.
3. If the first search doesn't find what you need, try search_documents again with different phrasing before giving up.
4. Answer once you have enough verified content from tool calls made in THIS turn.

## Accuracy — Non-Negotiable

- EVERY factual claim must come from a chunk returned by search_documents or read_document in THIS session — never from general knowledge or inference.
- ALWAYS cite your source after each claim using the exact rel_path and location_hint returned by the tools, e.g. (report.pdf, 第一章).
- NEVER cite a file you have not actually retrieved content from via search_documents or read_document this session.
- If the documents don't contain an answer, say so explicitly — do not guess or fill gaps with outside knowledge.
- Do not fabricate document names, section titles, or quotes.
- Before stating that a document does not exist in this notebook, you MUST call search_documents or read_document for it in THIS turn — never conclude non-existence just because it wasn't mentioned in searches from earlier, different questions in this conversation.
- NEVER describe having searched, read, or found something in a document unless you actually made that exact tool call in THIS turn. Do not narrate a tool action you did not take.
- Do NOT reuse a previous turn's search_documents/read_document results to answer a new question on a different topic. Even if a document is already visible earlier in this conversation, call the tools again for the current question — the right source for this question may be a different document entirely.

- Reminder: the entire reply, including all diagram labels, must be in {language}.
- **Mermaid diagrams**: node IDs must be plain ASCII identifiers. Wrap every node label and edge label in double quotes. NEVER put the characters `|`, `<`, `>`, or `"` inside a label — they collide with Mermaid syntax and break the diagram. Write `Bearer key` not `Bearer <key>`, and use a comma or space instead of `|`. Do not use `<br/>` inside labels — use a space instead. Prefer the theme's default node colors; if you must set a `fill`, keep it dark/muted so light text stays readable."#
    )
}

fn estimate_tokens(s: &str) -> usize {
    s.len() / 4
}

/// Build a deduplication key for a tool call.
fn tool_call_key(tool_name: &str, args: &serde_json::Value) -> String {
    format!("{}:{}", tool_name, args)
}

/// Parse tool calls from XML text that local models emit instead of proper JSON tool-calls.
/// Identical logic to `code_assistant::parse_xml_tool_calls` — duplicated rather than shared
/// to keep the two agent loops independently evolvable (see Plan B Architecture note).
fn parse_xml_tool_calls(text: &str) -> Vec<(String, serde_json::Value)> {
    let mut results = Vec::new();

    let mut search = text;
    while let Some(start) = search.find("<tool_call>") {
        let after = &search[start + "<tool_call>".len()..];
        let inner = if let Some(end) = after.find("</tool_call>") {
            after[..end].trim()
        } else {
            after.trim()
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(inner) {
            if let Some(name) = v["name"].as_str() {
                let args = v.get("arguments").cloned().unwrap_or(serde_json::json!({}));
                results.push((name.to_owned(), args));
            }
        }
        let skip = start + "<tool_call>".len();
        search = &search[skip..];
    }

    if results.is_empty() {
        let mut s = text;
        while let Some(fn_pos) = s.find("<function=") {
            let after_fn = &s[fn_pos + "<function=".len()..];
            let name_end = after_fn.find('>').unwrap_or(after_fn.len());
            let fn_name = after_fn[..name_end].trim().to_owned();
            let rest = &after_fn[name_end..];

            let mut args = serde_json::Map::new();
            let mut param_search = rest;
            while let Some(p) = param_search.find("<parameter=") {
                let after_p = &param_search[p + "<parameter=".len()..];
                let key_end = after_p.find('>').unwrap_or(after_p.len());
                let key = after_p[..key_end].trim().to_owned();
                let after_key = &after_p[key_end + 1..];
                let val = if let Some(c) = after_key.find("</parameter>") {
                    after_key[..c].trim()
                } else {
                    after_key.trim()
                };
                let json_val = if let Ok(n) = val.parse::<i64>() {
                    serde_json::Value::Number(n.into())
                } else {
                    serde_json::Value::String(val.to_owned())
                };
                args.insert(key, json_val);
                param_search = &after_key[after_key.find("</parameter>").map(|x| x + "</parameter>".len()).unwrap_or(after_key.len())..];
            }

            if !fn_name.is_empty() {
                results.push((fn_name, serde_json::Value::Object(args)));
            }
            s = &after_fn[name_end..];
        }
    }

    results
}

pub async fn run_chat(
    pool: SqlitePool,
    notebook: NotebookRow,
    messages: Vec<ChatMessage>,
    chat_provider: Arc<dyn AiProvider>,
    embedder: Arc<dyn Embedder>,
    session_id: String,
    chat_session_id: String,
    locale: Locale,
    app: AppHandle,
) -> Result<(), AiError> {
    let tool_defs = tool_definitions();
    let system_prompt = build_system_prompt(&notebook.name, locale);

    let mut conversation = messages;
    let last_user_text = conversation.iter().rev()
        .find(|m| m.role == "user")
        .and_then(|m| m.content.as_str())
        .unwrap_or("")
        .to_string();
    let mut persisted_tool_calls: Vec<PersistedToolCall> = Vec::new();
    let mut full_answer_text = String::new();
    // Prompt instructions alone ("call a tool every turn") aren't reliably followed by
    // smaller/local models — observed in practice reusing an earlier turn's document
    // content and answering with zero tool calls this turn. This flag lets us force a
    // single corrective round if that happens, without risking an infinite loop.
    let mut zero_tool_reprompted = false;
    let mut token_estimate = estimate_tokens(&system_prompt);
    let mut rounds = 0usize;
    let mut checkpoints = 0usize;
    let mut seen_calls: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::TokenCount {
            session_id: session_id.clone(),
            count: token_estimate,
            limit: TOKEN_ESTIMATE_LIMIT,
        });

        if token_estimate >= CHECKPOINT_THRESHOLD && checkpoints < MAX_CHECKPOINTS {
            checkpoints += 1;
            let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::Checkpoint {
                session_id: session_id.clone(),
                number: checkpoints,
            });
            let summary = generate_checkpoint_summary(&conversation, chat_provider.clone(), locale).await;
            conversation = compress_conversation(conversation, &summary, checkpoints);
            token_estimate = estimate_tokens(&summary);
            continue;
        }

        let force_answer = rounds >= MAX_TOOL_ROUNDS
            || (token_estimate >= TOKEN_ESTIMATE_LIMIT && checkpoints >= MAX_CHECKPOINTS);

        let language = crate::ai::language_name(locale);
        let effective_prompt = if force_answer {
            format!(
                "{system_prompt}\n\n\
                 STOP ALL TOOL CALLS NOW. Research limit reached after {rounds} rounds.\n\
                 Write your FINAL ANSWER in {language}. Rules:\n\
                 1. Natural language ONLY — absolutely NO JSON, NO arrays.\n\
                 2. Only state facts you directly retrieved via search_documents/read_document.\n\
                 3. If you did not find something, explicitly say so.\n\
                 4. Summarise your findings in clear prose, with citations."
            )
        } else {
            system_prompt.clone()
        };

        let req = GenerateRequest {
            system_prompt: effective_prompt,
            messages: conversation.clone(),
            context: Default::default(),
            mode: QueryMode::Chat,
            max_tokens: None,
        };

        let (tx, mut rx) = mpsc::channel::<GenerateChunk>(32);
        let provider_clone = chat_provider.clone();
        let tools_for_call = if force_answer { vec![] } else { tool_defs.clone() };
        let force_answer_clone = force_answer;

        let join = tokio::spawn(async move {
            if force_answer_clone {
                provider_clone.generate(req, tx).await
                    .map(|_| GenerateWithToolsResult::Text(String::new()))
            } else {
                provider_clone.generate_with_tools(req, tools_for_call, tx).await
            }
        });

        while let Some(chunk) = rx.recv().await {
            if !chunk.delta.is_empty() {
                full_answer_text.push_str(&chunk.delta);
                let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::TextDelta {
                    session_id: session_id.clone(),
                    delta: chunk.delta.clone(),
                });
            }
            if chunk.done { break; }
        }

        match join.await {
            Err(e) => {
                let msg = e.to_string();
                let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::Error {
                    session_id: session_id.clone(),
                    message: msg.clone(),
                });
                return Err(AiError::Network { message: msg });
            }
            Ok(Err(AiError::ToolCallingUnsupported)) |
            Ok(Ok(GenerateWithToolsResult::Unsupported)) => {
                let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::FallbackMode {
                    session_id: session_id.clone(),
                });
                return run_fallback(pool, notebook, conversation, chat_provider, embedder, session_id, chat_session_id.clone(), locale, app).await;
            }
            Ok(Err(e)) => {
                let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::Error {
                    session_id: session_id.clone(),
                    message: e.to_string(),
                });
                return Err(e);
            }
            Ok(Ok(GenerateWithToolsResult::Text(text))) => {
                let xml_calls = parse_xml_tool_calls(&text);
                if !xml_calls.is_empty() {
                    // Some models emit a malformed/unclosed <tool_call> block and then,
                    // instead of stopping to wait for the real result, keep generating
                    // straight into their own draft answer in the same completion (seen
                    // in practice: "<tool_call>...<parameter=top_k>10</parameter>什麼時候
                    // 要用...根據 Swift 的文件規範..."). That whole blob — junk syntax
                    // plus a hallucinated draft answer — already streamed live to the
                    // frontend before we got here to classify it as a tool-call round.
                    // Since this round produced a tool call, not a final answer, wipe
                    // that leaked text both server-side and in the UI so only a
                    // genuine no-more-tool-calls round's text ends up visible/persisted.
                    full_answer_text.clear();
                    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ClearContent {
                        session_id: session_id.clone(),
                    });
                    for (tool_name, args) in xml_calls {
                        let key = tool_call_key(&tool_name, &args);
                        let call_id = format!("xml_{}", uuid::Uuid::new_v4());
                        if seen_calls.contains(&key) {
                            let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolCall {
                                session_id: session_id.clone(),
                                call_id: call_id.clone(),
                                tool: tool_name.clone(),
                                args: args.clone(),
                            });
                            let skip_msg = "(skipped: you already made this exact call this turn — repeating it will not return new information. \
                                Do NOT call it again. Instead: try a different search phrasing, call read_document on a specific promising file, \
                                or answer now using what you've already gathered.)".to_string();
                            let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolResult {
                                session_id: session_id.clone(),
                                call_id: call_id.clone(),
                                content: skip_msg.clone(),
                                truncated: false,
                            });
                            conversation.push(ChatMessage {
                                role: "tool".into(),
                                content: serde_json::Value::String(skip_msg),
                                tool_call_id: Some(call_id),
                                tool_calls: None,
                            });
                            continue;
                        }
                        seen_calls.insert(key);

                        let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolCall {
                            session_id: session_id.clone(),
                            call_id: call_id.clone(),
                            tool: tool_name.clone(),
                            args: args.clone(),
                        });
                        let (result_content, truncated) =
                            dispatch_tool(&pool, &notebook.id, embedder.as_ref(), &tool_name, &args).await;
                        token_estimate += estimate_tokens(&result_content);
                        let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolResult {
                            session_id: session_id.clone(),
                            call_id: call_id.clone(),
                            content: result_content.clone(),
                            truncated,
                        });
                        persisted_tool_calls.push(PersistedToolCall {
                            tool: tool_name.clone(),
                            args: args.clone(),
                            result: result_content.clone(),
                        });
                        conversation.push(ChatMessage {
                            role: "tool".into(),
                            content: serde_json::Value::String(result_content),
                            tool_call_id: Some(call_id),
                            tool_calls: None,
                        });
                    }
                    rounds += 1;
                } else if persisted_tool_calls.is_empty() && !zero_tool_reprompted && !force_answer {
                    // Model tried to answer this turn without ever calling a tool. The
                    // system prompt already says every turn needs its own tool call, but
                    // that instruction alone isn't reliably followed by smaller/local
                    // models — observed reusing an earlier turn's document content
                    // instead of searching fresh. Force one corrective round.
                    zero_tool_reprompted = true;
                    full_answer_text.clear();
                    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ClearContent {
                        session_id: session_id.clone(),
                    });
                    conversation.push(ChatMessage {
                        role: "user".into(),
                        content: serde_json::Value::String(
                            "You answered without calling search_documents or read_document this turn. \
                             Call search_documents now for this specific question before answering — \
                             do not reuse content from earlier turns.".into()
                        ),
                        tool_call_id: None,
                        tool_calls: None,
                    });
                    continue;
                } else {
                    save_chat_turn(&pool, &chat_session_id, &last_user_text, &full_answer_text, &persisted_tool_calls).await;
                    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::Done {
                        session_id: session_id.clone(),
                    });
                    return Ok(());
                }
            }
            Ok(Ok(GenerateWithToolsResult::ToolCalls { calls, raw })) => {
                // Defensive mirror of the XML-branch fix above: native tool-calling
                // normally has no accompanying free text, but if a provider quirk ever
                // streams stray text alongside real tool_calls, don't let it linger as
                // if it were a final answer.
                full_answer_text.clear();
                let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ClearContent {
                    session_id: session_id.clone(),
                });
                conversation.push(ChatMessage {
                    role: "assistant".into(),
                    content: serde_json::Value::Null,
                    tool_call_id: None,
                    tool_calls: raw.or_else(|| serde_json::to_value(&calls).ok()),
                });

                for call in &calls {
                    let args: serde_json::Value =
                        serde_json::from_str(&call.args.to_string()).unwrap_or_default();

                    let key = tool_call_key(&call.tool_name, &args);
                    if seen_calls.contains(&key) {
                        let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolCall {
                            session_id: session_id.clone(),
                            call_id: call.id.clone(),
                            tool: call.tool_name.clone(),
                            args: args.clone(),
                        });
                        let skip_msg = "(skipped: you already made this exact call this turn — repeating it will not return new information. \
                            Do NOT call it again. Instead: try a different search phrasing, call read_document on a specific promising file, \
                            or answer now using what you've already gathered.)".to_string();
                        let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolResult {
                            session_id: session_id.clone(),
                            call_id: call.id.clone(),
                            content: skip_msg.clone(),
                            truncated: false,
                        });
                        conversation.push(ChatMessage {
                            role: "tool".into(),
                            content: serde_json::Value::String(skip_msg),
                            tool_call_id: Some(call.id.clone()),
                            tool_calls: None,
                        });
                        continue;
                    }
                    seen_calls.insert(key);

                    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolCall {
                        session_id: session_id.clone(),
                        call_id: call.id.clone(),
                        tool: call.tool_name.clone(),
                        args: args.clone(),
                    });

                    let (result_content, truncated) =
                        dispatch_tool(&pool, &notebook.id, embedder.as_ref(), &call.tool_name, &args).await;

                    token_estimate += estimate_tokens(&result_content);

                    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolResult {
                        session_id: session_id.clone(),
                        call_id: call.id.clone(),
                        content: result_content.clone(),
                        truncated,
                    });
                    persisted_tool_calls.push(PersistedToolCall {
                        tool: call.tool_name.clone(),
                        args: args.clone(),
                        result: result_content.clone(),
                    });

                    conversation.push(ChatMessage {
                        role: "tool".into(),
                        content: serde_json::Value::String(result_content),
                        tool_call_id: Some(call.id.clone()),
                        tool_calls: None,
                    });
                }

                rounds += 1;
            }
        }
    }
}

/// 不支援 tool-use 的 provider 走簡化版：直接用使用者原始問題做一次 search_documents，
/// 把結果組進 context 後一次性回答（無多跳能力）。比 code_assistant 的兩階段選檔更簡單，
/// 因為語意搜尋本身就不需要先列目錄。
async fn run_fallback(
    pool: SqlitePool,
    notebook: NotebookRow,
    messages: Vec<ChatMessage>,
    chat_provider: Arc<dyn AiProvider>,
    embedder: Arc<dyn Embedder>,
    session_id: String,
    chat_session_id: String,
    locale: Locale,
    app: AppHandle,
) -> Result<(), AiError> {
    let last_user_text = messages.iter().rev()
        .find(|m| m.role == "user")
        .and_then(|m| m.content.as_str())
        .unwrap_or("")
        .to_string();

    let (search_result, _truncated) = dispatch_tool(
        &pool, &notebook.id, embedder.as_ref(),
        "search_documents", &serde_json::json!({ "query": last_user_text, "top_k": 8 }),
    ).await;

    let language = crate::ai::language_name(locale);
    let phase_prompt = format!(
        "You are a research assistant. Answer the user's question using ONLY the document \
         excerpts below. Always cite the source file and location for each claim. If the \
         excerpts don't answer the question, say so explicitly. Respond in {language}.\n\n\
         ## Document excerpts\n{search_result}"
    );

    let req = GenerateRequest {
        system_prompt: phase_prompt,
        messages,
        context: Default::default(),
        mode: QueryMode::Chat,
        max_tokens: None,
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(32);
    let p = chat_provider.clone();
    let join = tokio::spawn(async move { p.generate(req, tx).await });
    let mut answer_buf = String::new();
    while let Some(chunk) = rx.recv().await {
        if !chunk.delta.is_empty() {
            answer_buf.push_str(&chunk.delta);
            let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::TextDelta {
                session_id: session_id.clone(),
                delta: chunk.delta.clone(),
            });
        }
        if chunk.done { break; }
    }
    let _ = join.await;

    save_chat_turn(&pool, &chat_session_id, &last_user_text, &answer_buf, &[]).await;

    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::Done {
        session_id: session_id.clone(),
    });
    Ok(())
}

/// 與 code_assistant 對應函式邏輯相同，措辭改為「文件研究」而非「程式碼調查」。
async fn generate_checkpoint_summary(
    conversation: &[ChatMessage],
    provider: Arc<dyn AiProvider>,
    locale: Locale,
) -> String {
    let language = crate::ai::language_name(locale);
    let system = format!(
        "You are creating a research checkpoint. The conversation below shows a document \
         research session with tool calls and results. Write a concise structured summary \
         IN {language} of ONLY what has been CONFIRMED — facts directly retrieved via tools:\n\
         - Documents consulted (exact file paths and sections)\n\
         - Specific facts, quotes, or values found (with their source citation)\n\
         - What was searched but NOT found\n\
         - What still needs to be investigated\n\n\
         Max 500 words. Facts only. Do NOT speculate. Do NOT make tool calls."
    );

    let req = GenerateRequest {
        system_prompt: system,
        messages: conversation.to_vec(),
        context: Default::default(),
        mode: QueryMode::Chat,
        max_tokens: Some(700),
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(32);
    let p = provider.clone();
    let join = tokio::spawn(async move { p.generate(req, tx).await });
    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }
    let _ = join.await;
    buf
}

fn compress_conversation(
    conversation: Vec<ChatMessage>,
    summary: &str,
    checkpoint_n: usize,
) -> Vec<ChatMessage> {
    let mut result: Vec<ChatMessage> = conversation
        .into_iter()
        .filter(|m| m.role == "user")
        .collect();

    // `summary` is raw streamed model output and normally ends with a newline
    // (and is empty outright when the summarising call failed). Anthropic
    // validates the last assistant turn as a prefill and rejects it with
    // "final assistant content cannot end with trailing whitespace", so trim.
    let summary = summary.trim();
    let checkpoint = if summary.is_empty() {
        format!("[Checkpoint #{checkpoint_n} — no findings could be summarised]")
    } else {
        format!("[Checkpoint #{checkpoint_n} — confirmed findings so far]\n{summary}")
    };

    result.push(ChatMessage {
        role: "assistant".into(),
        content: serde_json::Value::String(checkpoint),
        tool_call_id: None,
        tool_calls: None,
    });
    // Newer Claude models reject a trailing assistant turn outright (prefill is
    // unsupported), so close the compressed history with a user turn.
    result.push(ChatMessage {
        role: "user".into(),
        content: serde_json::Value::String(
            "Continue from the checkpoint above: keep searching the documents with the tools, \
             or write the final answer if you already have enough."
                .into(),
        ),
        tool_call_id: None,
        tool_calls: None,
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_xml_json_format() {
        let text = r#"<tool_call>{"name":"search_documents","arguments":{"query":"pricing","top_k":5}}</tool_call>"#;
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "search_documents");
        assert_eq!(calls[0].1["query"], "pricing");
        assert_eq!(calls[0].1["top_k"], 5);
    }

    #[test]
    fn parse_xml_attribute_format() {
        let text = "<function=read_document> <parameter=path> notes.md </parameter> </function>";
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "read_document");
        assert_eq!(calls[0].1["path"], "notes.md");
    }

    #[test]
    fn parse_xml_no_match_returns_empty() {
        let calls = parse_xml_tool_calls("Here is my answer: the answer is 42.");
        assert!(calls.is_empty());
    }

    #[test]
    fn dedup_key_is_stable() {
        let args = serde_json::json!({"query": "pricing", "top_k": 5});
        let k1 = tool_call_key("search_documents", &args);
        let k2 = tool_call_key("search_documents", &args);
        assert_eq!(k1, k2);
    }

    fn msg(role: &str, content: serde_json::Value) -> ChatMessage {
        ChatMessage { role: role.into(), content, tool_call_id: None, tool_calls: None }
    }

    #[test]
    fn compressed_conversation_is_anthropic_safe() {
        let conversation = vec![
            msg("user", serde_json::json!("這些文件的主要內容是什麼？")),
            msg("assistant", serde_json::Value::Null),
            msg("tool", serde_json::json!("search hit ...")),
        ];

        // Streamed model output normally ends with a newline.
        let out = compress_conversation(conversation, "已確認：TableSchema 共 12 張表。\n\n", 1);

        assert!(out.iter().all(|m| m.role != "tool"));
        assert_eq!(out.last().unwrap().role, "user", "must not end on an assistant turn");

        let checkpoint = out.iter().find(|m| m.role == "assistant").expect("checkpoint turn");
        let text = checkpoint.content.as_str().expect("checkpoint is plain text");
        assert!(text.contains("Checkpoint #1"));
        assert_eq!(text, text.trim_end(), "assistant content must not end with whitespace");
    }

    #[test]
    fn empty_checkpoint_summary_does_not_produce_blank_assistant_turn() {
        let out = compress_conversation(vec![msg("user", serde_json::json!("問題"))], "  \n ", 2);

        let checkpoint = out.iter().find(|m| m.role == "assistant").expect("checkpoint turn");
        let text = checkpoint.content.as_str().expect("checkpoint is plain text");
        assert!(!text.is_empty());
        assert_eq!(text, text.trim_end());
    }
}
