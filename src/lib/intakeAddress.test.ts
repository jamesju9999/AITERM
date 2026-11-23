import { describe, expect, it } from "vitest";
import { validAddress } from "./intakeAddress";

describe("validAddress", () => {
  it("accepts emails, Telegram chats and LINE user ids (same rules as the backend)", () => {
    for (const ok of ["a@b.co", "tg:12345", "tg:-1001234", "line:U0123456789abcdef0123456789abcdef", "LINE:u0123456789ABCDEF0123456789abcdef"]) {
      expect(validAddress(ok), ok).toBe(true);
    }
  });

  it("rejects malformed LINE ids", () => {
    for (const bad of ["line:", "line:U123", "line:X0123456789abcdef0123456789abcdef", "line:U0123456789abcdef0123456789abcdeg", "line:U0123456789abcdef0123456789abcdef0"]) {
      expect(validAddress(bad), bad).toBe(false);
    }
  });
});
