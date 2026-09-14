# 桌面版與 aiterm-host 發佈流程分離 — 設計

日期：2026-09-14
狀態：已核准，待實作

## 問題

`aiterm`（Tauri 桌面版）與 `aiterm-host`（headless CLI 主控端）目前共用一條發佈線：
一個 `v*` tag、一個版本號、一個 draft release、一個人工審核關卡。這造成四個問題：

1. host 改一行也會把桌面版版本號推上去，所有桌面使用者收到一個沒有任何桌面異動的更新通知（v1.25.1 就是這樣發出去的）。
2. 發一次 host 要等六個桌面平台建完——`finalize` 的 `needs` 掛著 `build`。
3. host 才剛起步卻已經是 1.25.1，版本號完全是桌面版的歷史包袱。
4. release 的更新說明把兩者混在一起。

## 前提：版本號可以分家

桌面版與 host 的相容性靠 `src-tauri/crates/aiterm-core/src/share/protocol.rs` 的
`PROTOCOL_VERSION`（目前是 2）在握手第一步互檢，不是靠產品版本號相等。
兩邊版本號各走各的在協定層完全安全。

## 現況耦合點

| 耦合 | 位置 |
|---|---|
| 桌面 build 連 host 的 `Cargo.toml` 一起改版 | `release.yml:349-357` |
| host build 自己也改一次版 | `release.yml:616-625` |
| `finalize` 等全部十一個 job | `release.yml:799` |
| `releases/latest` 只有一個 | `tauri.conf.json:31`（更新器）、`scripts/install.sh:45`、`scripts/install.ps1:29`；`release.yml` 的 `gh release edit --latest` 指派它 |
| CHANGELOG 按版本號抓段落 | `release.yml` 呼叫 `scripts/release_notes.py draft "$TAG" CHANGELOG.md` |

## 陷阱：pre-release guard 會吃掉每一次正式 host 發佈

npm / Homebrew / 容器三條對外管道的彩排版判斷都是：

```bash
if [[ "$TAG" == *-* ]]; then skip=true; fi
```

`host-v0.2.0` 本身就含 `-`。照搬過去的話**每一次正式 host 發佈都會被判成彩排、
三條管道全部安靜跳過，而且 job 是綠的**。

修法：先剝前綴再判。

```bash
VERSION="${TAG#host-v}"
if [[ "$VERSION" == *-* ]]; then skip=true; fi
```

三處都要改，且必須一致——`release.yml` 的註解已經寫過「改一邊忘了另一邊，
就會有一條管道把演練版發出去」。

## 設計

### 1. tag 與版本來源

| | 桌面 | host |
|---|---|---|
| 正式 tag | `v1.26.0` | `host-v0.2.0` |
| 彩排 tag | `v1.26.0-dist1` | `host-v0.2.0-dist1` |
| 版本來源 | tag（CI 改寫 `package.json` / `tauri.conf.json` / `src-tauri/Cargo.toml`，維持現況） | **repo 裡的 `src-tauri/crates/aiterm-host/Cargo.toml`**；CI 只驗證與 tag 一致，不一致就 fail |

host 改成「版本寫在 repo、CI 驗證」而不是照抄桌面的改寫法，理由：
今天 repo 裡的 `0.1.0` 是假的（原始碼跟發出去的執行檔 `--version` 對不起來），
因為版本跟著別人的 tag 走。拆開後 host 版本是它自己的真實資訊，
應該可讀、可 review、可跟 `CHANGELOG-host.md` 對照。

桌面的 build job 刪掉改寫 `aiterm-host/Cargo.toml` 那段（`release.yml:349-357`）。

host 版本從 **0.2.0** 重新起算。已發出的 1.25.0 / 1.25.1 視為誤植。
npm 的 `latest` dist-tag 不需手動處理：`npm publish` 不帶 `--tag` 時一律把
`latest` 指到剛發的版本，即使版號比既有的小。Homebrew tap repo 尚未建立、
從未成功發佈過，沒有既有使用者會遇到降版問題。

### 2. 檔案結構

```
.github/workflows/release.yml           桌面（v*）— 移除 cli-* 五個 job
.github/workflows/release-host.yml      新增（host-v*）
.github/actions/create-draft-release/   共用複合 action
CHANGELOG.md                            桌面
CHANGELOG-host.md                       新增，host
```

`create-draft-release` 抽成複合 action 而不是複製一份：那段 github-script
（掃 releases、優先取已發佈版而非殘留 draft、避免重複 draft）是踩過真實的坑
才長成現在這樣（repo 裡還留著 v0.1.77、v0.1.94 兩組重複 draft）。
複製到第二個檔案等於等它漂移。

介面：

- 輸入：`tag`、`name`、`body`
- 輸出：`release_id`
- 行為與現行 `create-release` job 的 `actions/github-script` 步驟完全相同

`scripts/release_notes.py` **不改**——它已經吃 changelog 路徑參數。
host workflow 先算 `VERSION=${TAG#host-v}`，傳 `v$VERSION` 給它，
搭配 `CHANGELOG-host.md`。

### 3. `releases/latest` 的歸屬

一個 repo 只有一個 `releases/latest`。兩條產品線都發到同一個 repo，
誰後發誰就搶走它。歸屬規則：

- **`latest` 永遠屬於桌面版。** 桌面 `finalize` 維持 `gh release edit --draft=false --latest`，
  `tauri.conf.json:31` 的更新器端點不用改。
- host `finalize` 用 `gh release edit --draft=false`，**不加 `--latest`**。
- `scripts/install.sh` / `scripts/install.ps1` 改成列出 releases
  （`GET /repos/{repo}/releases?per_page=30`），取第一個
  `draft=false && prerelease=false && tag_name` 以 `host-v` 開頭的版本。
  找不到就明確報錯，**不 fallback 回 `releases/latest`**——fallback 只會把
  「host 沒發成功」偽裝成「下載失敗」。

這順帶修掉一個現存的隱性 bug：今天 `install.sh` 打 `releases/latest`，
只要最新一版沒有 host 資產就會抓空。拆開後那會是常態。

`README.md` 的 release badge 指向 `releases/latest`，維持不動（它代表桌面版）。

### 4. host workflow 的 job 圖

```
create-release ─┬→ cli-build (5 targets) ─┬→ cli-checksums ─┐
                                          └→ cli-container ─┤
                                                            ├→ finalize
                                                            │  (environment: release-approval)
                                                            └→ ┬ cli-container-push
                                                               ├ cli-homebrew
                                                               └ cli-npm
```

與現行 host 相關的 job 完全相同，只是不再與桌面的 `build` 共用 `finalize`。

- 審核關卡沿用同一個 `release-approval` environment。同一批 reviewer，
  少建一個 environment 就少一個「environment 不存在 → 靜默放行」的風險面。
- host 的 `finalize` **不**下載 `.sig`、**不**產 `latest.json`——那是桌面更新器專用的。
  它只做：讀回審核期間人工編輯過的 release body、`release_notes.py extract`
  驗證 changelog 區塊非空、然後發佈。
- 三條對外管道維持 `needs: finalize` 與 `continue-on-error: true`。

桌面 `release.yml` 的 `finalize` 改成 `needs: build`。

### 5. 發佈內容分離

- 桌面 release：`AITerm v1.26.0`，body 維持現行格式（六個平台下載說明、
  macOS quarantine 提示、AppImage 執行權限提示）。
- host release：`AITerm Host 0.2.0`，body 只講 host——
  `brew` / `npx` / `curl | sh` / `irm | iex` / 容器五種取得方式，
  加上 `SHA256SUMS` 驗證步驟。
- 容器映像 tag 從 `aiterm-host:${{ github.ref_name }}` 改成
  `aiterm-host:${VERSION}` + `aiterm-host:latest`，不要把 `host-v` 前綴帶進映像 tag。

### 6. 測試與驗證

- `scripts/test_*.py`（`release_notes.py` 的單元測試）兩個 workflow 的
  `create-release` 都跑——兩邊都用到它。
- npm launcher 測試（`node --test npm/aiterm-host/test/*.test.mjs`）只留在
  host workflow。
- host workflow 新增一步：比對 `src-tauri/crates/aiterm-host/Cargo.toml` 的版本
  與 `${TAG#host-v}`，不一致就 fail。放在 `create-release`，
  讓它在整條線最前面擋下來。
- 真機驗證（這個 repo 既有的彩排慣例）：
  1. 推 `host-v0.2.0-dist1`，確認五個平台建得過、draft release 有全部資產、
     且三條對外管道**確實被 guard 跳過**。
  2. 推 `host-v0.2.0` 正式版，確認三條管道**確實沒有被跳過**。
  3. 確認 `releases/latest` 仍然是桌面版那一則。
  4. 在乾淨機器上跑 `install.sh`，確認它抓到 `host-v0.2.0` 而不是桌面版。

「確實被跳過」與「確實沒被跳過」兩個方向都要驗——只驗其中一邊的話，
pre-release guard 的前綴 bug 正好會通過。

### 7. 遷移順序

1. `src-tauri/crates/aiterm-host/Cargo.toml` 版本改成 `0.2.0`，commit。
2. 新增 `CHANGELOG-host.md`，把 `CHANGELOG.md` 裡 1.25.0 / 1.25.1 的 host 條目
   搬過去，標註「先前隨桌面版號發佈」。`CHANGELOG.md` 保留桌面條目。
3. 改 `scripts/install.sh` / `scripts/install.ps1`。
4. 抽出 `.github/actions/create-draft-release`，讓現行 `release.yml` 先改用它
   （此時行為不變，可獨立驗證）。
5. 新增 `release-host.yml`，從 `release.yml` 移除 cli-* 五個 job，
   `finalize` 改成 `needs: build`。
6. 推 `host-v0.2.0-dist1` 彩排。
7. 推 `host-v0.2.0` 正式。

## 明確不做

- 不把 host 拆到另一個 repo。`aiterm-core` 是原始碼層級共用，
  拆 repo 會逼出 crates.io 發佈或 git submodule，成本遠大於收益。
- 不改 `PROTOCOL_VERSION` 或任何相容性機制。
- 不動桌面版的更新器端點與 `latest.json` 產生邏輯。
- 不回收已發佈的 npm `aiterm-host@1.25.x`。
