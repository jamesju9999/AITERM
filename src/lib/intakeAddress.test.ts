import { describe, expect, it } from "vitest";
import { displayAddress, validAddress } from "./intakeAddress";

describe("validAddress", () => {
  it("accepts emails, Telegram chats and LINE user ids (same rules as the backend)", () => {
    for (const ok of ["a@b.co", "tg:12345", "tg:-1001234", "line:U0123456789abcdef0123456789abcdef", "U0123456789abcdef0123456789abcdef", "  u0123456789ABCDEF0123456789abcdef ", "LINE:u0123456789ABCDEF0123456789abcdef"]) {
      expect(validAddress(ok), ok).toBe(true);
    }
  });

  it("rejects malformed LINE ids", () => {
    for (const bad of ["line:", "line:U123", "line:X0123456789abcdef0123456789abcdef", "line:U0123456789abcdef0123456789abcdeg", "line:U0123456789abcdef0123456789abcdef0", "U0123456789abcdef0123456789abcde", "U0123456789abcdef0123456789abcdefg"]) {
      expect(validAddress(bad), bad).toBe(false);
    }
  });
});

describe("displayAddress", () => {
  it("shows a LINE user id with its capital U and leaves other addresses alone", () => {
    expect(displayAddress("line:ub345e5d492c48c02a3eaa279924c52d2")).toBe("line:Ub345e5d492c48c02a3eaa279924c52d2");
    expect(displayAddress("line:Ub345e5d492c48c02a3eaa279924c52d2")).toBe("line:Ub345e5d492c48c02a3eaa279924c52d2");
    for (const other of ["a@b.co", "tg:123", "line:nope"]) expect(displayAddress(other)).toBe(other);
  });
});
