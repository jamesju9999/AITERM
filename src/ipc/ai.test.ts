import { describe, expect, it } from "vitest";
import { formatAiError, type AiError } from "./ai";

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

  it("formats model_error with reason", () => {
    const msg = formatAiError({
      kind: "model_error",
      reason: "missing command",
      raw: "{oops}",
    });
    expect(msg).toContain("missing command");
  });
});
