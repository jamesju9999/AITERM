import { describe, expect, it } from "vitest";
import { formatAiError, type AiError } from "./ai";

describe("formatAiError", () => {
  it("formats not_configured with install hint", () => {
    const msg = formatAiError({ kind: "not_configured" });
    expect(msg).toContain("OPENAI_API_KEY");
    expect(msg).toContain("restart");
  });

  it("formats network with message", () => {
    const e: AiError = { kind: "network", message: "connection refused" };
    expect(formatAiError(e)).toContain("connection refused");
  });

  it("formats auth_failed", () => {
    expect(formatAiError({ kind: "auth_failed" })).toContain("authentication");
  });

  it("formats rate_limit with retry_after", () => {
    const msg = formatAiError({ kind: "rate_limit", retry_after: "30" });
    expect(msg).toContain("retry after 30");
  });

  it("formats rate_limit without retry_after", () => {
    const msg = formatAiError({ kind: "rate_limit", retry_after: null });
    expect(msg).toContain("rate limit");
    expect(msg).not.toContain("retry after");
  });

  it("formats model_error with reason and raw", () => {
    const msg = formatAiError({
      kind: "model_error",
      reason: "missing command",
      raw: "{oops}",
    });
    expect(msg).toContain("missing command");
    expect(msg).toContain("{oops}");
  });
});
