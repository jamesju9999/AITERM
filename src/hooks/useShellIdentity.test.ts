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

  it("重送同一份身分不產生新物件——否則每跑一個指令都會讓整個分頁重繪", async () => {
    // 腳本綁在 prompt 函式裡，每次提示字元都重送一次（那是為了避開處理器
    // 註冊時序的 race，見 shell.rs 的註解）。代價是這個 hook 必須自己擋掉
    // 內容相同的更新。
    const { result } = renderHook(() => useShellIdentity(term));
    const payload = "\x1b]7000;shell=PowerShell;edition=Desktop;version=5.1.26100.33158\x07";

    await act(async () => {
      await writeToTerm(term, payload);
    });
    const first = result.current;
    expect(first).not.toBeNull();

    await act(async () => {
      await writeToTerm(term, payload);
    });
    expect(result.current).toBe(first);
  });

  it("回報內容真的變了才換成新物件", async () => {
    const { result } = renderHook(() => useShellIdentity(term));

    await act(async () => {
      await writeToTerm(term, "\x1b]7000;shell=PowerShell;edition=Desktop;version=5.1.1\x07");
    });
    const first = result.current;

    await act(async () => {
      await writeToTerm(term, "\x1b]7000;shell=PowerShell;edition=Core;version=7.6.6\x07");
    });
    expect(result.current).not.toBe(first);
    expect(result.current?.edition).toBe("Core");
  });

  it("OSC 7000 的內容不會被當成文字印進終端機畫面", async () => {
    // 同時保護遠端觀看端：那邊沒有註冊這個 handler，走的是 xterm 的 fallback。
    const bare = new Terminal({ cols: 80, rows: 24 });
    await writeToTerm(bare, "\x1b]7000;shell=PowerShell;edition=Desktop;version=5.1\x07");
    expect(bare.buffer.active.getLine(0)?.translateToString(true)).toBe("");
    bare.dispose();
  });
});
