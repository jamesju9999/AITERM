# File & Image Paste Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow users to paste or drag-and-drop files into all three entry points: AI Panel (real multimodal attachments), WarpInput and PTY terminal (absolute path insertion).

**Architecture:** Widen `ChatMessage.content` from `String`/`string` to `serde_json::Value`/`string | ContentPart[]` through the full stack. AI Panel accumulates `Attachment[]` state and assembles OpenAI-format `ContentPart[]` on send. Each Rust provider transforms the content array into its native format. WarpInput and PTY only insert file paths as text—no IPC changes needed.

**Tech Stack:** React 19, TypeScript, Tauri 2, Rust (serde_json), xterm.js, Vitest, cargo test / wiremock

---

## File Map

| File | Action | Purpose |
|------|--------|---------|
| `src/types/attachment.ts` | Create | `Attachment` type + `buildContentParts` + `contentToDisplayString` helpers |
| `src/ipc/ai.ts` | Modify | Add `ContentPart`, widen `ChatMessage.content` to `string \| ContentPart[]` |
| `src/hooks/useAiChat.ts` | Modify | Extend `send()` to accept `Attachment[]`; fix `formatSessionTitle` for array content |
| `src/components/AiPanel/MessageBubble.tsx` | Modify | Accept `string \| ContentPart[]` content; render image thumbs in user bubbles |
| `src/components/AiPanel/MessageList.tsx` | Modify | Pass typed `content` to MessageBubble |
| `src/components/AiPanel/index.tsx` | Modify | Attachment state, paste/drop handlers, preview pills UI, pass attachments on send |
| `src/components/WarpInput.tsx` | Modify | paste/drop handlers → insert file path at cursor |
| `src/components/TerminalView.tsx` | Modify | paste/drop on `hostRef` → `writePty` file path |
| `src-tauri/src/ai/mod.rs` | Modify | `ChatMessage.content: String` → `serde_json::Value` |
| `src-tauri/src/ai/openai.rs` | Modify | `OpenAiMessage` owned fields; `build_request_body` clones `Value` content |
| `src-tauri/src/ai/compatible.rs` | Modify | Same pattern as openai.rs |
| `src-tauri/src/ai/anthropic.rs` | Modify | Add `to_anthropic_content()` conversion; owned `AnthropicMessage` |
| `src-tauri/src/ai/ollama.rs` | Modify | Extract base64 images from content array into `images` field |
| `src-tauri/tests/openai_client.rs` | Modify | Add test: multipart content reaches endpoint as array |
| `src-tauri/tests/anthropic_client.rs` | Modify | Add test: image_url converted to Anthropic format |

---

## Task 1: Rust — Widen `ChatMessage.content`

**Files:**
- Modify: `src-tauri/src/ai/mod.rs:69-73`
- Modify: `src-tauri/src/ai/openai.rs:74` (health_check ChatMessage constructor)
- Modify: `src-tauri/src/ai/anthropic.rs:142-143` (`health_check_request` function)
- Modify: `src-tauri/tests/openai_client.rs:16-26` (req helper)
- Modify: `src-tauri/tests/anthropic_client.rs` (req helper)
- Modify: `src-tauri/tests/compatible_client.rs` (req helper)
- Modify: `src-tauri/tests/ollama_client.rs` (req helper)

- [ ] **Step 1: Write a failing test in `src-tauri/src/ai/mod.rs`**

Add to the existing `#[cfg(test)] mod tests` block at the bottom of `mod.rs`:

```rust
#[test]
fn chat_message_content_accepts_array() {
    let msg = ChatMessage {
        role: "user".into(),
        content: serde_json::json!([
            {"type": "text", "text": "hello"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
        ]),
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert!(json["content"].is_array());
    assert_eq!(json["content"][0]["type"], "text");
}
```

- [ ] **Step 2: Run the test — expect compile error**

```bash
cd src-tauri && cargo test chat_message_content_accepts_array 2>&1 | head -30
```

Expected: compile error — `serde_json::json!(...)` does not match `String`.

- [ ] **Step 3: Change `ChatMessage.content` in `src-tauri/src/ai/mod.rs`**

Replace lines 69–73:

```rust
// Before:
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}
```

```rust
// After:
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    /// Either a plain string (legacy) or an OpenAI-format content array.
    pub content: serde_json::Value,
}
```

- [ ] **Step 4: Fix all `ChatMessage` constructors that use `.into()` on a string**

In `src-tauri/src/ai/openai.rs` health_check (around line 74):
```rust
// Before:
messages: vec![ChatMessage { role: "user".into(), content: "hi".into() }],
// After:
messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("hi") }],
```

In `src-tauri/src/ai/anthropic.rs` `health_check_request()` (around line 142):
```rust
// Before:
messages: vec![ChatMessage { role: "user".into(), content: "hi".into() }],
// After:
messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("hi") }],
```

- [ ] **Step 5: Fix test helpers in all 4 test files**

In `src-tauri/tests/openai_client.rs`, `src-tauri/tests/anthropic_client.rs`, `src-tauri/tests/compatible_client.rs`, `src-tauri/tests/ollama_client.rs` — each has a `req(text: &str)` helper. Change:
```rust
// Before:
content: text.into()
// After:
content: serde_json::json!(text)
```

- [ ] **Step 6: Run all Rust tests — expect pass**

```bash
cd src-tauri && cargo test 2>&1 | tail -20
```

Expected: all tests pass (the existing content-as-string tests still work because `serde_json::json!("hi")` serializes as `"hi"` — a JSON string).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/ai/mod.rs src-tauri/src/ai/openai.rs src-tauri/src/ai/anthropic.rs src-tauri/tests/openai_client.rs src-tauri/tests/anthropic_client.rs src-tauri/tests/compatible_client.rs src-tauri/tests/ollama_client.rs
git commit -m "feat(ai): widen ChatMessage.content to serde_json::Value for multimodal support"
```

---

## Task 2: Rust — OpenAI + Compatible providers: pass through content array

**Files:**
- Modify: `src-tauri/src/ai/openai.rs`
- Modify: `src-tauri/src/ai/compatible.rs`
- Modify: `src-tauri/tests/openai_client.rs`

- [ ] **Step 1: Write a failing test in `src-tauri/tests/openai_client.rs`**

Add after the existing tests:

```rust
#[tokio::test]
async fn multipart_content_reaches_endpoint_as_array() {
    let server = MockServer::start().await;

    // The mock captures the raw request body so we can inspect it.
    let mock = Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(bearer_token("test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_response_happy_path()),
        )
        .mount_as_scoped(&server)
        .await;

    let client = OpenAiClient::with_base_url(
        "test-key".into(),
        "gpt-4o".into(),
        server.uri(),
    );
    let (tx, _rx) = mpsc::channel(32);
    let multipart_req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!([
                {"type": "text", "text": "describe this"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
            ]),
        }],
        context: EnvSnapshot {
            os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default()
        },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    };
    client.generate(multipart_req, tx).await.unwrap();

    // Verify the request body sent to the mock contained an array content
    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    // Last message is the user message (after system)
    let user_content = &body["messages"][1]["content"];
    assert!(user_content.is_array(), "content should be an array, got: {user_content}");
    assert_eq!(user_content[0]["type"], "text");
    assert_eq!(user_content[1]["type"], "image_url");
}
```

- [ ] **Step 2: Run — expect fail**

```bash
cd src-tauri && cargo test multipart_content_reaches_endpoint_as_array 2>&1 | tail -20
```

Expected: test panics — `content` is serialized as a plain string, not array.

- [ ] **Step 3: Update `OpenAiMessage` and `build_request_body` in `src-tauri/src/ai/openai.rs`**

Replace the `OpenAiMessage` struct and `build_request_body` function (currently around lines 115–145):

```rust
#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: serde_json::Value,
}

fn build_request_body(
    model: &str,
    req: &GenerateRequest,
    json_mode: bool,
) -> OpenAiChatRequest {
    let mut messages: Vec<OpenAiMessage> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(OpenAiMessage {
        role: "system".to_owned(),
        content: serde_json::Value::String(req.system_prompt.clone()),
    });
    for m in &req.messages {
        messages.push(OpenAiMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        });
    }
    OpenAiChatRequest {
        model: model.to_owned(),
        messages,
        stream: true,
        response_format: if json_mode { Some(ResponseFormat { ty: "json_object" }) } else { None },
        max_tokens: req.max_tokens,
    }
}
```

Also update `OpenAiChatRequest` to use owned `String` instead of `&'a str` (remove the lifetime parameter):

```rust
#[derive(Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}
```

- [ ] **Step 4: Apply the same pattern to `src-tauri/src/ai/compatible.rs`**

Replace `CompatibleMessage`, `CompatibleChatRequest`, and `build_request_body` with the identical owned-String pattern:

```rust
#[derive(Serialize)]
struct CompatibleChatRequest {
    model: String,
    messages: Vec<CompatibleMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct CompatibleMessage {
    role: String,
    content: serde_json::Value,
}

fn build_request_body(
    model: &str,
    req: &GenerateRequest,
    json_mode: bool,
) -> CompatibleChatRequest {
    let mut messages: Vec<CompatibleMessage> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(CompatibleMessage {
        role: "system".to_owned(),
        content: serde_json::Value::String(req.system_prompt.clone()),
    });
    for m in &req.messages {
        messages.push(CompatibleMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        });
    }
    CompatibleChatRequest {
        model: model.to_owned(),
        messages,
        stream: true,
        response_format: if json_mode { Some(ResponseFormat { ty: "json_object" }) } else { None },
        max_tokens: req.max_tokens,
    }
}
```

- [ ] **Step 5: Run tests — expect pass**

```bash
cd src-tauri && cargo test 2>&1 | tail -20
```

Expected: all tests pass including the new multipart test.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ai/openai.rs src-tauri/src/ai/compatible.rs src-tauri/tests/openai_client.rs
git commit -m "feat(ai): openai+compatible providers pass through content array for vision"
```

---

## Task 3: Rust — Anthropic provider: convert OpenAI content array to Anthropic format

**Files:**
- Modify: `src-tauri/src/ai/anthropic.rs`
- Modify: `src-tauri/tests/anthropic_client.rs`

- [ ] **Step 1: Write a failing test in `src-tauri/tests/anthropic_client.rs`**

Add a helper and test (add near the end of the file):

```rust
fn multipart_req() -> GenerateRequest {
    use aiterm_lib::ai::ChatMessage;
    GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!([
                {"type": "text", "text": "what is this?"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw"}}
            ]),
        }],
        context: EnvSnapshot {
            os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default()
        },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    }
}

#[tokio::test]
async fn image_url_converted_to_anthropic_format() {
    let server = MockServer::start().await;
    // Use a minimal non-streaming response for simplicity
    let body_json = serde_json::json!({
        "content": [{"type": "text", "text": "a photo"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 10, "output_tokens": 5}
    });
    let mock = Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(&body_json),
        )
        .mount_as_scoped(&server)
        .await;

    let client = AnthropicClient::with_base_url(
        "test-key".into(),
        "claude-3-5-sonnet-20241022".into(),
        server.uri(),
    );
    let (tx, _rx) = mpsc::channel(32);
    // Note: non-streaming path; use stream=false via health_check path.
    // We can't easily test non-streaming here via generate(), so we check
    // via the mock request body.
    client.generate(multipart_req(), tx).await.ok();

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    let user_content = &body["messages"][0]["content"];
    assert!(user_content.is_array(), "content should be array");
    // First part: text stays as-is
    assert_eq!(user_content[0]["type"], "text");
    // Second part: image_url → Anthropic image format
    assert_eq!(user_content[1]["type"], "image");
    assert_eq!(user_content[1]["source"]["type"], "base64");
    assert_eq!(user_content[1]["source"]["media_type"], "image/png");
    assert_eq!(user_content[1]["source"]["data"], "iVBORw");
}
```

- [ ] **Step 2: Run — expect fail**

```bash
cd src-tauri && cargo test image_url_converted_to_anthropic_format 2>&1 | tail -20
```

Expected: compile error or assertion fail — content is currently serialized as string.

- [ ] **Step 3: Add `to_anthropic_content` helper and update `AnthropicMessage` in `src-tauri/src/ai/anthropic.rs`**

Add after the imports, before `AnthropicClient`:

```rust
/// Parse a data URI "data:<media_type>;base64,<data>" into (media_type, data).
fn parse_data_uri(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let comma = rest.find(',')?;
    let meta = &rest[..comma];
    let data = &rest[comma + 1..];
    let media_type = meta.split(';').next()?;
    Some((media_type.to_owned(), data.to_owned()))
}

/// Convert an OpenAI-format content value to Anthropic format.
/// - String → [{"type": "text", "text": "..."}]
/// - Array → map image_url parts to Anthropic image blocks; text parts pass through
fn to_anthropic_content(content: &serde_json::Value) -> serde_json::Value {
    if let Some(s) = content.as_str() {
        return serde_json::json!([{"type": "text", "text": s}]);
    }
    if let Some(parts) = content.as_array() {
        let converted: Vec<serde_json::Value> = parts.iter().map(|part| {
            if part.get("type").and_then(|t| t.as_str()) == Some("image_url") {
                if let Some(url) = part.get("image_url")
                    .and_then(|i| i.get("url"))
                    .and_then(|u| u.as_str())
                {
                    if let Some((media_type, data)) = parse_data_uri(url) {
                        return serde_json::json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": media_type,
                                "data": data
                            }
                        });
                    }
                }
            }
            part.clone()
        }).collect();
        return serde_json::Value::Array(converted);
    }
    content.clone()
}
```

- [ ] **Step 4: Update `AnthropicMessage` and `build_request_body`**

Replace the `AnthropicMessage` struct and `build_request_body` (currently around lines 112–135):

```rust
#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

fn build_request_body(model: &str, req: &GenerateRequest, stream: bool) -> AnthropicRequest {
    let messages = req
        .messages
        .iter()
        .map(|m| AnthropicMessage {
            role: m.role.clone(),
            content: to_anthropic_content(&m.content),
        })
        .collect();
    AnthropicRequest {
        model: model.to_owned(),
        system: req.system_prompt.clone(),
        messages,
        max_tokens: req.max_tokens.unwrap_or(1024),
        stream,
    }
}
```

Also update `AnthropicRequest` to use owned `String`:

```rust
#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    system: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    stream: bool,
}
```

- [ ] **Step 5: Run tests — expect pass**

```bash
cd src-tauri && cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ai/anthropic.rs src-tauri/tests/anthropic_client.rs
git commit -m "feat(ai): anthropic provider converts OpenAI image_url to Anthropic base64 format"
```

---

## Task 4: Rust — Ollama provider: extract images from content array

**Files:**
- Modify: `src-tauri/src/ai/ollama.rs`
- Modify: `src-tauri/tests/ollama_client.rs`

- [ ] **Step 1: Write a failing test in `src-tauri/tests/ollama_client.rs`**

Add a helper and test (check file for existing import pattern first, add after existing tests):

```rust
#[tokio::test]
async fn multipart_content_puts_images_in_images_field() {
    let server = MockServer::start().await;
    let mock = Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_string("{\"message\":{\"content\":\"ok\"},\"done\":true}\n"),
        )
        .mount_as_scoped(&server)
        .await;

    let client = OllamaClient::with_base_url("llava".into(), server.uri());
    let (tx, _rx) = mpsc::channel(32);
    use aiterm_lib::ai::{ChatMessage, EnvSnapshot, GenerateRequest, QueryMode};
    let req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!([
                {"type": "text", "text": "describe"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc123"}}
            ]),
        }],
        context: EnvSnapshot {
            os: "linux".into(), shell: "bash".into(),
            cwd: std::path::PathBuf::from("/"), ..Default::default()
        },
        mode: QueryMode::Chat,
        max_tokens: None,
    };
    client.generate(req, tx).await.unwrap();

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    // User message (index 1, after system)
    let user_msg = &body["messages"][1];
    assert_eq!(user_msg["content"], "describe");
    assert_eq!(user_msg["images"][0], "abc123");
}
```

- [ ] **Step 2: Run — expect fail**

```bash
cd src-tauri && cargo test multipart_content_puts_images_in_images_field 2>&1 | tail -20
```

Expected: assertion fails or compile error.

- [ ] **Step 3: Add helper and update `OllamaMessage` + `build_request_body` in `src-tauri/src/ai/ollama.rs`**

Add after the imports, before `OllamaClient`:

```rust
/// Flatten a content Value to (text_string, base64_images).
/// For plain strings: text = string, images = empty.
/// For arrays: concatenate text parts; collect base64 data from image_url parts.
fn flatten_ollama_content(content: &serde_json::Value) -> (String, Vec<String>) {
    if let Some(s) = content.as_str() {
        return (s.to_owned(), vec![]);
    }
    if let Some(parts) = content.as_array() {
        let mut texts = Vec::new();
        let mut images = Vec::new();
        for part in parts {
            match part.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                        texts.push(t.to_owned());
                    }
                }
                Some("image_url") => {
                    if let Some(url) = part.get("image_url")
                        .and_then(|i| i.get("url"))
                        .and_then(|u| u.as_str())
                    {
                        // Extract base64 data from data URI
                        if let Some(comma) = url.find(',') {
                            images.push(url[comma + 1..].to_owned());
                        }
                    }
                }
                _ => {}
            }
        }
        return (texts.join(" "), images);
    }
    (String::new(), vec![])
}
```

Replace `OllamaMessage` and `OllamaChatRequest`:

```rust
#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<String>,
}

fn build_request_body(model: &str, req: &GenerateRequest, stream: bool) -> OllamaChatRequest {
    let mut messages: Vec<OllamaMessage> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(OllamaMessage {
        role: "system".to_owned(),
        content: req.system_prompt.clone(),
        images: vec![],
    });
    for m in &req.messages {
        let (content, images) = flatten_ollama_content(&m.content);
        messages.push(OllamaMessage { role: m.role.clone(), content, images });
    }
    OllamaChatRequest { model: model.to_owned(), messages, stream }
}
```

- [ ] **Step 4: Run all tests — expect pass**

```bash
cd src-tauri && cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ai/ollama.rs src-tauri/tests/ollama_client.rs
git commit -m "feat(ai): ollama provider extracts base64 images into images[] field for vision models"
```

---

## Task 5: Frontend — Attachment types and content helpers

**Files:**
- Create: `src/types/attachment.ts`
- Modify: `src/ipc/ai.ts`

- [ ] **Step 1: Create `src/types/attachment.ts`**

```typescript
import { nanoid } from "nanoid";
import type { ContentPart } from "../ipc/ai";

export type AttachmentKind = "image" | "text" | "binary";

export interface Attachment {
  id: string;
  kind: AttachmentKind;
  name: string;
  mimeType: string;
  /** image → base64 data URI; text → raw content; binary → absolute path */
  data: string;
  /** Same as data for images — used as <img src>. */
  previewUrl?: string;
}

/** Classify a File by MIME type and extension. */
export function classifyFile(file: File): AttachmentKind {
  if (file.type.startsWith("image/")) return "image";
  const textExtensions = /\.(txt|md|json|csv|ts|tsx|js|jsx|py|rs|go|java|c|cpp|h|css|html|xml|yaml|yml|toml|sh|bash|zsh)$/i;
  if (file.type.startsWith("text/") || textExtensions.test(file.name)) return "text";
  return "binary";
}

/** Read a file into an Attachment. Returns a Promise. */
export async function readFileAsAttachment(file: File): Promise<Attachment> {
  const kind = classifyFile(file);
  const id = nanoid();
  // Tauri 2 injects .path on File objects in the webview
  const filePath = (file as File & { path?: string }).path ?? file.name;

  if (kind === "image") {
    const data = await readAsDataUrl(file);
    return { id, kind, name: file.name, mimeType: file.type, data, previewUrl: data };
  }
  if (kind === "text") {
    const data = await readAsText(file);
    return { id, kind, name: file.name, mimeType: file.type || "text/plain", data };
  }
  // binary: just store the path
  return { id, kind, name: file.name, mimeType: file.type || "application/octet-stream", data: filePath };
}

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

function readAsText(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsText(file);
  });
}

/** Build a ContentPart[] from user text + attachments. */
export function buildContentParts(text: string, attachments: Attachment[]): ContentPart[] {
  const parts: ContentPart[] = [];
  if (text.trim()) {
    parts.push({ type: "text", text });
  }
  for (const att of attachments) {
    if (att.kind === "image") {
      parts.push({ type: "image_url", image_url: { url: att.data } });
    } else if (att.kind === "text") {
      parts.push({ type: "text", text: `[${att.name}]\n${att.data}` });
    } else {
      parts.push({ type: "text", text: att.data });
    }
  }
  return parts;
}

/** Extract displayable text from a content value (for session title, etc.). */
export function contentToDisplayString(content: string | ContentPart[]): string {
  if (typeof content === "string") return content;
  return content
    .filter((p): p is Extract<ContentPart, { type: "text" }> => p.type === "text")
    .map((p) => p.text)
    .join(" ");
}
```

> Note: `nanoid` is already a dependency (check `package.json`). If missing, run `npm install nanoid`.

- [ ] **Step 2: Verify `nanoid` is available**

```bash
grep nanoid /Users/jamesju/Documents/GitHub/AITERM/package.json
```

Expected: nanoid appears in dependencies. If not: `npm install nanoid`.

- [ ] **Step 3: Add `ContentPart` and widen `ChatMessage.content` in `src/ipc/ai.ts`**

After line 1 (the `invoke` import), add:

```typescript
export type ContentPart =
  | { type: "text"; text: string }
  | { type: "image_url"; image_url: { url: string } };
```

Change `ChatMessage.content` type:

```typescript
// Before:
export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
}

// After:
export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string | ContentPart[];
}
```

- [ ] **Step 4: Write a Vitest test for `buildContentParts` and `contentToDisplayString`**

Create `src/types/attachment.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { buildContentParts, contentToDisplayString } from "./attachment";
import type { Attachment } from "./attachment";

const imageAtt: Attachment = {
  id: "1", kind: "image", name: "photo.png",
  mimeType: "image/png", data: "data:image/png;base64,abc", previewUrl: "data:image/png;base64,abc",
};
const textAtt: Attachment = {
  id: "2", kind: "text", name: "notes.md", mimeType: "text/markdown", data: "# hello",
};
const binaryAtt: Attachment = {
  id: "3", kind: "binary", name: "archive.zip",
  mimeType: "application/zip", data: "/Users/x/archive.zip",
};

describe("buildContentParts", () => {
  it("text only → single text part", () => {
    const parts = buildContentParts("hello", []);
    expect(parts).toEqual([{ type: "text", text: "hello" }]);
  });

  it("image attachment → image_url part", () => {
    const parts = buildContentParts("", [imageAtt]);
    expect(parts[0]).toEqual({ type: "image_url", image_url: { url: "data:image/png;base64,abc" } });
  });

  it("text file → text part with filename header", () => {
    const parts = buildContentParts("", [textAtt]);
    expect(parts[0].type).toBe("text");
    expect((parts[0] as { type: "text"; text: string }).text).toContain("[notes.md]");
    expect((parts[0] as { type: "text"; text: string }).text).toContain("# hello");
  });

  it("binary → path as text part", () => {
    const parts = buildContentParts("", [binaryAtt]);
    expect(parts[0]).toEqual({ type: "text", text: "/Users/x/archive.zip" });
  });
});

describe("contentToDisplayString", () => {
  it("string passthrough", () => {
    expect(contentToDisplayString("hello")).toBe("hello");
  });

  it("array → extracts text parts", () => {
    const result = contentToDisplayString([
      { type: "text", text: "describe" },
      { type: "image_url", image_url: { url: "data:..." } },
    ]);
    expect(result).toBe("describe");
  });
});
```

- [ ] **Step 5: Run tests — expect pass**

```bash
npm run test -- src/types/attachment.test.ts 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 6: Type-check**

```bash
npx tsc --noEmit 2>&1 | head -30
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add src/types/attachment.ts src/types/attachment.test.ts src/ipc/ai.ts
git commit -m "feat(frontend): add Attachment type, ContentPart, and widen ChatMessage.content"
```

---

## Task 6: Frontend — Extend `useAiChat.send()` for attachments

**Files:**
- Modify: `src/hooks/useAiChat.ts`

- [ ] **Step 1: Write a failing test**

In `src/hooks/useAiChat.test.ts` (create if it doesn't exist, otherwise add to existing):

Check if a test file exists first:
```bash
ls src/hooks/useAiChat.test.ts 2>/dev/null || echo "not found"
```

If not found, create `src/hooks/useAiChat.test.ts`:

```typescript
import { describe, it, expect, vi, beforeEach } from "vitest";
import { buildContentParts } from "../types/attachment";
import type { Attachment } from "../types/attachment";

// Test the buildContentParts helper used by useAiChat.send()
// (useAiChat itself is a React hook — we test the pure helper logic here)
describe("send with attachments uses buildContentParts", () => {
  it("image attachment produces ContentPart array", () => {
    const att: Attachment = {
      id: "1", kind: "image", name: "shot.png",
      mimeType: "image/png", data: "data:image/png;base64,xyz", previewUrl: "data:image/png;base64,xyz",
    };
    const parts = buildContentParts("what is this?", [att]);
    expect(parts).toHaveLength(2);
    expect(parts[0]).toMatchObject({ type: "text", text: "what is this?" });
    expect(parts[1]).toMatchObject({ type: "image_url" });
  });
});
```

- [ ] **Step 2: Run — expect pass** (this tests the helper we already wrote)

```bash
npm run test -- src/hooks/useAiChat.test.ts 2>&1 | tail -10
```

- [ ] **Step 3: Update `UseAiChatResult` interface and `send()` in `src/hooks/useAiChat.ts`**

In the `UseAiChatResult` interface, change the `send` signature:

```typescript
// Before:
send: (userText: string) => Promise<void>;

// After:
send: (userText: string, attachments?: import("../types/attachment").Attachment[]) => Promise<void>;
```

Add the import at the top of the file:

```typescript
import { buildContentParts, contentToDisplayString } from "../types/attachment";
import type { Attachment } from "../types/attachment";
```

Update `formatSessionTitle`:

```typescript
// Before:
function formatSessionTitle(messages: ChatMessage[]): string {
  const first = messages.find((m) => m.role === "user");
  return first ? first.content.slice(0, 30) : "（空對話）";
}

// After:
function formatSessionTitle(messages: ChatMessage[]): string {
  const first = messages.find((m) => m.role === "user");
  if (!first) return "（空對話）";
  const text = contentToDisplayString(first.content);
  return text.slice(0, 30) || "（附件）";
}
```

Update the `send` callback:

```typescript
// Before:
const send = useCallback(
  async (userText: string) => {
    if (isStreaming) return;
    const userMsg: ChatMessage = { role: "user", content: userText };
    const next = truncateHistory([...messages, userMsg], HISTORY_LIMIT);
    setMessages(next);
    await invokeChat(next);
  },
  [messages, isStreaming, invokeChat],
);

// After:
const send = useCallback(
  async (userText: string, attachments?: Attachment[]) => {
    if (isStreaming) return;
    const content =
      attachments && attachments.length > 0
        ? buildContentParts(userText, attachments)
        : userText;
    const userMsg: ChatMessage = { role: "user", content };
    const next = truncateHistory([...messages, userMsg], HISTORY_LIMIT);
    setMessages(next);
    await invokeChat(next);
  },
  [messages, isStreaming, invokeChat],
);
```

- [ ] **Step 4: Type-check**

```bash
npx tsc --noEmit 2>&1 | head -30
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useAiChat.ts src/hooks/useAiChat.test.ts
git commit -m "feat(hooks): extend useAiChat.send() to accept Attachment[] for multimodal messages"
```

---

## Task 7: Frontend — MessageBubble handles `ContentPart[]` for user messages

**Files:**
- Modify: `src/components/AiPanel/MessageBubble.tsx`
- Modify: `src/components/AiPanel/MessageList.tsx`

- [ ] **Step 1: Update `MessageBubble` to accept `string | ContentPart[]`**

In `src/components/AiPanel/MessageBubble.tsx`, update the import and props:

```typescript
// Add import at top:
import type { ContentPart } from "../../ipc/ai";
```

Change the props interface:

```typescript
// Before:
interface MessageBubbleProps {
  role: "user" | "assistant";
  content: string;
  onExecuteCommand: (cmd: string) => void;
  streaming?: boolean;
}

// After:
interface MessageBubbleProps {
  role: "user" | "assistant";
  content: string | ContentPart[];
  onExecuteCommand: (cmd: string) => void;
  streaming?: boolean;
}
```

Update the user bubble render block:

```typescript
// Before:
if (role === "user") {
  return (
    <div className="aiterm-bubble aiterm-bubble-user">
      <span>{content}</span>
    </div>
  );
}

// After:
if (role === "user") {
  return (
    <div className="aiterm-bubble aiterm-bubble-user">
      {typeof content === "string" ? (
        <span>{content}</span>
      ) : (
        <div className="aiterm-bubble-multipart">
          {content.map((part, i) =>
            part.type === "text" ? (
              <span key={i}>{part.text}</span>
            ) : (
              <img
                key={i}
                src={part.image_url.url}
                alt="attached image"
                className="aiterm-bubble-image-thumb"
              />
            )
          )}
        </div>
      )}
    </div>
  );
}
```

For the assistant path, `content` must be a string (AI always returns strings). Add a guard before `unescapeNewlines`:

```typescript
// Before (assistant path):
const cleaned = unescapeNewlines(extractResponseText(content));

// After:
const cleaned = unescapeNewlines(extractResponseText(typeof content === "string" ? content : ""));
```

- [ ] **Step 2: Add thumbnail CSS to `src/components/AiPanel/styles.css`**

Find the AI Panel CSS file (likely `src/components/AiPanel/styles.css`). Add:

```css
.aiterm-bubble-multipart {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.aiterm-bubble-image-thumb {
  max-width: 200px;
  max-height: 150px;
  border-radius: 4px;
  object-fit: contain;
}
```

- [ ] **Step 3: Update `MessageList.tsx` — the TypeScript compiler will guide this**

`MessageList.tsx:35` passes `content={m.content}` to `MessageBubble`. Since `ChatMessage.content` is now `string | ContentPart[]` and `MessageBubble.content` accepts the same type, this should compile without changes. Verify:

```bash
npx tsc --noEmit 2>&1 | grep MessageList
```

Expected: no errors for MessageList.

- [ ] **Step 4: Run frontend tests**

```bash
npm run test 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/AiPanel/MessageBubble.tsx src/components/AiPanel/styles.css
git commit -m "feat(ui): MessageBubble renders ContentPart[] with image thumbnails for user messages"
```

---

## Task 8: Frontend — AI Panel paste/drop + attachment preview UI

**Files:**
- Modify: `src/components/AiPanel/index.tsx`

- [ ] **Step 1: Add attachment state and paste/drop handlers**

In `src/components/AiPanel/index.tsx`, add the import at the top:

```typescript
import { readFileAsAttachment } from "../../types/attachment";
import type { Attachment } from "../../types/attachment";
```

Add attachment state after the existing state declarations (around line 51, after `const [input, setInput] = useState("");`):

```typescript
const [attachments, setAttachments] = useState<Attachment[]>([]);
const MAX_IMAGE_BYTES = 5 * 1024 * 1024; // 5 MB
```

Add `processFiles` helper (add as a `useCallback` after the state declarations):

```typescript
const processFiles = useCallback(async (files: FileList | File[]) => {
  const arr = Array.from(files);
  const results = await Promise.allSettled(arr.map(async (file) => {
    if (file.type.startsWith("image/") && file.size > MAX_IMAGE_BYTES) {
      throw new Error(`${file.name} 超過 5MB 限制`);
    }
    return readFileAsAttachment(file);
  }));
  const valid = results
    .filter((r): r is PromiseFulfilledResult<Attachment> => r.status === "fulfilled")
    .map((r) => r.value);
  setAttachments((prev) => [...prev, ...valid]);
}, []);
```

Add paste handler on the textarea. Find the textarea element and add `onPaste`:

```typescript
const handlePaste = useCallback(async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
  const files = e.clipboardData?.files;
  if (files && files.length > 0) {
    e.preventDefault();
    await processFiles(files);
  }
  // No files → let default text paste proceed
}, [processFiles]);
```

Add drag-and-drop handlers on the panel container:

```typescript
const handleDragOver = useCallback((e: React.DragEvent<HTMLDivElement>) => {
  e.preventDefault();
}, []);

const handleDrop = useCallback(async (e: React.DragEvent<HTMLDivElement>) => {
  e.preventDefault();
  const files = e.dataTransfer?.files;
  if (files && files.length > 0) {
    await processFiles(files);
  }
}, [processFiles]);
```

- [ ] **Step 2: Wire handlers into the JSX**

Find the outer panel `<div>` (around line 308):

```tsx
// Before:
<div className={panelClass} aria-hidden={!isOpen} style={{ width: `${panelWidth}px` }}>

// After:
<div
  className={panelClass}
  aria-hidden={!isOpen}
  style={{ width: `${panelWidth}px` }}
  onDragOver={handleDragOver}
  onDrop={handleDrop}
>
```

Find the textarea and add `onPaste={handlePaste}`.

- [ ] **Step 3: Add attachment preview pills above the textarea**

Locate the input area section in the JSX (the `<textarea>` or its container). Add the pills directly above it:

```tsx
{attachments.length > 0 && (
  <div className="aiterm-attachment-pills">
    {attachments.map((att) => (
      <div key={att.id} className="aiterm-attachment-pill">
        {att.kind === "image" && att.previewUrl ? (
          <img src={att.previewUrl} alt={att.name} className="aiterm-pill-thumb" />
        ) : att.kind === "text" ? (
          <span>📄</span>
        ) : (
          <span>📎</span>
        )}
        <span className="aiterm-pill-name">{att.name}</span>
        <button
          type="button"
          className="aiterm-pill-remove"
          onClick={() => setAttachments((prev) => prev.filter((a) => a.id !== att.id))}
          title="移除"
        >
          ×
        </button>
      </div>
    ))}
  </div>
)}
```

- [ ] **Step 4: Pass attachments to `chat.send()` and clear after send**

Update `handleSubmit`:

```typescript
// Before:
const handleSubmit = () => {
  const text = input.trim();
  if (!text || chat.isStreaming || agentRunning) return;
  setInput("");
  if (agentMode) {
    void submitAgent(text);
  } else {
    void chat.send(text);
  }
};

// After:
const handleSubmit = () => {
  const text = input.trim();
  if ((!text && attachments.length === 0) || chat.isStreaming || agentRunning) return;
  setInput("");
  const currentAttachments = attachments;
  setAttachments([]);
  if (agentMode) {
    void submitAgent(text);
  } else {
    void chat.send(text, currentAttachments.length > 0 ? currentAttachments : undefined);
  }
};
```

- [ ] **Step 5: Add pill CSS to `src/components/AiPanel/styles.css`**

```css
.aiterm-attachment-pills {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  padding: 6px 8px 0;
}

.aiterm-attachment-pill {
  display: flex;
  align-items: center;
  gap: 4px;
  background: var(--panel-bg, #2a2a2a);
  border: 1px solid var(--border-color, #444);
  border-radius: 6px;
  padding: 2px 6px 2px 4px;
  font-size: 12px;
  max-width: 160px;
}

.aiterm-pill-thumb {
  width: 24px;
  height: 24px;
  object-fit: cover;
  border-radius: 3px;
}

.aiterm-pill-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 100px;
}

.aiterm-pill-remove {
  background: none;
  border: none;
  cursor: pointer;
  color: var(--text-muted, #888);
  padding: 0 2px;
  font-size: 14px;
  line-height: 1;
}

.aiterm-pill-remove:hover {
  color: var(--text-primary, #fff);
}
```

- [ ] **Step 6: Type-check and lint**

```bash
npx tsc --noEmit 2>&1 | head -20 && npm run lint 2>&1 | grep -E "error|warning" | head -20
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add src/components/AiPanel/index.tsx src/components/AiPanel/styles.css
git commit -m "feat(ui): AI Panel supports paste/drop file attachments with preview pills"
```

---

## Task 9: Frontend — WarpInput paste/drop → path insertion

**Files:**
- Modify: `src/components/WarpInput.tsx`

- [ ] **Step 1: Add paste handler**

In `src/components/WarpInput.tsx`, add a helper function before the component:

```typescript
/** Insert text at the current cursor position in a textarea. */
function insertAtCursor(el: HTMLTextAreaElement, text: string) {
  const start = el.selectionStart ?? el.value.length;
  const end = el.selectionEnd ?? el.value.length;
  el.value = el.value.slice(0, start) + text + el.value.slice(end);
  el.selectionStart = el.selectionEnd = start + text.length;
}
```

Add a paste handler. In the `<textarea>` element, add `onPaste`:

```typescript
const handlePaste = useCallback(async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
  const files = e.clipboardData?.files;
  if (!files || files.length === 0) return; // let default text paste proceed
  e.preventDefault();
  const paths = Array.from(files)
    .map((f) => (f as File & { path?: string }).path ?? f.name)
    .join(" ");
  const el = textareaRef.current;
  if (!el) return;
  insertAtCursor(el, paths);
  // Sync React state with the imperative value change
  setValue(el.value);
  // Trigger height resize
  el.style.height = "auto";
  el.style.height = `${el.scrollHeight}px`;
}, []);
```

Add drag-and-drop handlers:

```typescript
const handleDragOver = useCallback((e: React.DragEvent<HTMLTextAreaElement>) => {
  e.preventDefault();
}, []);

const handleDrop = useCallback(async (e: React.DragEvent<HTMLTextAreaElement>) => {
  e.preventDefault();
  const files = e.dataTransfer?.files;
  if (!files || files.length === 0) return;
  const paths = Array.from(files)
    .map((f) => (f as File & { path?: string }).path ?? f.name)
    .join(" ");
  const el = textareaRef.current;
  if (!el) return;
  insertAtCursor(el, paths);
  setValue(el.value);
  el.style.height = "auto";
  el.style.height = `${el.scrollHeight}px`;
}, []);
```

Wire into the `<textarea>` element:

```tsx
<textarea
  ref={textareaRef}
  className="warp-input-textarea"
  value={value}
  onChange={handleChange}
  onKeyDown={handleKeyDown}
  onPaste={handlePaste}
  onDragOver={handleDragOver}
  onDrop={handleDrop}
  placeholder={...}
  rows={1}
  disabled={disabled}
/>
```

- [ ] **Step 2: Type-check**

```bash
npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/components/WarpInput.tsx
git commit -m "feat(ui): WarpInput inserts file path on paste/drop"
```

---

## Task 10: Frontend — TerminalView paste/drop → write path to PTY

**Files:**
- Modify: `src/components/TerminalView.tsx`

- [ ] **Step 1: Add paste and drop handlers on `hostRef`**

In `src/components/TerminalView.tsx`, after the existing `useEffect` blocks (around where font/theme effects are), add:

```typescript
// File paste/drop → write absolute path to PTY
useEffect(() => {
  const el = hostRef.current;
  if (!el) return;

  const handlePaste = async (e: ClipboardEvent) => {
    const files = e.clipboardData?.files;
    if (!files || files.length === 0) return; // let xterm handle text paste
    e.preventDefault();
    e.stopPropagation();
    const sid = sessionRef.current;
    if (!sid) return;
    const paths = Array.from(files)
      .map((f) => (f as File & { path?: string }).path ?? f.name)
      .join(" ");
    await writePty(sid, paths).catch(() => {});
  };

  const handleDragOver = (e: DragEvent) => {
    e.preventDefault();
  };

  const handleDrop = async (e: DragEvent) => {
    e.preventDefault();
    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) return;
    const sid = sessionRef.current;
    if (!sid) return;
    const paths = Array.from(files)
      .map((f) => (f as File & { path?: string }).path ?? f.name)
      .join(" ");
    await writePty(sid, paths + " ").catch(() => {});
  };

  el.addEventListener("paste", handlePaste);
  el.addEventListener("dragover", handleDragOver);
  el.addEventListener("drop", handleDrop);
  return () => {
    el.removeEventListener("paste", handlePaste);
    el.removeEventListener("dragover", handleDragOver);
    el.removeEventListener("drop", handleDrop);
  };
}, []); // hostRef is stable; sessionRef is a ref so no dep needed
```

> `writePty` and `sessionRef` are already available in scope. The `useEffect` has an empty dep array because both `hostRef` and `sessionRef` are refs (stable references).

- [ ] **Step 2: Type-check**

```bash
npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 3: Run all frontend tests**

```bash
npm run test 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add src/components/TerminalView.tsx
git commit -m "feat(ui): TerminalView writes file path to PTY on paste/drop"
```

---

## Final Verification

- [ ] **Full Rust test suite**

```bash
cd src-tauri && cargo test 2>&1 | tail -10
```

Expected: `test result: ok. N passed; 0 failed`

- [ ] **Full frontend test suite**

```bash
cd .. && npm run test 2>&1 | tail -10
```

Expected: all pass.

- [ ] **Type check**

```bash
npx tsc --noEmit 2>&1
```

Expected: no output (zero errors).

- [ ] **Lint**

```bash
npm run lint 2>&1 | grep -E "^src" | head -20
```

Expected: no new errors.
