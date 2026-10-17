# Agent 協作：精簡啟動

日期：2026-10-17
狀態：使用者同意（「依你的建議做直到功能完成為止」）

## 問題

每張派出去的卡都是新的 claude，一啟動就要讀一大段跟工作無關的內容：使用者全域的外掛
（例如 superpowers 的 SessionStart 提示）、技能清單、使用者自己的 MCP 伺服器的工具說明。
實測一張審查卡第一次呼叫的上下文約 5.8 萬 token，之後每次呼叫都會再讀一次（快取讀取）。

## 量測（claude 2.1.300，`-p` 模式第一次呼叫，空資料夾）

| 旗標 | 上下文 token |
|---|---|
| 預設 | 25,324 |
| `--strict-mcp-config` | 21,903 |
| ＋`--disable-slash-commands` | 14,381 |
| ＋`--no-chrome` | 14,381（沒差） |
| ＋`--settings` 關掉使用者啟用的外掛 | 11,768 |

`--bare` 不能用：它連 `--settings` 裡的 hook（額度續跑、等你回覆）都略過，也不讀 OAuth 登入。

## 作法

- 看板設定新增「Agent 協作精簡啟動」，預設開。
- 只套用在**非互動的 Agent 卡**（互動卡由使用者操作，保留完整環境）。
- 開著時：指令加 `--strict-mcp-config --disable-slash-commands`；派工寫的 `claude-settings.json`
  另外放 `"enabledPlugins": {<使用者在 ~/.claude/settings.json 啟用的外掛>: false}`。
- 卡片自己需要的 MCP（ask_human、驗收者的瀏覽器）照樣透過 `--mcp-config` 給。

## 代價

卡片拿不到使用者的外掛、技能與自己的 MCP 伺服器。需要的話在設定裡關掉。
