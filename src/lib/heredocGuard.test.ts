import { describe, expect, it } from "vitest";
import { repairUnterminatedHeredocs } from "./heredocGuard";

describe("repairUnterminatedHeredocs", () => {
  it("leaves a command with no heredoc alone", () => {
    expect(repairUnterminatedHeredocs("ls -la")).toBe("ls -la");
  });

  it("leaves a correctly terminated heredoc alone", () => {
    const cmd = "cat > f <<'EOF'\nbody\nEOF";
    expect(repairUnterminatedHeredocs(cmd)).toBe(cmd);
  });

  // 這是實機卡死的形狀：結束標記跟後面的指令擠在同一行，shell 永遠等不到
  // 單獨一行的 EOF，就停在 heredoc> 不動，而指令沒結束＝OSC 133 D 不會發出
  // ＝agent 的區塊永遠是 running＝整個迴圈就在那邊等。
  it("appends the terminator when it shares a line with another command", () => {
    const cmd = "cat > f <<'EOF'\nbody\nEOF echo done";
    expect(repairUnterminatedHeredocs(cmd)).toBe(`${cmd}\nEOF`);
  });

  it("appends the terminator when the whole command was flattened to one line", () => {
    const cmd = "cat > f <<'EOF' body EOF echo done";
    expect(repairUnterminatedHeredocs(cmd)).toBe(`${cmd}\nEOF`);
  });

  it("handles unquoted and double-quoted delimiters", () => {
    expect(repairUnterminatedHeredocs("cat <<EOF\nx")).toBe("cat <<EOF\nx\nEOF");
    expect(repairUnterminatedHeredocs('cat <<"END"\nx')).toBe('cat <<"END"\nx\nEND');
  });

  // <<- 容許結束標記前面有 tab，所以有 tab 的那一行也算數。
  it("accepts a tab-indented terminator for <<-", () => {
    const cmd = "cat <<-EOF\nbody\n\tEOF";
    expect(repairUnterminatedHeredocs(cmd)).toBe(cmd);
  });

  it("repairs several heredocs in the order they were opened", () => {
    const cmd = "cmd <<A <<B\nstuff";
    expect(repairUnterminatedHeredocs(cmd)).toBe("cmd <<A <<B\nstuff\nA\nB");
  });

  it("only appends the ones that are actually missing", () => {
    const cmd = "cmd <<A <<B\nstuff\nA";
    expect(repairUnterminatedHeredocs(cmd)).toBe(`${cmd}\nB`);
  });

  // <<< 是 herestring，不是 heredoc——補結束標記會把指令弄壞。
  it("ignores a herestring", () => {
    const cmd = "grep x <<< 'some text'";
    expect(repairUnterminatedHeredocs(cmd)).toBe(cmd);
  });

  it("ignores a left shift in arithmetic", () => {
    const cmd = "echo $((1 << 3))";
    expect(repairUnterminatedHeredocs(cmd)).toBe(cmd);
  });
});
