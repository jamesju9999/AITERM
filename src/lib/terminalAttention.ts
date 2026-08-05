/** 終端機分頁發生的、值得使用者注意的事件。 */
export type AttentionKind = "waiting" | "done" | "failed";

export interface AttentionInput {
  /** 這個事件來自使用者當前正在看的那個分頁嗎？ */
  isActiveTab: boolean;
  /** app 視窗此刻有沒有 focus？ */
  windowFocused: boolean;
  kind: AttentionKind;
}

export interface AttentionRouting {
  /** 要設在分頁上的提示點；null 表示不設。 */
  badge: AttentionKind | null;
  /** 要不要發桌面通知。 */
  notify: boolean;
}

/**
 * 把一個 attention 事件拆成兩個「互相獨立」的決定。
 *
 * 這兩個條件不能合併：app 失焦時，即使事件來自 active 分頁也要發通知——
 * 使用者人不在 app 前面，「它是 active 分頁」不代表有人在看。反過來，
 * 提示點對 active 分頁沒有意義，因為使用者一切回來就會看到終端機內容。
 */
export function routeAttention({ isActiveTab, windowFocused, kind }: AttentionInput): AttentionRouting {
  return {
    badge: isActiveTab ? null : kind,
    notify: !windowFocused && (kind === "waiting" || kind === "failed"),
  };
}

/** OSC 133 D 帶回來的 exit code 對應到哪一種 attention。 */
export function attentionForExitCode(exitCode: number): AttentionKind {
  return exitCode === 0 ? "done" : "failed";
}
