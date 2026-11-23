# 問題收集 Server 部署指南

`aiterm-intake-server` 是常駐的問題收集 server：收 LINE 訊息、保存共用的問題清單，讓團隊每個人的 AITerm 連上來看同一份清單與處理狀態。設計見 `docs/superpowers/specs/2026-11-23-intake-server-design.md`。

## 一、準備與啟動

```bash
# 1. 建資料夾、金鑰、自簽憑證與資料庫（可重複執行，不會覆蓋既有的金鑰與憑證）
aiterm-intake-server init --san intake.example.com --san 192.168.1.20

# 2. 建專案（可多個）
aiterm-intake-server project add 範例科技

# 3. 產生第一個管理員的邀請碼（15 分鐘內有效、只能用一次）
aiterm-intake-server pair --name 管理員 --role admin --address intake.example.com:8443

# 4. 啟動（預設 API 在 0.0.0.0:8443；LINE webhook 只收本機）
aiterm-intake-server run
```

資料夾預設是 `~/.aiterm-intake`，可用 `--data-dir` 改。裡面有 `intake.db`（問題清單與設定）、`server.db`（成員、邀請、稽核、專案）、`secrets.key` 與 `secrets.json`（加密後的祕密）、`tls/`（憑證）。**備份整個資料夾**；`secrets.key` 遺失就解不開 `secrets.json`。

## 二、成員連線

成員在 AITerm 的「問題收集 → 連線」輸入 server 位址與邀請碼。每位成員一組邀請碼：

```bash
aiterm-intake-server pair --name 小明 --role member
aiterm-intake-server members list
aiterm-intake-server members revoke 小明     # 已連線的 AITerm 會在幾秒內被中斷
```

- **member**：看與處理題目（核准、駁回、刪除、改草稿、回覆）。
- **admin**：另可改設定、白名單、專案、祕密與 AI 設定。
- 連到 server 期間，那台電腦自己的收訊與派送會停止（避免同一封信被收兩次）；中斷連線才恢復。

## 三、憑證

- **內網／VPN（預設）**：`init` 產生自簽憑證。AITerm 在配對時釘住它的指紋，之後只信這張；憑證改變時 AITerm 會警告並拒絕連線，必須重新配對。
- **公開網路**：用正式憑證：`aiterm-intake-server run --tls-cert fullchain.pem --tls-key privkey.pem`。AITerm 配對後會先試著用一般的根憑證驗證，通過就不釘指紋，憑證換新不會誤報。（這個模式目前只有自動測試的部分涵蓋，還沒有用真實的正式憑證驗收。）

## 四、LINE

1. 在 LINE Developers 的 Messaging API 取得 Channel secret 與 Channel access token，由管理員在 AITerm 的 LINE 分頁填入（只寫入，不會被讀出）。
2. 啟用 LINE、設定埠（預設 8321）後，server 在 `--channel-bind`（預設 127.0.0.1）的該埠接 `/line/webhook`。LINE 要求 webhook 是公開的 HTTPS，請用反向代理（Caddy、nginx）或 Tunnel（ngrok、Cloudflare Tunnel）把公開網址轉到這個埠；若代理在另一台機器，用 `--channel-bind 0.0.0.0` 並自行限制來源。
3. 把 LINE 的 webhook URL 設為 `https://<公開網址>/line/webhook`，並打開「Use webhook」。

## 五、AI（用於題目分類與群組「是不是新問題」）

```bash
echo "$API_KEY" | aiterm-intake-server ai --provider openai --model <模型> 
# provider：openai、anthropic、openai_compat（再加 --base-url）、ollama（不需要金鑰）
```

也可以由管理員在 AITerm 設定。不支援需要瀏覽器登入的訂閱帳號。沒設定時，LINE 群組每段安靜的訊息都會被當成新問題，建議設定。

## 六、systemd 範例（Linux）

```ini
[Unit]
Description=AITerm Intake Server
After=network-online.target

[Service]
ExecStart=/usr/local/bin/aiterm-intake-server --data-dir /var/lib/aiterm-intake run --bind 0.0.0.0:8443
Restart=on-failure
User=aiterm
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
```

## 七、目前的限制

- Server **不會自動處理題目**；診斷與回覆需要人。
- 只支援 **LINE** 收訊與回覆；Email、Telegram、網頁表單還不能在 server 上使用（設定會存起來，但不收訊）。
- 看板與 Agent 協作不在 server 上。
- 所有成員看得到所有專案。
