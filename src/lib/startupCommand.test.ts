import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createStartupInjector } from "./startupCommand";

const PROMPT = "\x1b]133;A\x07";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("createStartupInjector", () => {
  it("does not write on ConPTY-style startup noise, only after a prompt and a quiet moment", () => {
    const write = vi.fn();
    const inj = createStartupInjector({ command: "ls", write });
    inj.feed("\x1b[?9001h\x1b[?1004h"); // 不是 shell 的輸出
    vi.advanceTimersByTime(5_000);
    expect(write).not.toHaveBeenCalled();
    inj.feed(`${PROMPT}user@host % `);
    vi.advanceTimersByTime(249);
    expect(write).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(write).toHaveBeenCalledTimes(1);
    expect(write).toHaveBeenCalledWith("ls\r");
  });

  it("keeps waiting while output is still arriving after the prompt", () => {
    const write = vi.fn();
    const inj = createStartupInjector({ command: "ls", write });
    inj.feed(PROMPT);
    vi.advanceTimersByTime(200);
    inj.feed("more output"); // 重新計時
    vi.advanceTimersByTime(200);
    expect(write).not.toHaveBeenCalled();
    vi.advanceTimersByTime(50);
    expect(write).toHaveBeenCalledTimes(1);
  });

  it("falls back to sending anyway when no prompt marker ever shows up", () => {
    const write = vi.fn();
    createStartupInjector({ command: "ls", write });
    vi.advanceTimersByTime(9_999);
    expect(write).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(write).toHaveBeenCalledWith("ls\r");
  });

  it("sends at most once even if more prompts follow", () => {
    const write = vi.fn();
    const inj = createStartupInjector({ command: "ls", write });
    inj.feed(PROMPT);
    vi.advanceTimersByTime(250);
    inj.feed(PROMPT);
    vi.advanceTimersByTime(20_000);
    expect(write).toHaveBeenCalledTimes(1);
  });

  it("dispose cancels everything", () => {
    const write = vi.fn();
    const inj = createStartupInjector({ command: "ls", write });
    inj.feed(PROMPT);
    inj.dispose();
    vi.advanceTimersByTime(20_000);
    expect(write).not.toHaveBeenCalled();
  });

  it("ignores output that arrives after dispose", () => {
    const write = vi.fn();
    const inj = createStartupInjector({ command: "ls", write });
    inj.dispose();
    inj.feed(PROMPT); // 元件已卸載後才到的 chunk
    vi.advanceTimersByTime(20_000);
    expect(write).not.toHaveBeenCalled();
  });

  it("does not treat the bare text 133;A (no ESC) as a prompt marker", () => {
    const write = vi.fn();
    const inj = createStartupInjector({ command: "ls", write });
    inj.feed("]133;A"); // 例如 cat 到一份剛好含這串字的檔案
    vi.advanceTimersByTime(9_999);
    expect(write).not.toHaveBeenCalled();
  });
});
