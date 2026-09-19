export interface StartupInjector {
  /** 把 PTY 輸出餵進來。 */
  feed(chunk: string): void;
  dispose(): void;
}

interface Options {
  command: string;
  write: (data: string) => void;
  /** 看到提示字元之後，安靜多久才送。 */
  quietMs?: number;
  /** 一直看不到提示字元標記時的保底逾時。 */
  fallbackMs?: number;
}

const PROMPT_START = "\x1b]133;A";

/**
 * 在 shell 就緒後把 `command` 送進 PTY，只送一次。
 *
 * 「收到第一個輸出 chunk」不算就緒（Windows 的第一個 chunk 是 ConPTY 自己的
 * 序列，shell 還沒啟動，此時寫入的輸入會被丟掉）。可靠訊號是 shell 自己發的
 * OSC 133 A，再加上輸出安靜下來。
 */
export function createStartupInjector({
  command,
  write,
  quietMs = 250,
  fallbackMs = 10_000,
}: Options): StartupInjector {
  let sawPrompt = false;
  let finished = false;
  let quietTimer: ReturnType<typeof setTimeout> | undefined;

  const finish = () => {
    clearTimeout(quietTimer);
    clearTimeout(fallbackTimer);
    finished = true;
  };
  const send = () => {
    if (finished) return;
    finish();
    write(`${command}\r`);
  };
  const fallbackTimer = setTimeout(send, fallbackMs);

  return {
    feed(chunk) {
      if (finished) return;
      // 已知限制：標記若剛好被切在兩個 chunk 之間不會被偵測到，此時由保底逾時接手。
      if (chunk.includes(PROMPT_START)) sawPrompt = true;
      if (sawPrompt) {
        clearTimeout(quietTimer);
        quietTimer = setTimeout(send, quietMs);
      }
    },
    dispose: finish,
  };
}
