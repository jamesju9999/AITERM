import { useCallback, useEffect, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { cancelRemoteLogin, openLive, type LiveClientMsg, type LiveConnection } from "../../ipc/serverLive";

export interface LiveTarget {
  kind: "agent" | "login";
  systemId: string;
  systemName: string;
  cardId?: string;
}

/** 滑鼠移動訊息最多多久送一次（毫秒），免得塞滿連線。 */
const MOVE_THROTTLE_MS = 40;

const BUTTONS = ["left", "middle", "right"] as const;

/** 事件在影像上的位置，換成 0～1 的正規化座標（影像之外夾回邊界）。 */
export function normalized(clientX: number, clientY: number, rect: { left: number; top: number; width: number; height: number }) {
  const clamp = (v: number) => (Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0);
  return { x: clamp((clientX - rect.left) / (rect.width || 1)), y: clamp((clientY - rect.top) / (rect.height || 1)) };
}

/** 修飾鍵位元（CDP 的定義）：Alt=1、Ctrl=2、Meta=4、Shift=8。 */
export const modifiers = (e: { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }) => (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);

/**
 * 即時看（並在管理員時操作）server 上的瀏覽器：Agent 執行中的畫面（要先「接手」才能操作），或遠端登入。
 * 成員只能看。遠端登入時明講：輸入的帳號密碼會經過 server 的記憶體。
 */
export function LiveViewDialog({ target, onClose, onLoginDone }: { target: LiveTarget; onClose: () => void; onLoginDone?: () => void }) {
  const { t } = useLocale();
  const [frame, setFrame] = useState<string | null>(null);
  const [meta, setMeta] = useState<{ manual: boolean; url: string; canOperate: boolean; mode: "agent" | "login" } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ended, setEnded] = useState<string | null>(null);
  const conn = useRef<LiveConnection | null>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const lastMove = useRef(0);
  const composing = useRef(false);

  useEffect(() => {
    let alive = true;
    let opened: LiveConnection | null = null;
    openLive(
      { kind: target.kind, systemId: target.systemId, cardId: target.cardId },
      (m) => {
        if (!alive) return;
        if (m.t === "frame") setFrame(`data:image/jpeg;base64,${m.data}`);
        else if (m.t === "state") setMeta({ manual: m.manual, url: m.url, canOperate: m.can_operate, mode: m.mode });
        else if (m.t === "error") setError(m.message);
        else if (m.t === "closed") {
          setEnded(m.why);
          if (target.kind === "login" && m.why.includes("session 已保存")) onLoginDone?.();
        }
      },
      () => alive && setEnded((cur) => cur ?? t.ac_live_ended),
    )
      .then((c) => {
        if (!alive) c.close();
        else {
          opened = c;
          conn.current = c;
        }
      })
      .catch((e) => alive && setEnded(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e)));
    return () => {
      alive = false;
      opened?.close();
      conn.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target.kind, target.systemId, target.cardId]);

  // 錯誤訊息顯示幾秒就收掉。
  useEffect(() => {
    if (!error) return;
    const id = setTimeout(() => setError(null), 4000);
    return () => clearTimeout(id);
  }, [error]);

  const send = useCallback((m: LiveClientMsg) => conn.current?.send(m), []);
  const isAgent = (meta?.mode ?? target.kind) === "agent";
  const canInput = !!meta?.canOperate && (!isAgent || meta.manual) && !ended;

  const pos = (e: { clientX: number; clientY: number }) => (imgRef.current ? normalized(e.clientX, e.clientY, imgRef.current.getBoundingClientRect()) : { x: 0, y: 0 });

  // 滾輪要 preventDefault（不讓整個畫面跟著捲），React 的 onWheel 是被動的，所以直接掛原生事件。
  useEffect(() => {
    const el = imgRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!canInput) return;
      e.preventDefault();
      const { x, y } = normalized(e.clientX, e.clientY, el.getBoundingClientRect());
      send({ t: "mouse", type: "wheel", x, y, dx: e.deltaX, dy: e.deltaY });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [canInput, send, frame === null]);

  const onKey = (type: "down" | "up") => (e: React.KeyboardEvent) => {
    if (!canInput || composing.current || e.nativeEvent.isComposing || e.keyCode === 229) return;
    // 貼上交給 onPaste（它拿得到剪貼簿文字）；其餘的鍵都轉過去，不讓視窗自己處理。
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "v") return;
    e.preventDefault();
    const text = e.key === "Enter" ? "\r" : e.key.length === 1 && !e.ctrlKey && !e.metaKey ? e.key : undefined;
    send({ t: "key", type, key: e.key, code: e.code, text: type === "down" ? text : undefined, vk: e.keyCode, mods: modifiers(e) });
  };

  const title = t.ac_live_title(target.systemName);

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-live-dialog" role="dialog" aria-label={title} data-testid="live-view-dialog">
        <h3 className="task-dialog-title">{title}</h3>

        {target.kind === "login" && <p className="ac-dialog-desc ac-bad-text">{t.ac_live_login_warning}</p>}
        <div className="ac-live-status" data-testid="live-status">
          {ended ? ended : !meta ? t.ac_live_connecting : !meta.canOperate ? t.ac_live_readonly : isAgent ? (meta.manual ? t.ac_live_manual : t.ac_live_agent_running) : t.ac_live_login_hint}
          {meta?.url && !ended ? <span className="ac-db-meta"> · {meta.url}</span> : null}
        </div>

        <div
          className="ac-live-screen"
          tabIndex={0}
          onKeyDown={onKey("down")}
          onKeyUp={onKey("up")}
          onPaste={(e) => {
            if (!canInput) return;
            e.preventDefault();
            const text = e.clipboardData.getData("text");
            if (text) send({ t: "text", text });
          }}
          onCompositionStart={() => (composing.current = true)}
          onCompositionEnd={(e) => {
            composing.current = false;
            if (canInput && e.data) send({ t: "text", text: e.data });
          }}
          onContextMenu={(e) => e.preventDefault()}
        >
          {frame ? (
            <img
              ref={imgRef}
              src={frame}
              alt={title}
              draggable={false}
              className={canInput ? "ac-live-img ac-live-img--active" : "ac-live-img"}
              data-testid="live-frame"
              onPointerMove={(e) => {
                if (!canInput) return;
                const now = Date.now();
                if (now - lastMove.current < MOVE_THROTTLE_MS) return;
                lastMove.current = now;
                send({ t: "mouse", type: "move", ...pos(e) });
              }}
              onPointerDown={(e) => {
                if (!canInput) return;
                (e.currentTarget.parentElement as HTMLElement | null)?.focus();
                send({ t: "mouse", type: "down", ...pos(e), button: BUTTONS[e.button] ?? "left" });
              }}
              onPointerUp={(e) => canInput && send({ t: "mouse", type: "up", ...pos(e), button: BUTTONS[e.button] ?? "left" })}
            />
          ) : (
            <div className="ac-live-placeholder">{ended ?? t.ac_live_connecting}</div>
          )}
        </div>
        {error && (
          <p className="ac-bad-text" role="alert">
            {error}
          </p>
        )}

        <div className="task-dialog-actions">
          {meta?.canOperate && isAgent && !ended && (
            <button className="aiterm-btn aiterm-btn--primary" onClick={() => send({ t: "takeover", on: !meta.manual })}>
              {meta.manual ? t.ac_live_handback : t.ac_live_takeover}
            </button>
          )}
          {meta?.canOperate && !isAgent && !ended && (
            <>
              <button className="aiterm-btn aiterm-btn--secondary" onClick={() => send({ t: "back" })}>
                {t.ac_live_back}
              </button>
              <button className="aiterm-btn aiterm-btn--primary" onClick={() => send({ t: "done" })}>
                {t.ac_live_login_done}
              </button>
              <button
                className="aiterm-btn aiterm-btn--ghost"
                onClick={() => {
                  void cancelRemoteLogin(target.systemId).catch(() => {});
                  onClose();
                }}
              >
                {t.ac_live_login_cancel}
              </button>
            </>
          )}
          <span className="ac-spacer" />
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.ac_live_close}
          </button>
        </div>
      </div>
    </div>
  );
}
