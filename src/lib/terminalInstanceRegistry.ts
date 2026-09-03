import type { Terminal } from "@xterm/xterm";

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
