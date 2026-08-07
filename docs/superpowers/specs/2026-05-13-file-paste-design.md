# File & Image Paste Design

**Date:** 2026-05-13  
**Status:** Approved  

## Overview

Allow users to paste or drag-and-drop files (images, text files, arbitrary binaries) into all three terminal entry points. Behavior is differentiated by entry point: the AI panel receives real multimodal attachments, while WarpInput and the PTY terminal insert the file's absolute path as text.

---

## Entry Points & Behavior

| Entry Point | Paste (Ctrl+V) | Drag-and-Drop | Result |
|-------------|---------------|---------------|--------|
| **AI Panel** | Intercept `clipboardData.files` | `drop` on panel container | Real attachment: image → base64, text file → inline content, binary → path text |
| **WarpInput** | Intercept `clipboardData.files` | `drop` on textarea | Insert absolute path at cursor |
| **PTY Terminal** | Intercept `paste` on host container | `drop` on host container | Write absolute path to PTY via `writePty()` |

---

## Data Model

### Attachment (frontend)

```typescript
// src/types/attachment.ts
export type AttachmentKind = "image" | "text" | "binary";

export interface Attachment {
  id: string;           // nanoid — unique identifier per attachment
  kind: AttachmentKind;
  name: string;         // original filename
  mimeType: string;
  // image → base64 data URI; text file → raw text content; binary → absolute path
  data: string;
  previewUrl?: string;  // thumbnail for images (same as data)
}
```

### ChatMessage (IPC)

`content` is widened from `string` to support OpenAI-style multipart arrays while remaining backward-compatible:

```typescript
// src/ipc/ai.ts
export type ContentPart =
  | { type: "text"; text: string }
  | { type: "image_url"; image_url: { url: string } }; // base64 data URI

export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string | ContentPart[];  // string = legacy, array = multimodal
}
```

---

## Frontend Components

### AI Panel (`src/components/AiPanel/index.tsx`)

**Event handling:**
- `paste` on textarea: read `clipboardData.files`/`clipboardData.items`
- `dragover` + `drop` on panel container div

**Classification logic:**
- `image/*` MIME → read as base64 data URI → `kind: "image"`
- `text/*` or `.md/.json/.csv/.ts/.tsx/...` → `FileReader.readAsText()` → `kind: "text"`
- Everything else → `kind: "binary"`, `data` = absolute path (from `file.path` — Tauri 2 injects this property on `File` objects in the webview)

**Attachment preview UI:**
- Rendered above the textarea as a horizontal pill row
- Image: thumbnail (32×32) + filename
- Text file: 📄 icon + filename
- Binary: 📎 icon + filename
- Each pill has an × button to remove

**On submit:**
- If `attachments` is non-empty, build `ContentPart[]`:
  - User text input → `{ type: "text", text }`
  - Image attachment → `{ type: "image_url", image_url: { url: dataUri } }`
  - Text file → append content as additional `{ type: "text" }` block with filename header
  - Binary → insert path as `{ type: "text" }` block
- Pass as `content: ContentPart[]` in `ChatMessage`
- Clear attachments state after send

### WarpInput (`src/components/WarpInput.tsx`)

- `paste` event: if `clipboardData.files.length > 0`, get file path and insert at cursor; else let default text paste proceed
- `drop` on textarea: read `dataTransfer.files`, insert each path at cursor, `preventDefault()`
- No IPC changes needed — output remains a plain string

### PTY Terminal (`src/components/TerminalView.tsx`)

- Add `paste` listener on `hostRef` container element (captures before xterm.js)
- If `clipboardData.files.length > 0`: call `writePty(sessionId, path + " ")`, stop propagation
- Add `dragover` (preventDefault) + `drop` listener on `hostRef`
- On drop: write each file path to PTY

---

## Rust Backend

### `ChatMessage` deserialization

```rust
// content field widened to accept both String and Array
#[derive(Deserialize, Serialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: serde_json::Value,  // Value::String | Value::Array
}
```

### OpenAI provider

Content array is OpenAI's native format — serialize directly when `content` is `Value::Array`. When `content` is `Value::String`, wrap as `[{ "type": "text", "text": "..." }]` for consistency.

### Anthropic provider

Requires a `convert_content_parts()` helper to map from OpenAI format to Anthropic format:

| OpenAI | Anthropic |
|--------|-----------|
| `{ type: "image_url", image_url: { url: "data:image/png;base64,..." } }` | `{ type: "image", source: { type: "base64", media_type: "image/png", data: "..." } }` |
| `{ type: "text", text: "..." }` | `{ type: "text", text: "..." }` |

### Ollama / OpenAI-compat

Pass content array through as-is (OpenAI-compat endpoint). Vision capability depends on the loaded model (e.g. llava, llama3.2-vision). No provider-side filtering — unsupported models return an error naturally.

---

## Error Handling

- Oversized images (>5MB): reject in frontend before IPC, show inline error in attachment preview
- Provider does not support vision: backend returns existing `AiError::ModelError` or `AiError::InvalidInput`; frontend displays via `formatAiError()`
- File read failure (permissions, etc.): catch in frontend, show inline error on the attachment pill

---

## Out of Scope

- PDF rendering or content extraction
- Multi-file upload progress bar
- Persistent attachment history across sessions
- File size limit configuration (hard-coded 5MB for images)
