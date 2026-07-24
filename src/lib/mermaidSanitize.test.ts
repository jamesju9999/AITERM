import { describe, it, expect } from "vitest";
import mermaid from "mermaid";
import { sanitizeMermaid } from "./mermaidSanitize";

mermaid.initialize({ startOnLoad: false, theme: "dark", securityLevel: "loose" });

// The diagram reported failing from the Knowledge Base — a rectangle label
// containing `{id}` was corrupted into `{"id"}` by the diamond-label pass,
// injecting a stray quote and producing a "got 'STR'" parse error.
const KB_DIAGRAM = `flowchart TD
    Start["請求發起"] --> CheckToken{"Access Token\\n是否過期?"}
    CheckToken -- "有效" --> UseToken["使用現有 Token\\n發出 API 請求"]
    CheckToken -- "即將過期 / 不存在" --> RefreshFlow["觸發 Token 刷新"]
    subgraph BrowserAuth ["瀏覽器授權流程 (Browser Flow)"]
        direction TB
        OpenBrowser["開啟系統預設瀏覽器"] -->|導向 Google OAuth 頁面| GooglePage["Google 授權頁面"]
        GooglePage --> UserAuth["使用者點擊 '允許'"]
        UserAuth --> Redirect["Google 重導向至\\n本地 Callback URL"]
    end
    RefreshFlow --> OpenBrowser
    subgraph LocalServer ["本地回調伺服器 (server.ts)"]
        direction TB
        CallbackReq["接收 HTTP 請求\\n包含 'code'"] --> ExchangeTokens["exchangeCodeForTokens()\\n交換 Code 為 Tokens"]
    end
    Redirect --> CallbackReq
    ExchangeTokens --> StoreTokens["儲存至 TokenStorage\\nKey: oauth:google:{id}"]
    StoreTokens --> GetToken["取得新的 Access Token"]
    GetToken --> UseToken
    style BrowserAuth fill:#e1f5fe
    style LocalServer fill:#fff9c4`;

describe("sanitizeMermaid", () => {
  it("does not corrupt {id} inside an already-quoted rectangle label", () => {
    const out = sanitizeMermaid(KB_DIAGRAM);
    // The bug rewrote {id} -> {"id"} inside the quoted label. It must stay {id}.
    expect(out).toContain("oauth:google:{id}");
    expect(out).not.toContain('{"id"}');
  });

  it("produces a diagram Mermaid can parse", async () => {
    const out = sanitizeMermaid(KB_DIAGRAM);
    await expect(mermaid.parse(out)).resolves.toBeTruthy();
  });

  it("still quotes a genuine unquoted diamond label", () => {
    const out = sanitizeMermaid("flowchart TD\n  A --> B{decide now}");
    expect(out).toContain('B{"decide now"}');
  });

  it("still quotes a genuine unquoted rectangle label", () => {
    const out = sanitizeMermaid("flowchart TD\n  A[hello world] --> B[done]");
    expect(out).toContain('A["hello world"]');
    expect(out).toContain('B["done"]');
  });

  it("still quotes a genuine unquoted edge label", () => {
    const out = sanitizeMermaid("flowchart TD\n  A -->|go here| B");
    expect(out).toContain('|"go here"|');
  });

  it("leaves an already-quoted diamond label untouched (no double-quoting)", () => {
    const out = sanitizeMermaid('flowchart TD\n  A --> C{"already quoted"}');
    expect(out).toContain('C{"already quoted"}');
    expect(out).not.toContain('{""');
  });
});
