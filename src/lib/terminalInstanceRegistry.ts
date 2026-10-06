import type { Terminal } from "@xterm/xterm";
import { writePty } from "../ipc/pty";

/** Structural subset of @xterm/addon-serialize's SerializeAddon — avoids a
 * dependency on that package from this module (it's wired in by
 * TerminalView; this registry only needs the one method it calls). */
interface SerializeLike {
  serialize(options?: { scrollback?: number }): string;
}

interface RegistryEntry {
  term: Terminal;
  serializeAddon: SerializeLike;
}

const registry = new Map<string, RegistryEntry>();

/** Registers the live terminal + serialize addon for a tab/session id.
 * Called by TerminalView on mount; a second call for the same id replaces
 * the previous entry (e.g. if a tab's terminal were ever recreated without
 * an unregister in between — belt and suspenders, not expected in normal
 * operation). */
export function registerTerminal(id: string, term: Terminal, serializeAddon: SerializeLike): void {
  registry.set(id, { term, serializeAddon });
}

/** Removes the registry entry for a tab/session id. Called by TerminalView
 * on unmount. Safe to call for an id that isn't registered (no-op). */
export function unregisterTerminal(id: string): void {
  registry.delete(id);
}

/** Returns the current serialized screen-buffer text for a tab/session id,
 * or null if that tab isn't live (never registered, or already
 * unregistered — e.g. the tab was closed). Callers use null to mean "fall
 * back to the raw transcript already on disk", not an error.
 *
 * Passes `scrollback: 0` deliberately: with no options, serialize() dumps
 * the entire scrollback buffer, not just what's currently on screen. A TUI
 * app like Claude Code redraws by scrolling through many intermediate
 * frames (spinner animation, in-progress table drafts, repeated prompts) —
 * without this, all of that history comes back concatenated together,
 * which is just as unreadable as the raw capture this was meant to
 * replace. `scrollback: 0` limits it to the current viewport, i.e. exactly
 * what a human looking at the terminal right now would see. */
export function serializeTerminal(id: string): string | null {
  const entry = registry.get(id);
  if (!entry) return null;
  return entry.serializeAddon.serialize({ scrollback: 0 });
}

/** 把文字填進該終端機目前的輸入位置，**不送 Enter**。
 *
 * 程式有開 bracketed paste（Claude Code 會開）就包成貼上序列，讓它當成一整塊
 * 貼上而不是逐字鍵入。沒開就把換行壓成空白——否則換行會被 shell 當成 Enter，
 * 建議的文字就直接被執行了。兩條路都先剝掉文字裡的控制序列，避免內嵌的
 * `ESC[201~` 提前結束貼上、後面的內容變成真的按鍵。
 *
 * 回傳 false 表示這個 id 沒有活著的終端機，什麼都沒送。 */
export async function fillTerminalInput(id: string, text: string): Promise<boolean> {
  const entry = registry.get(id);
  if (!entry) return false;
  // eslint-disable-next-line no-control-regex -- stripping real ESC/control bytes is the point
  const clean = text.replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "").replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, "");
  const payload = entry.term.modes?.bracketedPasteMode
    ? `\x1b[200~${clean.replace(/\r\n?/g, "\n")}\x1b[201~`
    : clean.replace(/\s*[\r\n]+\s*/g, " ");
  await writePty(id, payload);
  return true;
}
