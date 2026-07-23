import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { findNextBlockMatch, findPreviousBlockMatch, type BlockSearchCursor } from "../lib/blockSearch";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";

// TerminalView.tsx itself pulls in @xterm/xterm, Tauri IPC (pty/ai/config/fs/vcs/...),
// and several other hooks, which makes a full render impractical to set up and brittle
// to maintain for a single wiring bug. Instead, this test extracts the exact
// useCallback+useEffect interaction shape doSearch/the "search as you type" effect use
// in TerminalView.tsx (see doSearch, blockSearchCursorRef, and the effect right below
// closeSearch there) and exercises it with the real blockSearch.ts functions.
//
// Regression covered: doSearch used to close over `blockSearchCursor` *state* and list
// it as its own useCallback dependency. Every match found called setBlockSearchCursor,
// which gave doSearch a new identity, which re-triggered the "search as you type" effect
// (dependent on `doSearch`), which called doSearch again from the new cursor — cascading
// through every remaining match in one synchronous burst instead of stopping at the
// first, and ending on "not found" once it ran off the end. The fix: doSearch reads the
// cursor from a ref (kept in sync via its own effect, same pattern as the pre-existing
// `blocksRef`) and keeps a stable `[]` dependency array, so finding a match no longer
// changes doSearch's identity and the "search as you type" effect no longer re-fires.

function makeBlock(id: string, text: string): TerminalBlock {
  return {
    id,
    command: `cmd-${id}`,
    status: "completed",
    exitCode: 0,
    startTime: 0,
    rawOutput: text,
    renderedLines: [{ spans: [{ text }] }],
  };
}

function Harness({
  blocks,
  query,
  onSearchCall,
  onResult,
}: {
  blocks: TerminalBlock[];
  query: string;
  onSearchCall: () => void;
  onResult: (info: { matchInfo: string; cursor: BlockSearchCursor | null }) => void;
}) {
  const [searchMatchInfo, setSearchMatchInfo] = useState("");
  const [blockSearchCursor, setBlockSearchCursor] = useState<BlockSearchCursor | null>(null);
  const blockSearchCursorRef = useRef(blockSearchCursor);
  useEffect(() => {
    blockSearchCursorRef.current = blockSearchCursor;
  }, [blockSearchCursor]);
  const blocksRef = useRef(blocks);
  useEffect(() => {
    blocksRef.current = blocks;
  }, [blocks]);

  const onSearchCallRef = useRef(onSearchCall);
  useEffect(() => {
    onSearchCallRef.current = onSearchCall;
  }, [onSearchCall]);

  const doSearch = useCallback((q: string, direction: "next" | "prev") => {
    onSearchCallRef.current();
    if (!q) {
      setSearchMatchInfo("");
      return;
    }
    const match =
      direction === "next"
        ? findNextBlockMatch(blocksRef.current, q, blockSearchCursorRef.current)
        : findPreviousBlockMatch(blocksRef.current, q, blockSearchCursorRef.current);
    if (match) {
      setBlockSearchCursor(match);
      setSearchMatchInfo("found");
    } else {
      setSearchMatchInfo("not found");
    }
  }, []);

  useEffect(() => {
    blockSearchCursorRef.current = null;
    // Intentionally mirrors the real "search as you type" effect in TerminalView.tsx
    // being regression-tested here.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setBlockSearchCursor(null);
    if (query) doSearch(query, "next");
  }, [query, doSearch]);

  useEffect(() => {
    onResult({ matchInfo: searchMatchInfo, cursor: blockSearchCursor });
  });

  return null;
}

describe("doSearch cascade safety (TerminalView wiring, extracted)", () => {
  it("stops at the first match instead of cascading through all matches to the last one", () => {
    const blocks = [
      makeBlock("a", "needle here"),
      makeBlock("b", "no match"),
      makeBlock("c", "needle again"),
      makeBlock("d", "needle third"),
    ];
    const calls: number[] = [];
    const results: Array<{ matchInfo: string; cursor: BlockSearchCursor | null }> = [];

    render(
      <Harness
        blocks={blocks}
        query="needle"
        onSearchCall={() => calls.push(1)}
        onResult={(r) => {
          results.push(r);
        }}
      />,
    );

    const lastResult = results[results.length - 1];

    // Buggy behavior cascaded through all 3 matches (a, c, d) plus one final failed
    // lookup, invoking doSearch 4 times and landing on "not found" at block "d".
    expect(calls.length).toBe(1);
    expect(lastResult?.matchInfo).toBe("found");
    expect(lastResult?.cursor?.blockId).toBe("a");
  });
});
