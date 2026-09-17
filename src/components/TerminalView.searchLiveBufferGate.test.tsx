import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { findNextBlockMatch, type BlockSearchCursor } from "../lib/blockSearch";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";

// Extracts the exact branching shape of doSearch in TerminalView.tsx (the `foundLive`
// check gated on `isAlternateBufferRef.current`), same rationale as
// TerminalView.searchCascade.test.tsx for not mounting the full component.
//
// Regression covered: outside alternate-buffer mode, the raw xterm.js instance is kept
// mounted at opacity:0 (see the `aiterm-live-frame` comment in TerminalView.tsx) — the
// block list is the only thing the user actually sees. doSearch used to call
// addon.findNext() unconditionally; on Windows, finalizeBlock deliberately skips
// clearing that hidden buffer (ConPTY row-desync workaround), so it retains the whole
// session's scrollback and findNext kept matching there first. That reported "found"
// and decorated a buffer nobody can see, while never touching blockSearchCursor — so
// the visible card list never got its `highlightQuery` and nothing appeared
// highlighted, even though the status said "found". macOS/Linux happened to clear that
// buffer after every block, so it was usually empty there and the bug stayed hidden.

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
  isAlternateBuffer,
  liveAddonAlwaysMatches,
  onResult,
}: {
  blocks: TerminalBlock[];
  query: string;
  isAlternateBuffer: boolean;
  liveAddonAlwaysMatches: boolean;
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
  const isAlternateBufferRef = useRef(isAlternateBuffer);
  useEffect(() => {
    isAlternateBufferRef.current = isAlternateBuffer;
  }, [isAlternateBuffer]);

  // Stand-in for searchAddonRef.current: simulates Windows' never-cleared hidden
  // xterm buffer by reporting a match for anything, regardless of what's on screen.
  const addon = { findNext: () => liveAddonAlwaysMatches };

  const doSearch = useCallback((q: string) => {
    if (!q) {
      setSearchMatchInfo("");
      return;
    }
    const foundLive = addon && isAlternateBufferRef.current ? addon.findNext() : false;
    if (foundLive) {
      setBlockSearchCursor(null);
      setSearchMatchInfo("found");
      return;
    }
    const match = findNextBlockMatch(blocksRef.current, q, blockSearchCursorRef.current);
    if (match) {
      setBlockSearchCursor(match);
      setSearchMatchInfo("found");
    } else {
      setSearchMatchInfo("not found");
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps -- mirrors TerminalView.tsx's real doSearch

  useEffect(() => {
    if (query) doSearch(query);
  }, [query, doSearch]);

  useEffect(() => {
    onResult({ matchInfo: searchMatchInfo, cursor: blockSearchCursor });
  });

  return null;
}

describe("doSearch live-buffer gate (TerminalView wiring, extracted)", () => {
  it("uses the block-level match (visible highlight) outside alternate-buffer mode, even if the hidden live buffer also reports a match", () => {
    const blocks = [makeBlock("a", "ipconfig output with an ip address")];
    const results: Array<{ matchInfo: string; cursor: BlockSearchCursor | null }> = [];

    render(
      <Harness
        blocks={blocks}
        query="ip"
        isAlternateBuffer={false}
        liveAddonAlwaysMatches={true}
        onResult={(r) => results.push(r)}
      />,
    );

    const lastResult = results[results.length - 1];
    expect(lastResult?.matchInfo).toBe("found");
    // The bug: this used to be null (foundLive short-circuited before block search
    // ever ran), so no block ever got `highlightQuery` and nothing was visibly marked.
    expect(lastResult?.cursor?.blockId).toBe("a");
  });

  it("still prefers the live buffer while an alternate-buffer program (vim/htop) is actually the visible content", () => {
    const blocks = [makeBlock("a", "ip also appears in a finished block")];
    const results: Array<{ matchInfo: string; cursor: BlockSearchCursor | null }> = [];

    render(
      <Harness
        blocks={blocks}
        query="ip"
        isAlternateBuffer={true}
        liveAddonAlwaysMatches={true}
        onResult={(r) => results.push(r)}
      />,
    );

    const lastResult = results[results.length - 1];
    expect(lastResult?.matchInfo).toBe("found");
    expect(lastResult?.cursor).toBeNull();
  });
});
