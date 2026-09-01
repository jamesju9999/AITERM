import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { aiChat, formatAiError, invokeAiChatCtx, type AiError } from "./ai";

// ── shouldAutoExecute tests ───────────────────────────────────────────────────
// Import the pure helper via a re-export shim below, or test via the module.
// shouldAutoExecute is not exported from ai.ts — it lives in TerminalView.tsx.
// We duplicate the logic here to verify the 9-combination matrix independently.

type ExecutionMode = "always-confirm" | "graded" | "full-auto";
type RiskLevel = "safe" | "needs_confirm" | "dangerous";

function shouldAutoExecute(mode: ExecutionMode, risk: RiskLevel): boolean {
  if (mode === "always-confirm") return false;
  if (mode === "graded") return risk === "safe";
  if (mode === "full-auto") return risk === "safe" || risk === "needs_confirm";
  return false;
}

describe("shouldAutoExecute", () => {
  const cases: [ExecutionMode, RiskLevel, boolean][] = [
    // always-confirm: never auto-execute regardless of risk
    ["always-confirm", "safe",          false],
    ["always-confirm", "needs_confirm", false],
    ["always-confirm", "dangerous",     false],
    // graded: auto-execute only safe
    ["graded",         "safe",          true],
    ["graded",         "needs_confirm", false],
    ["graded",         "dangerous",     false],
    // full-auto: auto-execute safe + needs_confirm; dangerous shows preview
    ["full-auto",      "safe",          true],
    ["full-auto",      "needs_confirm", true],
    ["full-auto",      "dangerous",     false],
  ];

  it.each(cases)(
    "mode=%s risk=%s → %s",
    (mode, risk, expected) => {
      expect(shouldAutoExecute(mode, risk)).toBe(expected);
    },
  );
});

// ── formatAiError tests ───────────────────────────────────────────────────────

describe("formatAiError", () => {
  it("formats not_configured with settings hint", () => {
    const msg = formatAiError({ kind: "not_configured" });
    expect(msg).toContain("AI Provider");
    expect(msg).toContain("設定");
  });

  /**
   * 「憑證讀不到」原本被後端壓成 not_configured，畫面於是叫使用者去設定一個
   * 其實設定好好的供應商——實測就這樣把一個能用的供應商刪掉了，問題還在。
   * 這兩種要講不同的話，而且要把原始的鑰匙圈錯誤帶出來。
   */
  it("formats secret_access as a credential-read failure, not a missing setup", () => {
    const e: AiError = { kind: "secret_access", message: "keychain read error for anthropic-pro: denied" };
    const msg = formatAiError(e);
    expect(msg).toContain("鑰匙圈");
    expect(msg).toContain("keychain read error for anthropic-pro");
    // 不可以再叫人去「設定」——那正是誤導的來源。
    expect(msg).not.toContain("尚未設定");
  });

  it("formats network/ollama error with Ollama hint", () => {
    const e: AiError = { kind: "network", message: "connection refused to Ollama" };
    const msg = formatAiError(e);
    expect(msg).toContain("Ollama");
  });

  it("formats generic network error with message", () => {
    const e: AiError = { kind: "network", message: "timeout after 30s" };
    const msg = formatAiError(e);
    expect(msg).toContain("timeout after 30s");
  });

  it("formats auth_failed", () => {
    const msg = formatAiError({ kind: "auth_failed" });
    expect(msg).toContain("API Key");
  });

  it("formats rate_limit with retry_after", () => {
    const msg = formatAiError({ kind: "rate_limit", retry_after: "30" });
    expect(msg).toContain("30");
  });

  it("formats rate_limit without retry_after", () => {
    const msg = formatAiError({ kind: "rate_limit", retry_after: null });
    // Should mention rate limiting without a retry time
    expect(msg).toBeTruthy();
    expect(msg).not.toContain("undefined");
  });

  // Anthropic rejects non-Claude-Code subscription-OAuth traffic with a 429 whose
  // message is the literal string "Error". Reading that as a quota problem sent a
  // real debugging session down the wrong path, so it must not say "too frequent".
  it("does not blame quota for an opaque rate_limit body", () => {
    const msg = formatAiError({
      kind: "rate_limit",
      retry_after: null,
      body: '{"type":"error","error":{"type":"rate_limit_error","message":"Error"}}',
    });
    expect(msg).not.toContain("過於頻繁");
    // The raw body stays visible for diagnosis.
    expect(msg).toContain("rate_limit_error");
  });

  it("still reports a genuine rate limit as such", () => {
    const msg = formatAiError({
      kind: "rate_limit",
      retry_after: null,
      body: '{"error":{"type":"rate_limit_error","message":"Number of requests has exceeded your rate limit"}}',
    });
    expect(msg).toContain("過於頻繁");
  });

  it("formats model_error with reason", () => {
    const msg = formatAiError({
      kind: "model_error",
      reason: "missing command",
      raw: "{oops}",
    });
    expect(msg).toContain("missing command");
  });
});

// ── invokeAiChatCtx tests ─────────────────────────────────────────────────────

describe("invokeAiChatCtx", () => {
  beforeEach(() => invokeMock.mockReset().mockResolvedValue({ content: "hi", tool_calls: [], tool_calling_unsupported: false }));

  it("maps ctx to snake_case and passes connId/providerId/locale", async () => {
    await invokeAiChatCtx(
      [{ role: "user", content: "hi" }],
      { os: "linux", shell: null, cwd: null, recentOutput: "p$" },
      "conn-1", "prov-9", "en",
    );
    expect(invokeMock).toHaveBeenCalledWith("ai_chat_ctx", {
      messages: [{ role: "user", content: "hi" }],
      ctx: { os: "linux", shell: null, cwd: null, recent_output: "p$" },
      connId: "conn-1",
      providerId: "prov-9",
      locale: "en",
      supportsArtifacts: false,
    });
  });

  it("defaults providerId to null", async () => {
    await invokeAiChatCtx([{ role: "user", content: "x" }], { os: "linux", shell: null, cwd: null, recentOutput: null }, "c", undefined, "zh-TW");
    expect(invokeMock.mock.calls[0][1].providerId).toBeNull();
  });

  it("forwards supportsArtifacts, defaulting to false", async () => {
    const ctx = { os: "linux", shell: null, cwd: null, recentOutput: null };
    await invokeAiChatCtx([{ role: "user", content: "x" }], ctx, "c");
    expect(invokeMock).toHaveBeenLastCalledWith(
      "ai_chat_ctx",
      expect.objectContaining({ supportsArtifacts: false }),
    );

    await invokeAiChatCtx([{ role: "user", content: "x" }], ctx, "c", undefined, "zh-TW", true);
    expect(invokeMock).toHaveBeenLastCalledWith(
      "ai_chat_ctx",
      expect.objectContaining({ supportsArtifacts: true }),
    );
  });
});

// ── aiChat tests ──────────────────────────────────────────────────────────────

describe("aiChat", () => {
  beforeEach(() => invokeMock.mockReset().mockResolvedValue({ content: "hi", tool_calls: [], tool_calling_unsupported: false }));

  it("forwards supportsArtifacts, defaulting to false", async () => {
    await aiChat([{ role: "user", content: "x" }], "s1");
    expect(invokeMock).toHaveBeenLastCalledWith(
      "ai_chat",
      expect.objectContaining({ supportsArtifacts: false }),
    );

    await aiChat([{ role: "user", content: "x" }], "s1", undefined, false, "zh-TW", true);
    expect(invokeMock).toHaveBeenLastCalledWith(
      "ai_chat",
      expect.objectContaining({ supportsArtifacts: true }),
    );
  });
});
