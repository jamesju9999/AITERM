import { useEffect, useState, type MouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./index.css";

const PLATFORM = navigator.platform.toLowerCase();
const IS_MAC = PLATFORM.includes("mac");
// macOS keeps the native window frame (traffic lights) via
// titleBarStyle: "overlay", so we draw NO custom buttons and NO resize borders
// there — the OS still owns them. Windows/Linux run decorations:false, so the
// app must supply the window controls and edge-resize handles itself.
const FRAMELESS = !IS_MAC;

type Dir =
  | "North" | "South" | "East" | "West"
  | "NorthEast" | "NorthWest" | "SouthEast" | "SouthWest";

/**
 * App-drawn title bar. Sits full-width at the very top of the shell.
 *
 * macOS: a transparent draggable strip that leaves room on the left for the
 * native traffic lights (kept via the overlay title-bar style).
 * Windows/Linux: draggable strip + custom minimize/maximize/close buttons, plus
 * fixed edge/corner handles that restore window resizing (decorations:false
 * removes the native resize border). Double-click-to-maximize is handled by
 * Tauri automatically on any `data-tauri-drag-region` element.
 */
export function TitleBar({ title = "AITerm" }: { title?: string }) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!FRAMELESS) return; // native frame reflects its own state on macOS
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    win.isMaximized().then(setMaximized).catch(() => {});
    win
      .onResized(() => {
        win.isMaximized().then(setMaximized).catch(() => {});
      })
      .then((u) => {
        unlisten = u;
      })
      .catch(() => {});
    return () => unlisten?.();
  }, []);

  return (
    <>
      {FRAMELESS && <ResizeBorders />}
      <div className={`aiterm-titlebar${IS_MAC ? " is-mac" : ""}`} data-tauri-drag-region>
        <span className="aiterm-titlebar__title" data-tauri-drag-region>
          {title}
        </span>
        {FRAMELESS && (
          <div className="aiterm-titlebar__controls">
            <button
              className="aiterm-titlebar__btn"
              title="Minimize"
              onClick={() => getCurrentWindow().minimize().catch(() => {})}
            >
              <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
                <rect x="0" y="4.6" width="10" height="0.9" fill="currentColor" />
              </svg>
            </button>
            <button
              className="aiterm-titlebar__btn"
              title="Maximize"
              onClick={() => getCurrentWindow().toggleMaximize().catch(() => {})}
            >
              {maximized ? (
                <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
                  <rect x="0.5" y="2.5" width="6" height="6" fill="none" stroke="currentColor" strokeWidth="0.9" />
                  <path d="M2.7 2.5 V0.7 H9.3 V7.3 H7.5" fill="none" stroke="currentColor" strokeWidth="0.9" />
                </svg>
              ) : (
                <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
                  <rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="0.9" />
                </svg>
              )}
            </button>
            <button
              className="aiterm-titlebar__btn aiterm-titlebar__btn--close"
              title="Close"
              onClick={() => getCurrentWindow().close().catch(() => {})}
            >
              <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
                <path d="M0.7 0.7 L9.3 9.3 M9.3 0.7 L0.7 9.3" stroke="currentColor" strokeWidth="0.9" />
              </svg>
            </button>
          </div>
        )}
      </div>
    </>
  );
}

function ResizeBorders() {
  const start = (dir: Dir) => (e: MouseEvent) => {
    e.preventDefault();
    getCurrentWindow().startResizeDragging(dir).catch(() => {});
  };
  return (
    <div className="aiterm-rz-layer">
      <div className="aiterm-rz aiterm-rz--n" onMouseDown={start("North")} />
      <div className="aiterm-rz aiterm-rz--s" onMouseDown={start("South")} />
      <div className="aiterm-rz aiterm-rz--e" onMouseDown={start("East")} />
      <div className="aiterm-rz aiterm-rz--w" onMouseDown={start("West")} />
      <div className="aiterm-rz aiterm-rz--ne" onMouseDown={start("NorthEast")} />
      <div className="aiterm-rz aiterm-rz--nw" onMouseDown={start("NorthWest")} />
      <div className="aiterm-rz aiterm-rz--se" onMouseDown={start("SouthEast")} />
      <div className="aiterm-rz aiterm-rz--sw" onMouseDown={start("SouthWest")} />
    </div>
  );
}
