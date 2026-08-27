# 遠端指令文字還原與卡片化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓遠端觀看者透過遠端終端機直接輸入的指令，在本機端也能被追蹤成完整的 Block Card（文字跟使用者實際輸入的一致），而不是永遠卡在高度受限、無法變成卡片的 live frame 小窗格裡。

**Architecture:** 新增 OSC 133 `B` 標記（prompt 結束、即將輸入），接在提示字元文字本身的尾端（zsh/bash 接 `PS1`、PowerShell 接 `prompt` 函式回傳值），保證在提示字元真正畫出來之後才出現。`useTerminalBlocks.ts` 收到 `B` 時記錄游標絕對位置，收到 `C` 時（若沒有本機追蹤區塊）用這個位置到目前游標位置之間的畫面內容，截出使用者實際打的指令文字，餵給既有的 `beginTrackedBlock`，讓遠端指令完全匯入本機既有的卡片資料流。還原失敗時退回既有的 `onUntrackedCommandBoundary` 高度保底機制。

**Tech Stack:** TypeScript / React 19 / xterm.js（`useTerminalBlocks.ts`、`TerminalView.tsx`）、Rust（`src-tauri/src/pty/shell.rs`，zsh/bash/PowerShell 整合腳本）、Vitest、Rust 內建測試。

**參考設計文件：** `docs/superpowers/specs/2026-08-27-remote-command-text-recovery-design.md`

---

### Task 1: zsh 整合腳本新增 `B` 標記

**Files:**
- Modify: `src-tauri/src/pty/shell.rs`（`inject_shell_integration` 裡 zsh 分支的 `zshrc_content`，約第 175-218 行）
- Test: `src-tauri/src/pty/shell.rs`（檔案底部 `#[cfg(test)] mod tests`）

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/pty/shell.rs` 底部的 `mod tests { ... }` 區塊內（`which_on_path_finds_nothing_for_garbage_name` 測試之後），新增：

```rust
    #[cfg(not(windows))]
    #[test]
    fn zsh_integration_appends_b_marker_to_ps1_in_precmd() {
        let _ = inject_shell_integration(PathBuf::from("/bin/zsh"));
        let content = std::fs::read_to_string(
            std::env::temp_dir().join("aiterm_zsh").join(".zshrc"),
        )
        .expect("zshrc should have been written");

        assert!(
            content.contains(r#"printf '\x1b]133;A\x07'"#),
            "expected the existing A marker printf to still be present"
        );
        assert!(
            content.contains(r#"local b_marker=$'%{\e]133;B\a%}'"#),
            "expected the precmd hook to compute a B marker to append to PS1"
        );
        assert!(
            content.contains(r#"if [[ "$PS1" != *"$b_marker"* ]]; then"#),
            "expected a guard against appending the B marker more than once"
        );
        assert!(
            content.contains(r#"PS1="${PS1}${b_marker}""#),
            "expected PS1 to be extended with the B marker"
        );
    }
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `cd src-tauri && cargo test --lib zsh_integration_appends_b_marker_to_ps1_in_precmd`
Expected: FAIL — 斷言 `b_marker` 相關內容失敗（目前的 `zshrc_content` 完全沒有 `B` 標記）。

- [ ] **Step 3: 在 `__aiterm_precmd` 裡新增 `B` 標記邏輯**

找到 `src-tauri/src/pty/shell.rs` 裡的這段（zsh 分支的 `zshrc_content`）：

```rust
__aiterm_precmd() {
  local ec=$?
  if [[ $__aiterm_cmd_running -eq 1 ]]; then
    __aiterm_cmd_running=0
    printf '\x1b]133;D;%s\x07' "$ec"
  fi
  printf '\x1b]133;A\x07'
}
```

改成：

```rust
__aiterm_precmd() {
  local ec=$?
  if [[ $__aiterm_cmd_running -eq 1 ]]; then
    __aiterm_cmd_running=0
    printf '\x1b]133;D;%s\x07' "$ec"
  fi
  printf '\x1b]133;A\x07'
  local b_marker=$'%{\e]133;B\a%}'
  if [[ "$PS1" != *"$b_marker"* ]]; then
    PS1="${PS1}${b_marker}"
  fi
}
```

（`%{...%}` 是 zsh 提示字元語法裡的零寬度標記，避免這段不可見位元組干擾游標定位/自動換行的欄位計算。`if [[ "$PS1" != *"$b_marker"* ]]` 這個檢查避免每次 precmd 都無限疊加——若框架每次都重新生成一份不含標記的 `PS1`，每次都會補一次；若 `PS1` 是靜態的，只會補一次。）

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test --lib zsh_integration_appends_b_marker_to_ps1_in_precmd`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src-tauri/src/pty/shell.rs
git commit -m "$(cat <<'EOF'
feat(shell): zsh 整合腳本新增 OSC 133 B 標記

接在 PS1 尾端，保證在提示字元文字真正畫出來之後才出現——跟現有的
A 標記（precmd hook 裡印出來，提示字元文字之前就已執行完）不同，
B 提供一個可以信賴的「輸入從這裡開始」座標，供之後遠端指令文字
還原機制使用。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: bash 整合腳本新增 `B` 標記

**這個 Task 的設計跟 zsh 版（Task 1）不同，請仔細看 Step 3 的說明再動手。** Task 1
完成後的程式碼品質審查發現：bash 沒有 zsh `add-zsh-hook` 那種「保證排在框架自己的
hook 之後執行」的機制——若沿用 zsh 版「直接在 `__aiterm_precmd` 裡接 `B` 標記」的
寫法，starship 這類每次都整段重新生成 `PS1` 的框架會把我們剛接上去的 `B` 標記蓋掉。
因此 bash 版要把「接 `B` 標記」拆成一個獨立函式，附加在 `PROMPT_COMMAND` 鏈的
**最後面**，`__aiterm_precmd` 本身（`D`/`A` 標記、`$?` 擷取）維持在最前面、完全不動。

**Files:**
- Modify: `src-tauri/src/pty/shell.rs`（`inject_shell_integration` 裡 bash 分支的 `bashrc_content`，約第 229-259 行）
- Test: `src-tauri/src/pty/shell.rs`

- [ ] **Step 1: 寫失敗的測試**

在同一個 `mod tests { ... }` 區塊內，緊接 Task 1 新增的測試之後：

```rust
    #[cfg(not(windows))]
    #[test]
    fn bash_integration_appends_b_marker_to_ps1_after_prompt_command_chain() {
        let spec = inject_shell_integration(PathBuf::from("/bin/bash"));
        let rcfile_idx = spec
            .args
            .iter()
            .position(|a| a == "--rcfile")
            .expect("bash spec should pass --rcfile");
        let rcfile = PathBuf::from(&spec.args[rcfile_idx + 1]);
        let content = std::fs::read_to_string(rcfile).expect("bashrc should have been written");

        assert!(
            content.contains(r#"printf '\x1b]133;A\x07'"#),
            "expected the existing A marker printf to still be present"
        );
        assert!(
            content.contains("__aiterm_append_b_marker() {"),
            "expected a dedicated function for appending the B marker, kept separate from __aiterm_precmd"
        );
        assert!(
            content.contains(r#"local b_marker=$'\[\e]133;B\a\]'"#),
            "expected the B marker append function to compute a B marker"
        );
        assert!(
            content.contains(r#"if [[ "$PS1" != *"$b_marker"* ]]; then"#),
            "expected a guard against appending the B marker more than once"
        );
        assert!(
            content.contains(r#"PS1="${PS1}${b_marker}""#),
            "expected PS1 to be extended with the B marker"
        );
        assert!(
            content.contains(
                r#"PROMPT_COMMAND="__aiterm_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND};__aiterm_append_b_marker""#
            ),
            "expected __aiterm_append_b_marker to run LAST in PROMPT_COMMAND — after any \
             framework's own PROMPT_COMMAND entries (which __aiterm_precmd is prepended \
             before, to capture $? correctly) have already finalized PS1 for this cycle, \
             so the B marker survives even if a framework fully reassigns PS1 rather than \
             appending to it"
        );
    }
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `cd src-tauri && cargo test --lib bash_integration_appends_b_marker_to_ps1_after_prompt_command_chain`
Expected: FAIL — `__aiterm_append_b_marker`/`b_marker` 相關斷言失敗（目前完全沒有這個函式）。

- [ ] **Step 3: 新增獨立的 `__aiterm_append_b_marker` 函式，附加在 `PROMPT_COMMAND` 鏈尾**

找到 `src-tauri/src/pty/shell.rs` 裡 bash 分支的這段：

```rust
__aiterm_precmd() {
  local ec=$?
  if [[ $__aiterm_cmd_running -eq 1 ]]; then
    __aiterm_cmd_running=0
    printf '\x1b]133;D;%s\x07' "$ec"
  fi
  printf '\x1b]133;A\x07'
}
PROMPT_COMMAND="__aiterm_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
```

改成：

```rust
__aiterm_precmd() {
  local ec=$?
  if [[ $__aiterm_cmd_running -eq 1 ]]; then
    __aiterm_cmd_running=0
    printf '\x1b]133;D;%s\x07' "$ec"
  fi
  printf '\x1b]133;A\x07'
}

__aiterm_append_b_marker() {
  local b_marker=$'\[\e]133;B\a\]'
  if [[ "$PS1" != *"$b_marker"* ]]; then
    PS1="${PS1}${b_marker}"
  fi
}

PROMPT_COMMAND="__aiterm_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND};__aiterm_append_b_marker"
```

（**`__aiterm_precmd` 保持在最前面，這是刻意的、不能動**：`local ec=$?` 必須在任何
其他指令執行之前讀走 exit code，晚一步 `$?` 就被覆蓋了。`\[...\]` 是 bash/readline
提示字元語法裡的零寬度標記寫法，既有的顏色控制碼在使用者自己的 `PS1` 裡也是這樣包
的。`__aiterm_append_b_marker` 附加在鏈的**最後面**——這一步的目的是要在框架自己的
`PROMPT_COMMAND` 條目（如果有的話，此時已經跑完、`PS1` 已經是這次的最終版本）之後
才接上 `B` 標記，不然框架整段覆蓋 `PS1` 會把標記蓋掉。同樣用「先檢查、不存在才接」
避免無限疊加。）

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test --lib bash_integration_appends_b_marker_to_ps1_after_prompt_command_chain`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src-tauri/src/pty/shell.rs
git commit -m "$(cat <<'EOF'
feat(shell): bash 整合腳本新增 OSC 133 B 標記

跟 Task 1 的 zsh 版本不同，B 標記拆成獨立函式 __aiterm_append_b_marker，
附加在 PROMPT_COMMAND 鏈的最後面，而不是直接寫進 __aiterm_precmd 裡：
bash 沒有 zsh add-zsh-hook 那種「保證排在框架 hook 之後執行」的機制，
現有 __aiterm_precmd 是刻意插在 PROMPT_COMMAND 最前面（要在任何東西
動到 $? 之前先讀走它），若沿用 zsh 版寫法，starship 這類每次整段重新
生成 PS1 的框架會把我們剛接上去的 B 標記蓋掉。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: PowerShell 整合腳本新增 `C`（Enter 鍵覆寫）與 `B` 標記

**這個 Task 的驗證步驟有平台限制，請仔細閱讀 Step 2/4 的說明再執行。**

`inject_powershell_integration` 整支函式是 `#[cfg(windows)]`，在 macOS/Linux 開發機上完全不會被編譯進二進位——這不是這次新增的限制，是這支函式原本就有的既有限制。也就是說 Step 2/4 要求的「執行測試、確認失敗/通過」在 macOS/Linux 上實際上**不會執行到這段程式碼**（`cargo test` 會直接跳過，不會報紅也不會報綠）。這個 Task 仍然要照樣完成程式碼變更與測試撰寫，但這兩步驟只能先用「逐字對照下面提供的程式碼」代替真正執行，真正的紅燈/綠燈驗證要等這個分支之後在 Windows 機器（CI 或使用者的實機）上跑 `cargo test` 才會發生——請在完成這個 Task 時明確在對話中告知使用者這個限制。

**Files:**
- Modify: `src-tauri/src/pty/shell.rs`（`inject_powershell_integration`，約第 44-91 行）
- Test: `src-tauri/src/pty/shell.rs`

- [ ] **Step 1: 寫測試（`#[cfg(windows)]`，此機器上不會編譯執行）**

在同一個 `mod tests { ... }` 區塊內，緊接在既有的 `windows_default_shell_returns_exe_path` 測試之後：

```rust
    #[cfg(windows)]
    #[test]
    fn powershell_integration_emits_c_via_enter_override_and_b_after_rendered_prompt() {
        let spec = inject_powershell_integration(PathBuf::from("pwsh.exe"));
        let script_path = std::env::temp_dir().join("aiterm_ps").join("shell_integration.ps1");
        let content = std::fs::read_to_string(&script_path).expect("script should have been written");

        assert!(
            content.contains(r#"(Get-PSReadLineKeyHandler -Bound |"#),
            "expected the correct Get-PSReadLineKeyHandler usage (-Bound, no -Chord — \
             -Chord is only valid on Set-/Remove-PSReadLineKeyHandler, using it on Get- \
             is a parameter-binding error that prints visibly on every new tab and isn't \
             suppressed by -ErrorAction)"
        );
        assert!(
            content.contains(r#"Where-Object { $_.Key -eq "Enter" }).Function"#),
            "expected the Enter handler lookup to filter the bound-keys list down to Enter"
        );
        assert!(
            content.contains("Set-PSReadLineKeyHandler -Chord Enter"),
            "expected an Enter key handler override to emit the C marker"
        );
        assert!(
            content.contains("    try {"),
            "expected the dynamic method-name invocation to be guarded by try/catch — a \
             custom Enter -ScriptBlock binding makes .Function return a non-method \
             placeholder, and calling that without a catch crashes Enter handling entirely"
        );
        assert!(
            content.contains(r#"[Console]::Write("$([char]27)]133;C$([char]7)")"#),
            "expected the Enter override to emit the C marker after AcceptLine runs"
        );
        assert!(
            content.contains(r#"$rendered = $renderedRaw -join "`n""#),
            "expected renderedRaw to be joined with newlines rather than left for $OFS \
             (default: a single space) to silently mangle a multi-line custom prompt"
        );
        assert!(
            content.contains(r#""$rendered$([char]27)]133;B$([char]7)""#),
            "expected the prompt function to append a B marker after the rendered prompt text"
        );

        // spec.program 本身已經被既有的 windows_default_shell_returns_exe_path 測試涵蓋，
        // 這裡只是避免 unused 警告。
        assert_eq!(spec.program, PathBuf::from("pwsh.exe"));
    }
```

- [ ] **Step 2: （只能在 Windows 上）執行測試，確認失敗**

Run（僅在 Windows 機器/CI 上）：`cd src-tauri && cargo test --lib powershell_integration_emits_c_via_enter_override_and_b_after_rendered_prompt`
Expected: FAIL — 目前的 `prompt` 函式完全沒有 `Set-PSReadLineKeyHandler`、也沒有 `B` 標記。

在 macOS/Linux 開發機上：跳過實際執行，改成逐字比對下面 Step 3 的程式碼是否跟這個測試的斷言字串完全吻合（尤其是 `[Console]::Write` 那行跟 `"$rendered..."` 那行，必須逐字元一致）。

- [ ] **Step 3: 修改 `inject_powershell_integration`**

把 `src-tauri/src/pty/shell.rs` 裡目前的這一整段（含 `$global:__aiterm_orig_prompt` 到 `function global:prompt { ... }` 結尾）：

```rust
    let script = r#"
# ── AITerm Shell Integration (PowerShell) ──
$global:__aiterm_orig_prompt = if (Test-Path Function:\prompt) { ${function:prompt} } else { $null }

function global:prompt {
    # Capture success/exit code FIRST — later statements would overwrite $?
    $wasSuccess = $?
    $origExit = $global:LASTEXITCODE
    $ec = if ($wasSuccess) { 0 } else { if ($origExit) { $origExit } else { 1 } }

    [Console]::Write("$([char]27)]133;D;$ec$([char]7)")
    [Console]::Write("$([char]27)]133;A$([char]7)")

    if ($global:__aiterm_orig_prompt) {
        & $global:__aiterm_orig_prompt
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
    }

    # Restore so user scripts are not affected by our prompt logic
    $global:LASTEXITCODE = $origExit
}
"#;
```

改成：

```rust
    let script = r#"
# ── AITerm Shell Integration (PowerShell) ──
$global:__aiterm_orig_prompt = if (Test-Path Function:\prompt) { ${function:prompt} } else { $null }

function global:prompt {
    # Capture success/exit code FIRST — later statements would overwrite $?
    $wasSuccess = $?
    $origExit = $global:LASTEXITCODE
    $ec = if ($wasSuccess) { 0 } else { if ($origExit) { $origExit } else { 1 } }

    [Console]::Write("$([char]27)]133;D;$ec$([char]7)")
    [Console]::Write("$([char]27)]133;A$([char]7)")

    $renderedRaw = if ($global:__aiterm_orig_prompt) {
        & $global:__aiterm_orig_prompt
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
    }
    # 少見情況：使用者原本的 prompt 函式若回傳多筆管線輸出（沒有用分號/換行抑制的
    # 多行輸出），直接字串插值會被 $OFS（預設一個空白）接起來，跟主控台原本逐行
    # 印出的樣子不一樣。用換行接回去，貼近原本會呈現的樣子。
    $rendered = $renderedRaw -join "`n"

    # Restore so user scripts are not affected by our prompt logic
    $global:LASTEXITCODE = $origExit

    # B marker: appended to the actual rendered prompt text, so it's
    # guaranteed to arrive AFTER the visible prompt characters — unlike A
    # (printed above), which fires before this function's return value is
    # ever echoed to the screen.
    "$rendered$([char]27)]133;B$([char]7)"
}

# C marker: PowerShell has no preexec-equivalent hook, so this overrides the
# Enter key itself. AcceptLine (or the user's own Enter binding, preserved
# below) runs FIRST — that's what produces the newline echo — and C is
# printed AFTER it, once the cursor has actually moved to the new line. This
# matches zsh/bash's ordering (preexec fires after the line editor's own
# newline echo), which recoverUntrackedCommand's cursor-position math in
# useTerminalBlocks.ts relies on being consistent across all three shells.
#
# Get-PSReadLineKeyHandler only accepts -Bound/-Unbound — NOT -Chord (that's
# only valid on Set-/Remove-PSReadLineKeyHandler). Passing -Chord here is a
# parameter-binding error that -ErrorAction SilentlyContinue does NOT
# suppress (it's a statement-level terminating error, not a cmdlet-internal
# one), so it would print visibly on every new tab and leave
# __aiterm_orig_enter_handler always $null, silently discarding any custom
# Enter binding the user actually had.
$global:__aiterm_orig_enter_handler = (Get-PSReadLineKeyHandler -Bound |
    Where-Object { $_.Key -eq "Enter" }).Function

Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock {
    param($key, $arg)
    # .Function isn't guaranteed to be a real PSConsoleReadLine static method
    # name — a user with a custom -ScriptBlock Enter binding gets a
    # placeholder value back (typically "Unknown"), and invoking that
    # dynamically throws. Fall back to AcceptLine if the dynamic call fails,
    # so a custom binding degrades gracefully instead of crashing Enter.
    try {
        if ($global:__aiterm_orig_enter_handler -and
            $global:__aiterm_orig_enter_handler -ne "AcceptLine") {
            [Microsoft.PowerShell.PSConsoleReadLine]::($global:__aiterm_orig_enter_handler)($key, $arg)
        } else {
            [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine($key, $arg)
        }
    } catch {
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine($key, $arg)
    }
    [Console]::Write("$([char]27)]133;C$([char]7)")
}
"#;
```

- [ ] **Step 4: （只能在 Windows 上）執行測試，確認通過**

Run（僅在 Windows 機器/CI 上）：`cd src-tauri && cargo test --lib powershell_integration_emits_c_via_enter_override_and_b_after_rendered_prompt`
Expected: PASS

在 macOS/Linux 開發機上：改跑 `cd src-tauri && cargo build --no-default-features` 確認整個 crate 仍然能正常編譯（這段新程式碼被 `#[cfg(windows)]` 排除在外，不會影響 macOS/Linux 編譯結果，但可以確認沒有不小心破壞這個檔案裡其他部分的語法）。

- [ ] **Step 5: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src-tauri/src/pty/shell.rs
git commit -m "$(cat <<'EOF'
feat(shell): PowerShell 整合腳本新增 OSC 133 B 標記與 C（Enter 覆寫）

PowerShell 原本只有 D+A，完全沒有等效 zsh/bash preexec 的「即將執行」
時間點。用 Set-PSReadLineKeyHandler 覆寫 Enter 鍵補上 C（在呼叫原本
Enter 處理邏輯之後才印，讓時序跟 zsh/bash 一致）；B 則接在 prompt
函式實際回傳的提示字元文字尾端。

Get-PSReadLineKeyHandler 用 -Bound + Where-Object 篩出 Enter，不是
-Chord（那是 Set-/Remove- 才有的參數，用在 Get- 上是參數綁定失敗，
每次開新分頁都會噴紅字錯誤）；動態呼叫方法名稱包 try/catch，避免
使用者自訂 Enter 綁定時當機；$rendered 若是多筆管線輸出改用換行接
回去，避免被 $OFS 用空白接壞。

此 Task 的測試是 #[cfg(windows)]，無法在 macOS/Linux 開發機上實際
執行紅燈/綠燈驗證，需要之後在 Windows 機器或 CI 上補驗證。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `useTerminalBlocks.ts` 新增指令文字還原機制

**Files:**
- Modify: `src/hooks/useTerminalBlocks.ts`
- Test: `src/hooks/useTerminalBlocks.test.ts`

- [ ] **Step 1: 寫失敗的測試**

在 `src/hooks/useTerminalBlocks.test.ts` 檔案末尾，緊接既有的 `"does not signal onUntrackedCommandBoundary when a local block already covers the command"` 測試之後、`});`（`describe` 的收尾）之前，新增：

```ts
  describe("remote-viewer command text recovery (OSC 133 B/C)", () => {
    it("recovers the typed command text and calls beginTrackedBlock when no local block is tracked", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      // 模擬 shell 實際回顯的位元組序列：提示字元文字 → B 標記（提示字元
      // 結束、輸入開始）→ 使用者打的指令文字（遠端觀看者的按鍵，經由 shell
      // 回顯出現在畫面上）→ Enter 的換行回顯 → C 標記。
      await act(async () => {
        await writeToTerm(term, "user@host:~$ \x1b]133;B\x07ls -la\r\n\x1b]133;C\x07");
      });

      expect(result.current.blocks).toHaveLength(1);
      expect(result.current.blocks[0].command).toBe("ls -la");
      expect(result.current.blocks[0].status).toBe("running");
    });

    it("recovers a command that auto-wraps across multiple rows", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));
      // term 是 80 欄；prompt「user@host:~$ 」佔 13 欄，所以這個 95 字元的
      // 指令一定會自動換行到第二行。
      const longCommand = "echo " + "a".repeat(90);

      await act(async () => {
        await writeToTerm(term, `user@host:~$ \x1b]133;B\x07${longCommand}\r\n\x1b]133;C\x07`);
      });

      expect(result.current.blocks).toHaveLength(1);
      expect(result.current.blocks[0].command).toBe(longCommand);
    });

    it("recovers correctly when the prompt itself spans multiple rows before B fires", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      await act(async () => {
        await writeToTerm(term, "== host ==\r\nprompt> \x1b]133;B\x07pwd\r\n\x1b]133;C\x07");
      });

      expect(result.current.blocks).toHaveLength(1);
      expect(result.current.blocks[0].command).toBe("pwd");
    });

    it("does not create a block when the user pressed Enter with nothing typed", async () => {
      const boundaryMock = vi.fn();
      const { result } = renderHook(() =>
        useTerminalBlocks(
          "session-1",
          term,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          boundaryMock,
        ),
      );

      await act(async () => {
        await writeToTerm(term, "user@host:~$ \x1b]133;B\x07\r\n\x1b]133;C\x07");
      });

      expect(result.current.blocks).toHaveLength(0);
      expect(boundaryMock).toHaveBeenCalledWith("start");
    });
  });
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npx vitest run src/hooks/useTerminalBlocks.test.ts -t "remote-viewer command text recovery"`
Expected: FAIL —前三個測試會是 `expect(result.current.blocks).toHaveLength(1)` 收到 `0`（目前 `B` 完全沒被處理，`C` 在沒有本機區塊時只會呼叫 `onUntrackedCommandBoundary`，不會建立區塊）。

- [ ] **Step 3: 新增 `promptEndRef`**

在 `src/hooks/useTerminalBlocks.ts` 裡找到：

```ts
  const blocksRef = useRef<TerminalBlock[]>([]);
  const completionCallbacksRef = useRef<Map<string, (block: TerminalBlock) => void>>(new Map());
```

改成：

```ts
  const blocksRef = useRef<TerminalBlock[]>([]);
  const completionCallbacksRef = useRef<Map<string, (block: TerminalBlock) => void>>(new Map());
  // OSC 133 B 標記記錄的「輸入從這裡開始」絕對座標，給 recoverUntrackedCommand
  // 用——只在遠端指令（沒有本機追蹤區塊）時才會被讀取，見該函式的文件註解。
  const promptEndRef = useRef<{ row: number; col: number } | null>(null);
```

- [ ] **Step 4: 新增 `recoverUntrackedCommand` 輔助函式**

找到檔案裡的：

```ts
function isClearCommand(cmd: string): boolean {
  const trimmed = cmd.trim().toLowerCase();
  return trimmed === "clear" || trimmed === "cls";
}
```

改成：

```ts
function isClearCommand(cmd: string): boolean {
  const trimmed = cmd.trim().toLowerCase();
  return trimmed === "clear" || trimmed === "cls";
}

/**
 * 從一個絕對緩衝區座標（OSC 133 B 標記記錄的「輸入從這裡開始」位置）到目前
 * 遊標所在行，把畫面上的文字截出來當作還原出的指令文字。只在「沒有本機
 * 追蹤區塊」時才會被呼叫——見設計文件
 * docs/superpowers/specs/2026-08-27-remote-command-text-recovery-design.md
 * 的「recoverUntrackedCommand() 演算法」一節。
 */
function recoverUntrackedCommand(
  term: Terminal,
  promptEnd: { row: number; col: number } | null,
): string | null {
  if (!promptEnd) return null;
  const { row: startRow, col: startCol } = promptEnd;
  // OSC C 觸發時，遊標已經因為 Enter 換行到新的一行，所以往上一行才是輸入
  // 內容實際結束的地方。
  const endRow = term.buffer.active.cursorY + term.buffer.active.baseY - 1;
  if (endRow < startRow) return null;

  let fullLine = "";
  for (let row = startRow; row <= endRow; row++) {
    const line = term.buffer.active.getLine(row);
    if (!line) return null;
    fullLine += row === startRow ? line.translateToString(true, startCol) : line.translateToString(true);
  }
  const trimmed = fullLine.trim();
  return trimmed.length > 0 ? trimmed : null;
}
```

- [ ] **Step 5: 把 `beginTrackedBlock` 搬到 OSC effect 之前**

`beginTrackedBlock` 目前定義在 `submitCommand` 之後（檔案偏下方），但下面的 OSC 133 effect 需要直接呼叫它——effect 的 closure 抓的是變數本身，函式定義必須出現在它前面（跟 `finalizeBlock` 已經在 effect 之前是同一個理由）。

先找到 `finalizeBlock` 結尾與 OSC effect 開頭的交界處：

```ts
        if (opts?.clearOnParsed) term?.clear();

        const cb = completionCallbacksRef.current.get(blockId);
        if (cb) {
          completionCallbacksRef.current.delete(blockId);
          const finalBlock = withLines.find((b) => b.id === blockId)!;
          setTimeout(() => cb(finalBlock), 50);
        }
      });
    },
    [term],
  );

  useEffect(() => {
    if (!term) return;
```

改成（在 `finalizeBlock` 跟 `useEffect` 之間插入 `beginTrackedBlock` 的完整定義）：

```ts
        if (opts?.clearOnParsed) term?.clear();

        const cb = completionCallbacksRef.current.get(blockId);
        if (cb) {
          completionCallbacksRef.current.delete(blockId);
          const finalBlock = withLines.find((b) => b.id === blockId)!;
          setTimeout(() => cb(finalBlock), 50);
        }
      });
    },
    [term],
  );

  /**
   * Starts tracking a block for a command that was typed directly into the
   * live terminal (bypassing WarpInput's submitCommand — WarpInput isn't the
   * only way to type into a real terminal). Unlike submitCommand, this does
   * NOT write anything to the PTY: the caller (TerminalView's onData handler,
   * or the OSC 133 B/C recovery path below for remote-viewer-issued commands)
   * has already streamed the keystrokes to the PTY, so writing here again
   * would duplicate/corrupt input. This only does the block-bookkeeping half
   * of submitCommand.
   *
   * 搬到這裡（原本在 submitCommand 之後、檔案偏下方）是因為下面的 OSC 133
   * effect 需要直接呼叫它——effect 的 closure 抓的是變數本身，函式定義必須
   * 出現在它前面。
   */
  const beginTrackedBlock = useCallback(
    (cmd: string) => {
      if (!sessionId) return;

      onCommandStarted?.(cmd);

      if (isClearCommand(cmd)) {
        // Same reasoning as submitCommand's `clear`/`cls` handling — wipe the
        // whole block history instead of tracking a card for it. The keystrokes
        // (including the trailing Enter) are already streaming to the PTY via
        // onData, so there's nothing to write here.
        clearAllBlocks();
        return;
      }

      const prevBlocks = blocksRef.current;
      const prevLatest = prevBlocks[prevBlocks.length - 1];
      if (prevLatest?.status === "running") {
        // Already tracking a block — most likely this Enter press belongs to
        // a submitCommand-initiated command whose OSC 133 D hasn't fired yet.
        // Don't create a second, competing block.
        return;
      }

      const newBlock: TerminalBlock = {
        id: Math.random().toString(36).substring(2, 15) + Date.now().toString(36),
        command: cmd,
        status: "running",
        startTime: Date.now(),
        cwd: cwdRef?.current,
        rawOutput: "",
      };

      const updated = [...blocksRef.current, newBlock];
      blocksRef.current = updated;
      setBlocks(updated);
    },
    [sessionId, cwdRef, clearAllBlocks, onCommandStarted],
  );

  useEffect(() => {
    if (!term) return;
```

接著刪除 `beginTrackedBlock` 原本的位置（在 `submitCommand` 之後、`return {` 之前）。找到：

```ts
  /**
   * Starts tracking a block for a command that was typed directly into the
   * live terminal (bypassing WarpInput's submitCommand — WarpInput isn't the
   * only way to type into a real terminal). Unlike submitCommand, this does
   * NOT write anything to the PTY: the caller (TerminalView's onData handler)
   * has already streamed the keystrokes to the PTY character-by-character as
   * the user typed, so writing here again would duplicate/corrupt input.
   * This only does the block-bookkeeping half of submitCommand.
   */
  const beginTrackedBlock = useCallback(
    (cmd: string) => {
      if (!sessionId) return;

      onCommandStarted?.(cmd);

      if (isClearCommand(cmd)) {
        // Same reasoning as submitCommand's `clear`/`cls` handling — wipe the
        // whole block history instead of tracking a card for it. The keystrokes
        // (including the trailing Enter) are already streaming to the PTY via
        // onData, so there's nothing to write here.
        clearAllBlocks();
        return;
      }

      const prevBlocks = blocksRef.current;
      const prevLatest = prevBlocks[prevBlocks.length - 1];
      if (prevLatest?.status === "running") {
        // Already tracking a block — most likely this Enter press belongs to
        // a submitCommand-initiated command whose OSC 133 D hasn't fired yet.
        // Don't create a second, competing block.
        return;
      }

      const newBlock: TerminalBlock = {
        id: Math.random().toString(36).substring(2, 15) + Date.now().toString(36),
        command: cmd,
        status: "running",
        startTime: Date.now(),
        cwd: cwdRef?.current,
        rawOutput: "",
      };

      const updated = [...blocksRef.current, newBlock];
      blocksRef.current = updated;
      setBlocks(updated);
    },
    [sessionId, cwdRef, clearAllBlocks, onCommandStarted],
  );

  return {
```

改成（只留 `return {`）：

```ts
  return {
```

- [ ] **Step 6: 修改 OSC handler，新增 `B` 分支、`C` 分支改用還原機制**

找到：

```ts
    const disposeOsc = term.parser.registerOscHandler(133, (data) => {
      if (data === "C") {
        // Command start — usually a no-op, since the block was already
        // created synchronously by submitCommand (WarpInput) or
        // beginTrackedBlock (typed directly into the live terminal), both of
        // which run well before this async shell-emitted event round-trips
        // back to the frontend. But if nothing is tracked as "running" at
        // this point, the command didn't come through either of those two
        // paths — it was written to the PTY some other way (e.g. a remote
        // viewer with control access) — so tell the caller "something is
        // running" via the lighter-weight boundary signal instead.
        const prev = blocksRef.current;
        const latest = prev[prev.length - 1];
        if (!latest || latest.status !== "running") {
          onUntrackedCommandBoundary?.("start");
        }
        return true;
      } else if (data.startsWith("D")) {
```

改成：

```ts
    const disposeOsc = term.parser.registerOscHandler(133, (data) => {
      if (data === "B") {
        // Prompt text has just finished being drawn (this marker is embedded
        // at the tail of the shell's PS1/prompt output itself — see
        // src-tauri/src/pty/shell.rs — so it's guaranteed to arrive AFTER the
        // visible prompt characters, unlike A which fires from a hook BEFORE
        // the prompt is drawn). Record exactly where input begins so
        // recoverUntrackedCommand can slice from here.
        promptEndRef.current = {
          row: term.buffer.active.cursorY + term.buffer.active.baseY,
          col: term.buffer.active.cursorX,
        };
        return true;
      } else if (data === "C") {
        // Command start — usually a no-op, since the block was already
        // created synchronously by submitCommand (WarpInput) or
        // beginTrackedBlock (typed directly into the live terminal), both of
        // which run well before this async shell-emitted event round-trips
        // back to the frontend. But if nothing is tracked as "running" at
        // this point, the command didn't come through either of those two
        // paths — it was written to the PTY some other way (e.g. a remote
        // viewer with control access). Try to recover the actual typed text
        // from the screen content between the last B marker and here; only
        // fall back to the lighter-weight boundary signal if that fails (see
        // recoverUntrackedCommand's doc comment for when/why it can fail).
        const prev = blocksRef.current;
        const latest = prev[prev.length - 1];
        if (!latest || latest.status !== "running") {
          const recovered = recoverUntrackedCommand(term, promptEndRef.current);
          if (recovered !== null) {
            beginTrackedBlock(recovered);
          } else {
            onUntrackedCommandBoundary?.("start");
          }
        }
        return true;
      } else if (data.startsWith("D")) {
```

然後找到這個 effect 的依賴陣列：

```ts
  }, [term, finalizeBlock, onLiveClear, onCommandSettled, hostPlatform, onUntrackedCommandBoundary]);
```

改成：

```ts
  }, [term, finalizeBlock, beginTrackedBlock, onLiveClear, onCommandSettled, hostPlatform, onUntrackedCommandBoundary]);
```

- [ ] **Step 7: 更新 `onUntrackedCommandBoundary` 參數的文件註解，反映它現在是保底機制**

找到：

```ts
  /** 偵測到 OSC 133 C/D 標記、但當下沒有任何本機追蹤到的區塊在跑時呼叫。
   *
   *  實機測試抓到的 bug：本機分頁不管指令是從 WarpInput 送出還是直接在
   *  畫面上打字都會走 `submitCommand`/`beginTrackedBlock`，所以永遠有一個
   *  區塊可以標記成「running」；但當指令是遠端觀看者透過分享連線寫進
   *  PTY 時，這兩個函式都不會被呼叫——本機這端看到的只是 shell 回顯的
   *  輸出位元組，OSC 133 C/D 照樣會發生（那是 shell 自己送的，跟輸入
   *  來源無關），只是沒有對應的區塊可以標記。`TerminalView.tsx` 原本
   *  「即時窗格自動撐高」的邏輯完全依賴「有沒有一個 running 中的區塊」
   *  這個信號，於是遠端指令執行時窗格永遠撐不高。
   *
   *  這個 callback 讓呼叫端在不需要知道指令文字（那需要完整重建輸入
   *  當下的畫面快照，複雜度跟遠端分頁要不要支援直接打字變卡片是同一類
   *  問題，刻意不在這裡處理）的前提下，也能拿到「現在有東西在跑」這個
   *  單純的訊號。C 只在**沒有**本機追蹤區塊時才視為「開始」；D 沿用既有
   *  兩個提早 return 的分支（`prev.length === 0` / `latest.status !==
   *  "running"`）——那兩個分支本來就是「沒有東西可以結案」的判斷，同一個
   *  條件借來判斷「這次 D 沒有對應本機區塊」。
   *
   *  必須是穩定的參考（useCallback 空依賴或 ref 橋接），理由跟
   *  `onCommandSettled`/`onCommandStarted` 一樣。 */
  onUntrackedCommandBoundary?: (kind: "start" | "end") => void,
```

改成：

```ts
  /** **保底機制**——只有在 `recoverUntrackedCommand`（見同檔案內的定義）
   *  無法從畫面內容還原出指令文字時才會被呼叫，例如這個連線還沒收過任何
   *  OSC 133 B 標記（見 `promptEndRef`）。正常情況下遠端指令會直接透過
   *  `beginTrackedBlock` 變成完整的卡片，不會走到這裡。
   *
   *  實機測試抓到的原始 bug：遠端觀看者透過分享連線寫進 PTY 的指令，不會
   *  經過 `submitCommand`/`beginTrackedBlock`，導致 `TerminalView.tsx`
   *  「即時窗格自動撐高」邏輯賴以判斷的「有沒有一個 running 中的區塊」
   *  信號永遠是 false。這個 callback 讓呼叫端在指令文字還原失敗的少見
   *  情況下，仍能拿到「現在有東西在跑」這個單純的訊號、維持窗格至少不
   *  裁切輸出——完整解法是 `recoverUntrackedCommand` 成功時直接呼叫
   *  `beginTrackedBlock`，不需要這個訊號介入。
   *
   *  C 只在**沒有**本機追蹤區塊、且還原也失敗時才視為「開始」；D 沿用既有
   *  兩個提早 return 的分支（`prev.length === 0` / `latest.status !==
   *  "running"`）——那兩個分支本來就是「沒有東西可以結案」的判斷，同一個
   *  條件借來判斷「這次 D 沒有對應本機區塊」。
   *
   *  必須是穩定的參考（useCallback 空依賴或 ref 橋接），理由跟
   *  `onCommandSettled`/`onCommandStarted` 一樣。 */
  onUntrackedCommandBoundary?: (kind: "start" | "end") => void,
```

- [ ] **Step 8: 執行測試，確認新測試通過，且沒有弄壞既有測試**

Run: `npx vitest run src/hooks/useTerminalBlocks.test.ts`
Expected: PASS（全部，包含 Task 4 新增的 4 個測試，以及既有的「signals
onUntrackedCommandBoundary when OSC 133 C/D fire with no locally-tracked
block」「does not signal onUntrackedCommandBoundary when a local block
already covers the command」兩個測試——這兩個既有測試完全沒動過斷言，
應該不需要修改就能繼續通過：前者因為它從沒送過 `B`，`promptEndRef` 維持
`null`，`recoverUntrackedCommand` 回傳 `null`，照樣走保底路徑；後者因為
`submitCommand` 已經建立了一個 running 區塊，`recoverUntrackedCommand`
根本不會被呼叫。若這兩個測試意外變紅，代表 Step 5/6 的搬移或改寫破壞了
既有行為，需要回頭檢查，不能直接修改這兩個測試的斷言遷就新程式碼）。

- [ ] **Step 9: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src/hooks/useTerminalBlocks.ts src/hooks/useTerminalBlocks.test.ts
git commit -m "$(cat <<'EOF'
feat(terminal): 用 OSC 133 B/C 標記還原遠端指令文字並建立卡片

新增 recoverUntrackedCommand：收到 B 標記時記錄游標絕對位置（保證
在提示字元文字畫出來之後才會收到，時序上比原本設計依賴 A 標記正確），
收到 C 標記且沒有本機追蹤區塊時，用這個位置到目前游標之間的畫面內容
截出指令文字，直接餵給 beginTrackedBlock——讓遠端觀看者送進來的指令
完全匯入本機既有的卡片資料流，不再需要專屬的 UI 分支。還原失敗時
（例如還沒收過任何 B 標記）維持退回既有的 onUntrackedCommandBoundary
高度保底機制。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: `TerminalView.tsx` 更新保底機制的註解

這個 Task 沒有功能變更，只更新兩段中文註解，反映 `onUntrackedCommandBoundary` 現在的定位是「文字還原失敗時的保底」而不是主要機制。

**Files:**
- Modify: `src/components/TerminalView.tsx`

- [ ] **Step 1: 更新 `untrackedCommandBoundaryRef` 宣告處的註解**

找到：

```ts
  // 實機測試抓到的 bug：遠端觀看者拿到控制權後送進來的指令，本機這端從
  // 沒呼叫過 submitCommand/beginTrackedBlock（那兩支只在「這台機器自己
  // 送出指令」的路徑上），導致下面 liveRows 賴以撐高的「有沒有一個
  // running 中的區塊」這個信號永遠是 false，即時窗格因此撐不高、遠端
  // 指令的輸出被擠在一小條裡。這個 ref 讓 useTerminalBlocks 在偵測到
  // OSC 133 C/D 但沒有對應本機區塊時，也能撐高/縮回同一個 liveRows——
  // 宣告在這裡是因為 useTerminalBlocks 呼叫點在 setLiveRows 宣告之前，
  // 真正賦值要等 liveRows 宣告完才能做（見下方賦值處），先用 ref 佔位。
  const untrackedCommandBoundaryRef = useRef<((kind: "start" | "end") => void) | null>(null);
```

改成：

```ts
  // 保底機制——正常情況下，遠端觀看者送進來的指令會被 useTerminalBlocks
  // 內部的 recoverUntrackedCommand 從畫面內容還原出指令文字，直接變成
  // 跟本機一樣的完整卡片（見 useTerminalBlocks.ts 的 recoverUntrackedCommand
  // 與 docs/superpowers/specs/2026-08-27-remote-command-text-recovery-design.md），
  // 走的是 beginTrackedBlock 那條正常路徑，不會用到這個 ref。這個 ref
  // 只在還原失敗的少見情況下才會被呼叫（例如這個連線還沒收過任何 OSC 133
  // B 標記），確保即使拿不到指令文字，即時窗格至少不會因為完全沒有信號而
  // 裁切掉遠端指令的輸出。
  // 宣告在這裡是因為 useTerminalBlocks 呼叫點在 setLiveRows 宣告之前，
  // 真正賦值要等 liveRows 宣告完才能做（見下方賦值處），先用 ref 佔位。
  const untrackedCommandBoundaryRef = useRef<((kind: "start" | "end") => void) | null>(null);
```

- [ ] **Step 2: 更新賦值處的註解，並順手修正一個既有的簡體字錯字**

找到：

```ts
  // 見上面 untrackedCommandBoundaryRef 宣告處的說明——這裡才真的賦值，
  // 因為 setLiveRows 要到這裡才存在。跟這個檔案其他 ref 一樣直接在
  // render 當下賦值，不用額外包一層 effect。
  //
  // "start" 先撐到 MAX_LIVE_ROWS，避免指令執行途中输出被裁掉（滑鼠滾輪
  // 跟 liveRows 毫無關聯，裁掉了就拿不回來）。"end" 則改成量測游標實際
```

改成：

```ts
  // 見上面 untrackedCommandBoundaryRef 宣告處的說明——這裡才真的賦值，
  // 因為 setLiveRows 要到這裡才存在。跟這個檔案其他 ref 一樣直接在
  // render 當下賦值，不用額外包一層 effect。這整段只在指令文字還原失敗
  // 的保底情況下才會被呼叫，正常情況下走 beginTrackedBlock 直接變成卡片，
  // 不會執行到這裡。
  //
  // "start" 先撐到 MAX_LIVE_ROWS，避免指令執行途中輸出被裁掉（滑鼠滾輪
  // 跟 liveRows 毫無關聯，裁掉了就拿不回來）。"end" 則改成量測游標實際
```

- [ ] **Step 3: 確認 tsc 沒有因為這次改動報錯（純註解變更，理論上不會，但要照規矩驗證）**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

- [ ] **Step 4: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src/components/TerminalView.tsx
git commit -m "$(cat <<'EOF'
docs(terminal): 更新即時窗格保底機制的註解，反映新的還原機制定位

純註解變更，無功能改動。onUntrackedCommandBoundary 現在只在
recoverUntrackedCommand（useTerminalBlocks.ts）還原指令文字失敗時
才會被呼叫，不再是遠端指令唯一能拿到的信號。順手修正一個先前修復
時誤植的簡體字（输出 → 輸出）。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: 完整驗證與真機測試準備

**Files:** 無新增/修改，純驗證。

- [ ] **Step 1: 前端型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

- [ ] **Step 2: 前端完整測試套件**

Run: `npx vitest run`
Expected: 全數通過（含 Task 4 新增的 4 個測試），沒有既有測試被改壞。

- [ ] **Step 3: Rust 測試（zsh/bash 部分在這台機器上可執行；PowerShell 部分會被 `#[cfg(windows)]` 跳過，不算失敗）**

Run: `cd src-tauri && cargo test --lib pty::shell`
Expected: 全數通過，包含 Task 1、Task 2 新增的兩個測試。

- [ ] **Step 4: Rust 完整編譯檢查（含 Windows-only 程式碼的語法正確性間接確認）**

Run: `cd src-tauri && cargo build --no-default-features`
Expected: 成功編譯，無錯誤。

- [ ] **Step 5: Lint 範圍比對（沿用本分支既有的作法：只跑實際改動過的檔案，不要對整個 repo 跑 diff，也不要用 `git checkout` 動到目前這個 worktree 本身——如果需要跟改動前比較，另開一個 disposable 的 `git worktree add --detach` 來比對，絕對不要對這個作用中的 worktree 執行 `git checkout <舊commit> -- .`）**

Run: `npx eslint src/hooks/useTerminalBlocks.ts src/hooks/useTerminalBlocks.test.ts src/components/TerminalView.tsx`
Expected: 沒有新增的 lint 錯誤（這幾個檔案原本就有的、跟這次改動無關的既有錯誤——例如 `react-hooks/refs` 那幾個既有告警——不算這次要修的範圍）。

- [ ] **Step 6: 重新啟動 dev build，準備讓使用者做真機測試**

```bash
ps aux | grep -i "tauri dev\|target/debug/app\|node.*vite" | grep -v grep
```

把列出的舊 process（`npm exec tauri dev`、`node .../tauri`、`node .../vite`、`target/debug/app`）逐一 `kill`，確認 `ps aux` 再查一次是乾淨的，然後：

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity && nohup npm run tauri:dev > /tmp/aiterm-dev.log 2>&1 &
disown
```

等 20 秒後 `tail -40 /tmp/aiterm-dev.log`，確認沒有 port 衝突（`Address already in use`）、沒有重複輪詢（`Another instance is already polling`）之類的錯誤，且 `ps aux | grep target/debug/app` 只有一個新啟動的 process。

- [ ] **Step 7: 明確告知使用者以下事項，不要自己代為判斷完成**

1. **macOS/Linux（zsh/bash）**：請實測「遠端觀看者取得控制權 → 直接在遠端終端機分頁打字送出指令 → 本機端出現文字正確、高度正確的卡片」這個完整流程，包含至少一個會自動換行的長指令。
2. **Windows（PowerShell）**：Task 3 的程式碼在這個 session 完全沒有機會被實際編譯或執行過（`#[cfg(windows)]`），是這次風險最高的部分，需要在真的 Windows 機器上驗證：
   - `cargo test --lib pty::shell` 裡 Task 3 新增的測試是否真的通過。
   - Enter 鍵覆寫是否跟使用者常見的 PowerShell 設定檔（PSReadLine 相關外掛、自訂 Enter 綁定）衝突。
   - 完整流程是否跟 macOS 版一樣正確。
3. 若上述任一項在真機測試發現問題，比照這個 session 一貫的作法：不要用視訊/截圖描述去猜根因，讀實際程式碼、寫紅燈測試證明重現、再修。
