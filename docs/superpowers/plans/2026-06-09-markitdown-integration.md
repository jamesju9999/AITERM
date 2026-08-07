# MarkItDown 整合 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 將 Microsoft MarkItDown Python 函式庫整合進 DocConverterView，取代現有三個 JS 前端解析器（xlsx/mammoth/pdfjs-dist），並擴充支援格式（.pptx、.html、圖片等）。

**Architecture:** Python sidecar 模式（與 ApiDocFetcher 相同）：前端呼叫 Tauri IPC `markitdown_convert(file_path)`，Rust 自動安裝 pip 依賴後執行 `tools/MarkItDown/converter.py`，透過 JSON line protocol 回傳 Markdown 字串。檔案選擇改用 Rust 原生 `rfd` 對話框（已存在於 codebase 中），OS drag-drop 維持不變。

**Tech Stack:** Python 3 + markitdown[all]、Rust/Tokio subprocess、rfd 0.15（已安裝）、React 19 + Tauri IPC

---

## 檔案結構

| 操作 | 路徑 | 責任 |
|------|------|------|
| 新增 | `tools/MarkItDown/converter.py` | Python 腳本：接受 file_path 參數，輸出 JSON line |
| 新增 | `tools/MarkItDown/requirements.txt` | `markitdown[all]` |
| 新增 | `src-tauri/src/commands/markitdown.rs` | Tauri commands: `markitdown_convert` + `markitdown_pick_file` |
| 修改 | `src-tauri/src/commands/mod.rs` | 新增 `pub mod markitdown;` |
| 修改 | `src-tauri/src/lib.rs` | 在 invoke_handler 中註冊兩個新 command |
| 新增 | `src/ipc/markitdown.ts` | 前端 IPC wrapper |
| 修改 | `src/components/DocConverter/DocConverterView.tsx` | 移除 JS 解析器，改用新 IPC |
| 修改 | `src/lib/i18n.ts` | 更新 `dc_subtitle` 和 `dc_dropzone_formats` |
| 修改 | `src-tauri/tauri.macos.conf.json` | 新增 MarkItDown 資源 bundle |
| 修改 | `src-tauri/tauri.windows.conf.json` | 新增 MarkItDown 資源 bundle |
| 修改 | `src-tauri/tauri.linux.conf.json` | 新增 MarkItDown 資源 bundle |

---

## Task 1: Python converter 腳本

**Files:**
- Create: `tools/MarkItDown/requirements.txt`
- Create: `tools/MarkItDown/converter.py`

- [ ] **Step 1: 建立 requirements.txt**

```
markitdown[all]
```

- [ ] **Step 2: 建立 converter.py**

```python
#!/usr/bin/env python3
"""
MarkItDown converter — stdin/stdout JSON line protocol.
Usage: python converter.py <file_path>

Stdout (exactly one line):
  {"type": "done", "markdown": "<converted text>"}
  {"type": "error", "message": "<error description>"}
"""
import sys
import json


def main() -> None:
    if len(sys.argv) < 2:
        print(json.dumps({"type": "error", "message": "Usage: converter.py <file_path>"}))
        sys.exit(1)

    file_path = sys.argv[1]
    try:
        from markitdown import MarkItDown
        md = MarkItDown()
        result = md.convert(file_path)
        print(json.dumps({"type": "done", "markdown": result.text_content}))
    except Exception as exc:  # noqa: BLE001
        print(json.dumps({"type": "error", "message": str(exc)}))
        sys.exit(1)


if __name__ == "__main__":
    main()
```

- [ ] **Step 3: 手動驗證腳本（需先 pip install markitdown[all]）**

```bash
pip install markitdown[all]
python tools/MarkItDown/converter.py /path/to/test.docx
# 預期輸出：{"type": "done", "markdown": "..."}
```

- [ ] **Step 4: commit**

```bash
git add tools/MarkItDown/
git commit -m "feat: add MarkItDown Python converter script"
```

---

## Task 2: Rust command

**Files:**
- Create: `src-tauri/src/commands/markitdown.rs`
- Modify: `src-tauri/src/commands/mod.rs`

- [ ] **Step 1: 建立 src-tauri/src/commands/markitdown.rs**

```rust
// src-tauri/src/commands/markitdown.rs
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tokio::io::AsyncBufReadExt;
use serde::Deserialize;

fn find_python() -> &'static str {
    if cfg!(target_os = "windows") { "python" } else { "python3" }
}

fn converter_script_path(app: &AppHandle) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_path = manifest_dir
        .parent()
        .unwrap_or(&manifest_dir)
        .join("tools")
        .join("MarkItDown")
        .join("converter.py");
    if dev_path.exists() {
        return dev_path;
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let prod_path = resource_dir.join("MarkItDown").join("converter.py");
        if prod_path.exists() {
            return prod_path;
        }
    }
    dev_path
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PythonLine {
    Done { markdown: String },
    Error { message: String },
}

/// Convert a local file to Markdown using MarkItDown.
/// Auto-installs Python deps on first use (fast no-op if already installed).
#[tauri::command]
pub async fn markitdown_convert(app: AppHandle, file_path: String) -> Result<String, String> {
    let script = converter_script_path(&app);
    let python = find_python();
    let script_dir = script.parent().unwrap_or(script.as_path());
    let req_file = script_dir.join("requirements.txt");

    // Auto-install deps (same pattern as api_docs/runner.rs)
    if req_file.exists() {
        #[allow(unused_mut)]
        let mut pip_cmd = tokio::process::Command::new(python);
        pip_cmd
            .args(["-m", "pip", "install", "-r"])
            .arg(&req_file)
            .args(["--quiet", "--disable-pip-version-check"])
            .current_dir(script_dir);
        #[cfg(target_os = "linux")]
        pip_cmd.args(["--user", "--break-system-packages"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            pip_cmd.creation_flags(0x08000000);
        }
        let _ = pip_cmd.output().await;
    }

    let mut cmd = tokio::process::Command::new(python);
    cmd.arg(&script)
        .arg(&file_path)
        .env("PYTHONIOENCODING", "utf-8")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn Python: {e}"))?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut reader = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let mut result: Option<String> = None;

    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<PythonLine>(line) {
            Ok(PythonLine::Done { markdown }) => {
                result = Some(markdown);
            }
            Ok(PythonLine::Error { message }) => {
                return Err(message);
            }
            Err(_) => {}
        }
    }

    let stderr_output = stderr_task.await.unwrap_or_default();
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() && result.is_none() {
        let detail = if stderr_output.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", stderr_output.trim())
        };
        return Err(format!(
            "converter.py exited with code {:?}{}",
            status.code(),
            detail
        ));
    }

    result.ok_or_else(|| "converter.py did not emit markdown".to_string())
}

/// Open a native OS file picker and return the selected path, or None if cancelled.
#[tauri::command]
pub async fn markitdown_pick_file() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .add_filter(
            "Documents",
            &[
                "xlsx", "xls", "csv", "docx", "pdf", "pptx", "html", "htm",
                "jpg", "jpeg", "png", "gif", "webp", "epub", "msg",
                "txt", "md", "rst", "xml", "json",
            ],
        )
        .pick_file()
        .await
        .map(|h| h.path().to_string_lossy().to_string())
}
```

- [ ] **Step 2: 在 src-tauri/src/commands/mod.rs 加入 markitdown module**

在現有 `pub mod web;` 之後加入：
```rust
pub mod markitdown;
```

完整結果：
```rust
pub mod ai;
pub mod api_docs;
pub mod config;
pub mod db;
pub mod design;
pub mod enterprise;
pub mod markitdown;
pub mod provider;
pub mod secret;
pub mod shell;
pub mod vcs;
pub mod web;
```

- [ ] **Step 3: 驗證編譯**

```bash
cd src-tauri && cargo check 2>&1 | head -30
# 預期：無 error（可能有 unused import warning，之後清理）
```

- [ ] **Step 4: commit**

```bash
git add src-tauri/src/commands/markitdown.rs src-tauri/src/commands/mod.rs
git commit -m "feat(rust): add markitdown_convert and markitdown_pick_file commands"
```

---

## Task 3: 在 lib.rs 註冊新 commands

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 加入 import**

在 `src-tauri/src/lib.rs` 的 commands imports 區塊加入：

```rust
    commands::markitdown::{markitdown_convert, markitdown_pick_file},
```

完整 commands imports 的最後兩行（加在 `vcs_detect_repo,` 等的 use block 結尾前，或加在末尾的 `};` 前）：

找到這一行：
```rust
use commands::{
    api_docs::{
```

在整個 `use commands::{...};` 區塊內，於 `shell::open_url,` 之後加入：
```rust
    markitdown::{markitdown_convert, markitdown_pick_file},
```

- [ ] **Step 2: 在 invoke_handler 加入兩個 command**

在 `lib.rs` 的 `tauri::generate_handler![` 區塊，找到：
```rust
            // API Docs
            api_docs_detect,
```

在該段落之前插入：
```rust
            // MarkItDown
            markitdown_convert,
            markitdown_pick_file,
```

- [ ] **Step 3: 驗證編譯**

```bash
cd src-tauri && cargo check 2>&1 | head -30
# 預期：無 error
```

- [ ] **Step 4: commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(rust): register markitdown commands in invoke_handler"
```

---

## Task 4: 前端 IPC wrapper

**Files:**
- Create: `src/ipc/markitdown.ts`

- [ ] **Step 1: 建立 src/ipc/markitdown.ts**

```typescript
import { invoke } from "@tauri-apps/api/core";

/**
 * Convert a local file to Markdown using MarkItDown (Python backend).
 * Resolves with the Markdown string, rejects with an error message on failure.
 */
export function markitdownConvert(filePath: string): Promise<string> {
  return invoke<string>("markitdown_convert", { filePath });
}

/**
 * Open a native OS file picker filtered to supported document formats.
 * Resolves with the selected file path, or null if the user cancelled.
 */
export function markitdownPickFile(): Promise<string | null> {
  return invoke<string | null>("markitdown_pick_file");
}
```

- [ ] **Step 2: 型別檢查**

```bash
npx tsc --noEmit 2>&1 | head -20
# 預期：無 error（新檔案只有 invoke 呼叫，不應有型別問題）
```

- [ ] **Step 3: commit**

```bash
git add src/ipc/markitdown.ts
git commit -m "feat(ipc): add markitdown IPC wrapper"
```

---

## Task 5: 更新 i18n 字串

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 更新 zh-TW 字串**

找到（`src/lib/i18n.ts` 約第 184-187 行）：
```typescript
    dc_title: "文件轉換器",
    dc_subtitle: "將 Word / PDF / Excel 資料字典轉換成結構化 Markdown Schema 文件",
    dc_dropzone_hint: "拖放或點擊選擇檔案",
    dc_dropzone_formats: "支援 .xlsx .xls .csv .docx .pdf",
```

替換為：
```typescript
    dc_title: "文件轉換器",
    dc_subtitle: "將文件轉換成 Markdown（支援 Office、PDF、圖片、網頁等格式）",
    dc_dropzone_hint: "拖放或點擊選擇檔案",
    dc_dropzone_formats: "支援 .xlsx .docx .pdf .pptx .html .jpg .png 等格式",
```

- [ ] **Step 2: 更新 en 字串**

找到（約第 458-461 行）：
```typescript
    dc_title: "Doc Converter",
    dc_subtitle: "Convert Word / PDF / Excel data dictionaries to structured Markdown schema docs",
    dc_dropzone_hint: "Drag & drop or click to select a file",
    dc_dropzone_formats: "Supports .xlsx .xls .csv .docx .pdf",
```

替換為：
```typescript
    dc_title: "Doc Converter",
    dc_subtitle: "Convert documents to Markdown (Office, PDF, images, HTML, and more)",
    dc_dropzone_hint: "Drag & drop or click to select a file",
    dc_dropzone_formats: "Supports .xlsx .docx .pdf .pptx .html .jpg .png and more",
```

- [ ] **Step 3: 型別檢查**

```bash
npx tsc --noEmit 2>&1 | head -20
# 預期：無 error
```

- [ ] **Step 4: commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(i18n): update DocConverter format descriptions for MarkItDown"
```

---

## Task 6: 重寫 DocConverterView

**Files:**
- Modify: `src/components/DocConverter/DocConverterView.tsx`

- [ ] **Step 1: 寫新的 DocConverterView.tsx**

完整替換 `src/components/DocConverter/DocConverterView.tsx`：

```tsx
// src/components/DocConverter/DocConverterView.tsx
import { useState, useRef, useCallback, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { aiChat, formatAiError } from "../../ipc/ai";
import { markitdownConvert, markitdownPickFile } from "../../ipc/markitdown";
import { useLocale } from "../../contexts/LocaleContext";
import "./DocConverterView.css";

interface ExtractState {
  fileName: string;
  rawText: string;
}

const CHUNK_SIZE = 3500;

const NORMALIZATION_SYSTEM_PROMPT = `你是資料字典格式化工具。將輸入的原始文字整理成結構化的 Markdown 格式。

每個資料表輸出：
## TABLE_NAME
一行說明（如果有）

| 欄位名 | 型別 | 說明 |
|--------|------|------|
| 欄位1 | 型別 | 說明文字 |

規則：
1. 每張表必須以 ## 開頭的標題行（## 表名）
2. 欄位資訊放在 3 欄 Markdown 表格（欄位名 | 型別 | 說明）
3. 如果原文件未提供型別，填入 -
4. 只輸出 Markdown，不要加解釋文字或開場白
5. 保留原始的資料表名稱和欄位名稱（不要翻譯）`;

export function DocConverterView({ isActive: _isActive }: { isActive: boolean }) {
  const { t } = useLocale();
  const [extractState, setExtractState] = useState<ExtractState | null>(null);
  const [extracting, setExtracting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [downloadSuccess, setDownloadSuccess] = useState<string | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [mdOutput, setMdOutput] = useState<string>("");
  const [normalizing, setNormalizing] = useState(false);
  const [normalizeProgress, setNormalizeProgress] = useState<{ step: number; total: number } | null>(null);
  const stoppedRef = useRef(false);

  const processFilePath = useCallback(async (filePath: string) => {
    stoppedRef.current = true;
    setNormalizing(false);
    setNormalizeProgress(null);
    setError(null);
    setDownloadSuccess(null);
    setExtractState(null);
    setMdOutput("");
    setExtracting(true);
    try {
      const markdown = await markitdownConvert(filePath);
      const fileName = filePath.split(/[\\/]/).pop() ?? filePath;
      setExtractState({ fileName, rawText: markdown });
    } catch (e) {
      setError(`提取失敗：${String(e)}`);
    } finally {
      setExtracting(false);
    }
  }, []);

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      const def = list.find((p) => p.is_default);
      if (def) setSelectedProviderId(def.id);
    }).catch(console.error);
  }, []);

  // OS-level drag-drop (Tauri intercepts before the web DOM)
  useEffect(() => {
    type DragDropPayload = { paths: string[]; position?: unknown };
    const unlisten = listen<DragDropPayload>("tauri://drag-drop", async (event) => {
      const paths = event.payload?.paths;
      if (!paths?.length) return;
      processFilePath(paths[0]);
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [processFilePath]);

  const handleDropzoneClick = useCallback(async () => {
    const path = await markitdownPickFile();
    if (path) processFilePath(path);
  }, [processFilePath]);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    // Web-level drag-drop: extract path from DataTransfer (works for some browsers/OS combos)
    // The OS-level event listener above handles the common case on Tauri.
    const item = e.dataTransfer.items[0];
    if (item?.kind === "file") {
      const file = item.getAsFile();
      if (file) {
        // Tauri exposes the real path on File objects in a web context
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const path: string | undefined = (file as any).path;
        if (path) processFilePath(path);
      }
    }
  }, [processFilePath]);

  const normalizeWithAi = useCallback(async () => {
    if (!extractState) return;
    stoppedRef.current = false;
    setError(null);
    setDownloadSuccess(null);
    setMdOutput("");
    setNormalizing(true);

    const text = extractState.rawText;
    const totalChunks = Math.ceil(text.length / CHUNK_SIZE);
    setNormalizeProgress({ step: 0, total: totalChunks });

    const parts: string[] = [];
    for (let i = 0; i < totalChunks; i++) {
      if (stoppedRef.current) break;
      setNormalizeProgress({ step: i + 1, total: totalChunks });
      const chunk = text.slice(i * CHUNK_SIZE, (i + 1) * CHUNK_SIZE);
      try {
        const result = await aiChat(
          chunk,
          NORMALIZATION_SYSTEM_PROMPT,
          [],
          selectedProviderId || undefined,
        );
        parts.push(result.trim());
      } catch (e) {
        const aiErr = typeof e === "object" && e !== null && "kind" in e
          ? formatAiError(e as import("../../ipc/ai").AiError)
          : String(e);
        setError(`AI 正規化失敗（步驟 ${i + 1}）：${aiErr}`);
        break;
      }
    }

    setMdOutput(parts.join("\n\n"));
    setNormalizing(false);
    setNormalizeProgress(null);
  }, [extractState, selectedProviderId]);

  const downloadMd = useCallback(() => {
    if (!mdOutput) return;
    const baseName = extractState?.fileName.replace(/\.[^.]+$/, "") ?? "schema";
    const fileName = `${baseName}.md`;
    const blob = new Blob([mdOutput], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = fileName;
    a.click();
    URL.revokeObjectURL(url);
    setDownloadSuccess(t.dc_download_success(fileName));
    setTimeout(() => setDownloadSuccess(null), 4000);
  }, [mdOutput, extractState, t]);

  return (
    <div className="doc-converter">
      <div className="doc-converter__header">
        <h2>📄 {t.dc_title}</h2>
        <p>{t.dc_subtitle}</p>
      </div>

      <div
        className="doc-converter__dropzone"
        onDragOver={(e) => e.preventDefault()}
        onDrop={handleDrop}
        onClick={handleDropzoneClick}
      >
        {extracting ? (
          <span>{t.dc_extracting}</span>
        ) : (
          <>
            <span className="doc-converter__dropzone-icon">📂</span>
            <span>{t.dc_dropzone_hint}</span>
            <span className="doc-converter__dropzone-hint">{t.dc_dropzone_formats}</span>
          </>
        )}
      </div>

      {error && (
        <div className="doc-converter__error">{error}</div>
      )}

      {downloadSuccess && (
        <div className="doc-converter__success">{downloadSuccess}</div>
      )}

      <div className="doc-converter__toolbar">
        <span className="doc-converter__toolbar-label">{t.dc_model_label}</span>
        <select
          value={selectedProviderId}
          onChange={(e) => setSelectedProviderId(e.target.value)}
          className="doc-converter__select"
        >
          {providers.map((p) => (
            <option key={p.id} value={p.id}>
              {p.display_name} ({p.model}){p.is_default ? " ★" : ""}
            </option>
          ))}
          {providers.length === 0 && <option value="">{t.dc_model_none}</option>}
        </select>
      </div>

      {extractState && (
        <div className="doc-converter__raw-preview">
          <div className="doc-converter__raw-header">
            {t.dc_detected(extractState.fileName, "Markdown", extractState.rawText.length)}
          </div>
        </div>
      )}

      {extractState && !normalizing && (
        <div className="doc-converter__actions">
          <button
            className="doc-converter__btn doc-converter__btn--primary"
            onClick={normalizeWithAi}
            disabled={!selectedProviderId}
          >
            {t.dc_normalize_btn}
          </button>
          {mdOutput && (
            <button
              className="doc-converter__btn doc-converter__btn--secondary"
              onClick={downloadMd}
            >
              {t.dc_download_btn}
            </button>
          )}
        </div>
      )}

      {normalizing && (
        <div className="doc-converter__actions">
          <div className="doc-converter__progress">
            <span>⟳</span>
            <span>
              {t.dc_normalizing}
              {normalizeProgress && t.dc_normalizing_step(normalizeProgress.step, normalizeProgress.total)}
            </span>
          </div>
          <button
            className="doc-converter__btn doc-converter__btn--secondary"
            onClick={() => { stoppedRef.current = true; }}
          >
            {t.dc_stop_btn}
          </button>
        </div>
      )}

      {mdOutput && (
        <div className="doc-converter__preview">
          <div className="doc-converter__preview-header">
            <span className="doc-converter__preview-label">
              {t.dc_preview_label(mdOutput.length)}
            </span>
            <button
              className="doc-converter__btn doc-converter__btn--secondary"
              style={{ fontSize: 11, padding: "2px 8px" }}
              onClick={downloadMd}
            >
              {t.dc_download_btn}
            </button>
          </div>
          <pre className="doc-converter__preview-box">{mdOutput}</pre>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: 型別檢查**

```bash
npx tsc --noEmit 2>&1 | head -30
# 預期：無 error
```

- [ ] **Step 3: commit**

```bash
git add src/components/DocConverter/DocConverterView.tsx
git commit -m "feat(frontend): rewrite DocConverterView to use MarkItDown IPC"
```

---

## Task 7: 移除舊 npm 依賴

**Files:**
- Modify: `package.json` (via npm uninstall)

- [ ] **Step 1: 確認無其他使用者**

```bash
grep -r "from 'xlsx'\|from \"xlsx\"\|from 'mammoth'\|from \"mammoth\"\|pdfjs-dist" src/ --include="*.ts" --include="*.tsx"
# 預期：無輸出（確認只有 DocConverterView 使用這些依賴）
```

- [ ] **Step 2: 移除 npm 套件**

```bash
npm uninstall xlsx mammoth pdfjs-dist
```

- [ ] **Step 3: 前端 lint + 型別檢查**

```bash
npm run lint 2>&1 | head -30
npx tsc --noEmit 2>&1 | head -30
# 預期：無 error
```

- [ ] **Step 4: commit**

```bash
git add package.json package-lock.json
git commit -m "chore: remove xlsx mammoth pdfjs-dist (replaced by MarkItDown)"
```

---

## Task 8: 更新 Tauri bundle 資源設定

**Files:**
- Modify: `src-tauri/tauri.macos.conf.json`
- Modify: `src-tauri/tauri.windows.conf.json`
- Modify: `src-tauri/tauri.linux.conf.json`

- [ ] **Step 1: 更新 tauri.macos.conf.json**

找到現有 `resources` 物件，加入 MarkItDown 條目：

```json
{
  "bundle": {
    "externalBin": [],
    "resources": {
      "binaries/db2-sidecar-mac-arm64": "db2-sidecar",
      "../tools/ApiDocFetcher/*.py": "ApiDocFetcher/",
      "../tools/ApiDocFetcher/strategies/*.py": "ApiDocFetcher/strategies/",
      "../tools/ApiDocFetcher/requirements.txt": "ApiDocFetcher/requirements.txt",
      "../tools/MarkItDown/converter.py": "MarkItDown/converter.py",
      "../tools/MarkItDown/requirements.txt": "MarkItDown/requirements.txt"
    }
  }
}
```

- [ ] **Step 2: 更新 tauri.windows.conf.json**

```json
{
  "bundle": {
    "externalBin": [],
    "resources": {
      "binaries/db2-sidecar-win-x64": "db2-sidecar",
      "resources/ApiDocFetcher/*.py": "ApiDocFetcher/",
      "resources/ApiDocFetcher/strategies/*.py": "ApiDocFetcher/strategies/",
      "resources/ApiDocFetcher/requirements.txt": "ApiDocFetcher/requirements.txt",
      "../tools/MarkItDown/converter.py": "MarkItDown/converter.py",
      "../tools/MarkItDown/requirements.txt": "MarkItDown/requirements.txt"
    }
  }
}
```

- [ ] **Step 3: 更新 tauri.linux.conf.json**

```json
{
  "bundle": {
    "externalBin": [],
    "resources": {
      "../tools/ApiDocFetcher/*.py": "ApiDocFetcher/",
      "../tools/ApiDocFetcher/strategies/*.py": "ApiDocFetcher/strategies/",
      "../tools/ApiDocFetcher/requirements.txt": "ApiDocFetcher/requirements.txt",
      "../tools/MarkItDown/converter.py": "MarkItDown/converter.py",
      "../tools/MarkItDown/requirements.txt": "MarkItDown/requirements.txt"
    }
  }
}
```

- [ ] **Step 4: commit**

```bash
git add src-tauri/tauri.macos.conf.json src-tauri/tauri.windows.conf.json src-tauri/tauri.linux.conf.json
git commit -m "feat(bundle): include MarkItDown Python script in app resources"
```

---

## Task 9: 前端測試更新

**Files:**
- Modify 或 Create: `src/components/DocConverter/DocConverterView.test.tsx`

- [ ] **Step 1: 確認現有測試**

```bash
ls src/components/DocConverter/
# 如果沒有 DocConverterView.test.tsx，就新增
```

- [ ] **Step 2: 寫測試**

建立（或替換）`src/components/DocConverter/DocConverterView.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { DocConverterView } from "./DocConverterView";

// Mock IPC
vi.mock("../../ipc/markitdown", () => ({
  markitdownConvert: vi.fn(),
  markitdownPickFile: vi.fn(),
}));
vi.mock("../../ipc/provider", () => ({
  listProviders: vi.fn().mockResolvedValue([]),
}));
vi.mock("../../ipc/ai", () => ({
  aiChat: vi.fn(),
  formatAiError: vi.fn((e) => String(e)),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { markitdownConvert, markitdownPickFile } from "../../ipc/markitdown";
import { LocaleProvider } from "../../contexts/LocaleContext";

function renderView() {
  return render(
    <LocaleProvider>
      <DocConverterView isActive={true} />
    </LocaleProvider>
  );
}

describe("DocConverterView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders dropzone", () => {
    renderView();
    expect(screen.getByText(/拖放或點擊選擇檔案/)).toBeInTheDocument();
  });

  it("calls markitdownPickFile when dropzone is clicked", async () => {
    vi.mocked(markitdownPickFile).mockResolvedValue(null);
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(markitdownPickFile).toHaveBeenCalledOnce();
  });

  it("calls markitdownConvert with picked path and shows extracted state", async () => {
    vi.mocked(markitdownPickFile).mockResolvedValue("/tmp/test.docx");
    vi.mocked(markitdownConvert).mockResolvedValue("# Hello\nworld");
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(markitdownConvert).toHaveBeenCalledWith("/tmp/test.docx");
    expect(screen.getByText(/test\.docx/)).toBeInTheDocument();
  });

  it("shows error when markitdownConvert rejects", async () => {
    vi.mocked(markitdownPickFile).mockResolvedValue("/tmp/bad.xyz");
    vi.mocked(markitdownConvert).mockRejectedValue(new Error("unsupported format"));
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(screen.getByText(/提取失敗/)).toBeInTheDocument();
  });

  it("does nothing when file picker is cancelled (null path)", async () => {
    vi.mocked(markitdownPickFile).mockResolvedValue(null);
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(markitdownConvert).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 3: 執行測試**

```bash
npm run test -- DocConverterView 2>&1 | tail -20
# 預期：全部通過
```

- [ ] **Step 4: commit**

```bash
git add src/components/DocConverter/DocConverterView.test.tsx
git commit -m "test: add DocConverterView tests for MarkItDown IPC"
```

---

## Task 10: 整合驗收

- [ ] **Step 1: 完整型別與 lint 檢查**

```bash
npm run lint 2>&1 | head -20
npx tsc --noEmit 2>&1 | head -20
cd src-tauri && cargo check 2>&1 | head -20
# 全部無 error
```

- [ ] **Step 2: 執行所有前端測試**

```bash
npm run test 2>&1 | tail -30
# 預期：全部通過，無 regression
```

- [ ] **Step 3: 手動驗收（需啟動 Tauri dev server）**

```bash
npm run tauri:dev
```

驗收清單：
- [ ] 點擊 dropzone → 開啟原生檔案選擇對話框 → 選擇 `.docx` → 顯示提取狀態
- [ ] 拖放 `.xlsx` 到 dropzone → 顯示提取狀態
- [ ] 拖放 `.pptx` 到 dropzone → 顯示提取狀態（新格式）
- [ ] 拖放 `.html` → 顯示提取狀態（新格式）
- [ ] 提取後按「AI 正規化」→ 正常運作
- [ ] 提取後按「下載」→ 正常運作

- [ ] **Step 4: 最終 commit**

```bash
git add -A
git commit -m "feat: integrate MarkItDown — replace JS parsers with Python backend"
```

---

## 注意事項

- **Python 必須存在於 PATH**：與 ApiDocFetcher 相同前提；若未安裝，`markitdown_convert` 回傳 "Failed to spawn Python" 錯誤。
- **首次使用**：`markitdown[all]` 約 50-100 MB，auto-pip-install 會有短暫延遲。後續為 no-op。
- **Windows 路徑**：`converter_script_path` 中使用與 `fetcher_script_path` 相同邏輯；Windows 的 tauri.windows.conf.json 用 `resources/` 前綴。
- **`dc_detected` i18n 呼叫**：原本傳入 `format.toUpperCase()`，現改傳 `"Markdown"`，確認 i18n function signature 接受此參數。
