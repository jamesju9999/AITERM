import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import type { AttentionKind } from "../lib/terminalAttention";

// TerminalView.tsx itself pulls in @xterm/xterm, Tauri IPC (pty/ai/config/fs/vcs/...),
// and several other hooks, which makes a full render impractical to set up and brittle
// to maintain for a single wiring bug (see TerminalView.searchCascade.test.tsx for the
// established precedent of extracting instead of mounting). This test extracts the exact
// onAttentionRef/emitAttention/handleCommandSettled/bell-effect shape TerminalView.tsx
// uses (see onAttentionRef, emitAttention, handleCommandSettled, and the term.onBell
// effect there), wired to a real @xterm/xterm Terminal instance (same technique as
// useTerminalBlocks.test.ts's writeToTerm), and exercises it directly.
//
// Regression covered: TerminalApp passes onAttention as an inline arrow function, so its
// identity changes on every TerminalApp render. If emitAttention closed over `onAttention`
// directly instead of bridging through a ref, a bell ringing after TerminalApp re-rendered
// (but before this component's own effect re-ran) could still fire the *previous* render's
// callback — or, depending on effect timing, silently drop the call. Bridging through a ref
// keeps emitAttention itself referentially stable while always reading the latest callback.

async function writeToTerm(term: Terminal, data: string) {
  await new Promise<void>((resolve) => term.write(data, resolve));
}

function Harness({
  onAttention,
  onReady,
}: {
  onAttention: (kind: AttentionKind) => void;
  onReady: (api: { term: Terminal; handleCommandSettled: (exitCode: number) => void }) => void;
}) {
  const [term] = useState(() => new Terminal({ cols: 80, rows: 24 }));

  const onAttentionRef = useRef(onAttention);
  useEffect(() => { onAttentionRef.current = onAttention; }, [onAttention]);

  const emitAttention = useCallback((kind: AttentionKind) => {
    onAttentionRef.current?.(kind);
  }, []);

  const handleCommandSettled = useCallback((exitCode: number) => {
    emitAttention(exitCode === 0 ? "done" : "failed");
  }, [emitAttention]);

  useEffect(() => {
    const disposable = term.onBell(() => emitAttention("waiting"));
    return () => disposable.dispose();
  }, [term, emitAttention]);

  useEffect(() => {
    onReady({ term, handleCommandSettled });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return null;
}

describe("TerminalView attention wiring (extracted)", () => {
  it("fires onAttention('waiting') when the terminal bell rings", async () => {
    const onAttention = vi.fn();
    let api: { term: Terminal; handleCommandSettled: (exitCode: number) => void } | null = null;

    render(<Harness onAttention={onAttention} onReady={(a) => { api = a; }} />);
    expect(api).not.toBeNull();

    await writeToTerm(api!.term, "\x07");

    expect(onAttention).toHaveBeenCalledWith("waiting");
    api!.term.dispose();
  });

  it("reads the latest onAttention from the ref instead of a stale closure", async () => {
    const first = vi.fn();
    const second = vi.fn();
    let api: { term: Terminal; handleCommandSettled: (exitCode: number) => void } | null = null;

    const { rerender } = render(
      <Harness onAttention={first} onReady={(a) => { api = a; }} />,
    );
    // A new inline arrow every render, exactly like TerminalApp -> TerminalView.
    rerender(<Harness onAttention={second} onReady={(a) => { api = a; }} />);

    await writeToTerm(api!.term, "\x07");

    expect(second).toHaveBeenCalledWith("waiting");
    expect(first).not.toHaveBeenCalled();
    api!.term.dispose();
  });

  it("maps a zero exit code to done and a non-zero exit code to failed", () => {
    const onAttention = vi.fn();
    let api: { term: Terminal; handleCommandSettled: (exitCode: number) => void } | null = null;

    render(<Harness onAttention={onAttention} onReady={(a) => { api = a; }} />);

    api!.handleCommandSettled(0);
    api!.handleCommandSettled(1);

    expect(onAttention).toHaveBeenNthCalledWith(1, "done");
    expect(onAttention).toHaveBeenNthCalledWith(2, "failed");
    api!.term.dispose();
  });
});
