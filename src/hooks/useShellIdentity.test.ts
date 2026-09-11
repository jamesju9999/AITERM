import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { Terminal } from "@xterm/xterm";
import { useShellIdentity } from "./useShellIdentity";

async function writeToTerm(term: Terminal, data: string) {
  await new Promise<void>((resolve) => term.write(data, resolve));
}

let term: Terminal;

beforeEach(() => {
  term = new Terminal({ cols: 80, rows: 24 });
});

afterEach(() => {
  term.dispose();
});

describe("useShellIdentity", () => {
  it("收到 OSC 7000 之後解析出 shell / edition / version", async () => {
    const { result } = renderHook(() => useShellIdentity(term));
    expect(result.current).toBeNull();

    await act(async () => {
      await writeToTerm(term, "\x1b]7000;shell=PowerShell;edition=Desktop;version=5.1.26100.33158\x07");
    });

    expect(result.current).toEqual({
      shell: "PowerShell",
      edition: "Desktop",
      version: "5.1.26100.33158",
    });
  });

  it("PowerShell 7 回報的 edition 是 Core", async () => {
    const { result } = renderHook(() => useShellIdentity(term));

    await act(async () => {
      await writeToTerm(term, "\x1b]7000;shell=PowerShell;edition=Core;version=7.6.6\x07");
    });

    expect(result.current?.edition).toBe("Core");
  });

  it("欄位不齊的 payload 不採用，維持 null", async () => {
    // 寧可不顯示，也不要顯示錯的——徽章的整個價值就是「它說什麼就是什麼」。
    const { result } = renderHook(() => useShellIdentity(term));

    await act(async () => {
      await writeToTerm(term, "\x1b]7000;shell=PowerShell\x07");
    });

    expect(result.current).toBeNull();
  });

  it("OSC 7000 的內容不會被當成文字印進終端機畫面", async () => {
    // 同時保護遠端觀看端：那邊沒有註冊這個 handler，走的是 xterm 的 fallback。
    const bare = new Terminal({ cols: 80, rows: 24 });
    await writeToTerm(bare, "\x1b]7000;shell=PowerShell;edition=Desktop;version=5.1\x07");
    expect(bare.buffer.active.getLine(0)?.translateToString(true)).toBe("");
    bare.dispose();
  });
});
