import type { TerminalBlock } from "../hooks/useTerminalBlocks";

export interface BlockSearchCursor {
  blockId: string;
  offset: number;
}

export function blockPlainText(block: TerminalBlock): string {
  return (block.renderedLines ?? []).map((line) => line.spans.map((s) => s.text).join("")).join("\n");
}

export function findNextBlockMatch(
  blocks: TerminalBlock[],
  query: string,
  cursor: BlockSearchCursor | null,
): BlockSearchCursor | null {
  if (!query) return null;
  const q = query.toLowerCase();
  const startIndex = cursor ? blocks.findIndex((b) => b.id === cursor.blockId) : -1;

  for (let i = Math.max(startIndex, 0); i < blocks.length; i++) {
    const block = blocks[i];
    const text = blockPlainText(block).toLowerCase();
    const searchFrom = i === startIndex ? cursor!.offset + 1 : 0;
    const idx = text.indexOf(q, searchFrom);
    if (idx !== -1) {
      return { blockId: block.id, offset: idx };
    }
  }
  return null;
}

export function findPreviousBlockMatch(
  blocks: TerminalBlock[],
  query: string,
  cursor: BlockSearchCursor | null,
): BlockSearchCursor | null {
  if (!query) return null;
  const q = query.toLowerCase();
  const startIndex = cursor ? blocks.findIndex((b) => b.id === cursor.blockId) : blocks.length;

  for (let i = Math.min(startIndex, blocks.length - 1); i >= 0; i--) {
    const block = blocks[i];
    const text = blockPlainText(block).toLowerCase();
    const searchUpTo = i === startIndex ? cursor!.offset - 1 : text.length;
    if (searchUpTo < 0) continue;
    const idx = text.lastIndexOf(q, searchUpTo);
    if (idx !== -1) {
      return { blockId: block.id, offset: idx };
    }
  }
  return null;
}
