# Python 執行環境管理（uv sidecar + 受管 venv）

日期：2026-07-30
狀態：設計已與使用者確認，待實作計畫

## 背景與目標

AITerm 有兩條 Python 依賴鏈：

- **API 文件抓取**（`api_docs`）→ `tools/ApiDocFetcher/`（curl_cffi、beautifulsoup4、pyyaml）
- **文件轉換與知識庫匯入**（`markitdown`）→ `tools/MarkItDown/`。知識庫並非獨立實作，`commands/knowledge_base.rs:54` 直接呼叫 `markitdown_convert`

現況有四個具體問題：

1. **兩份互不一致的 Python 偵測。** `api_docs/mod.rs:10` 的 doc comment 寫「Tries `python3` then `python` — returns the first that resolves」，實作卻只是 `if cfg!(windows) { "python" } else { "python3" }`，**完全沒有偵測**。在沒安裝 Python 的 Windows 上 `python` 依然存在（WindowsApps 的 App Execution Alias stub），執行它會去開 Microsoft Store，使用者得到的是莫名卡住而非「找不到 Python」。`commands/markitdown.rs:12` 那份則有跑 `-c "import sys; exit(0 if sys.version_info >= (3,10) else 1)"` 實際驗證，是正確的。
2. **污染使用者的 Python 環境。** 兩處都用 `pip install --user --break-system-packages`（`runner.rs:84`、`markitdown.rs:152`），套件落在使用者的 `~/.local`，可能與其自身專案衝突。
3. **首次使用等待無回饋。** pip 以 `--quiet` + `output()` 一次收完，MarkItDown 全套 extras 首次下載很久，畫面看起來像卡死。`runner.rs:92` 的 pip 失敗只 emit warn 就繼續執行，最終表現為更難理解的 Python `ImportError`。
4. **macOS GUI 的 PATH 落差。** 從 Dock 啟動的 app 只有 `/usr/bin:/bin:/usr/sbin:/sbin`，看不到 `/opt/homebrew/bin`。`markitdown.rs` 以硬寫絕對路徑繞過，`api_docs` 沒繞，會落到內建的 `/usr/bin/python3`（3.9，低於 MarkItDown 需要的 3.10+）。

目標：使用者安裝 AITerm 時**不需要**任何 Python 前置條件；真正用到 Python 的功能在被點擊的那一刻自行把環境準備好，過程可見、失敗可修，且不動使用者既有的 Python 環境。

## 範圍界定（已與使用者確認的決策）

| 決策點 | 結論 |
|---|---|
| 安裝時檢查 Python | **不做**。Python 只是週邊功能的依賴，安裝門檻換不到價值；且安裝程式看到的 PATH ≠ app 執行時的 PATH，檢查結果不可靠 |
| 環境建立時機 | **首次使用該功能時自動建立**，過程顯示進度面板 |
| 依賴範圍 | MarkItDown 拆成兩層：預設只裝文件類（pdf/docx/pptx/xlsx），語音轉文字為**候選安裝**（原本寫「影像／語音」，實作後證實 `markitdown[image]` 這個 extra 不存在、且圖片根本不需要它 —— 見文末「實作後推翻的設計前提」） |
| 缺 Python 時 | 引導卡 + 手動指定路徑 + **「幫我安裝」按鈕** |
| 舊版 `--user` 殘留套件 | **不動**，只在設定頁說明。自動移除他人環境中的套件風險過高（其專案可能正在用 markitdown） |
| 環境管理方式 | **內建 uv sidecar**，由 uv 統一負責裝 Python、建 venv、裝套件 |

### 為何選 uv（方案取捨紀錄）

比較過三個方案：

- **A：系統 Python + 專屬 venv**（工程量最小）。但使用者要的「幫我安裝」在 Linux 做不到 —— `apt` 需要 sudo，GUI app 無法乾淨取得權限，只能退化成顯示指令；`winget`／`brew` 也可能不存在。macOS 的 3.9 問題仍需靠引導繞。
- **B：內建 uv sidecar**（選定）。`uv` 單一 binary 同時解決三件事：`uv python install`（自行取得 python-build-standalone，**不需 sudo、UAC 或任何套件管理器**）、`uv venv`、`uv pip install`（比 pip 快一個量級）。三平台是同一條路徑、同一種錯誤處理，程式碼比方案 A 更簡單，因為不必寫三套平台分支。順帶解掉「首次安裝等很久」。
- **C：混合（缺 Python 才即時下載 uv）**。安裝檔零膨脹，但兩條路徑都要維護、測試矩陣翻倍，複雜度買到的只有安裝檔大小。

方案 B 的代價需明確承擔：**安裝檔變大**（本機實測 uv 0.11.19 macOS arm64 為 47.8 MB 未壓縮，打包壓縮後較小；各平台只帶自己那份）、CI 多一個下載步驟、多一個第三方元件需跟版。

### 明確不做（Non-goals）

- 安裝階段的 Python 前置檢查
- uv 自動升級
- 清理舊版 `pip --user` 殘留套件（僅在設定頁說明）
- ApiDocFetcher 去 Python 化（技術上可行 —— 抓網頁 + 解析 HTML 可用 reqwest + scraper，`curl_cffi` 的 TLS 指紋需求 Rust 亦有對應方案 —— 但屬另一個 spec）
- MarkItDown 轉換邏輯本身
- `telegram/mod.rs:167` 的靜默 secret 寫入（同類問題，但不在本次範圍）

## 與既有設計的關係（本 spec 刻意推翻的兩個決策）

`docs/superpowers/specs/2026-06-09-markitdown-integration-design.md` 有兩項當時的明確決定，本 spec 予以變更 —— 它們不是疏漏，變更需要理由：

1. **「Python 3 必須存在於系統 PATH（與現有 ApiDocFetcher 相同前提）」。** 這個前提在 Windows（多數使用者沒有 Python）與 macOS GUI（Dock 啟動看不到 Homebrew 路徑、內建僅 3.9）上都站不住，正是本次要解決的問題。改為由 app 自備 uv 並自行取得 Python，不再對系統 PATH 有任何假設。
2. **「pip install 失敗 → emit warn log，嘗試繼續執行（markitdown 可能已安裝）」。** 當時的理由在共用系統 Python 的前提下是合理的 —— 套件確實可能早已由使用者裝好。但在受管 venv 下該前提消失：venv 由 app 獨佔，且標記檔明確記錄裝了什麼，「可能已安裝」不再是需要容忍的不確定性。因此改為明確早退，避免失敗表現為後續難以理解的 Python `ImportError`。

## 架構

### 唯一權威：`src-tauri/src/python_env/`

新模組取代目前兩份偵測實作（`api_docs::find_python` 與 `markitdown::find_python_for_markitdown` **一併刪除**）。

```rust
pub enum Profile { ApiDocs, DocCore, DocAudio }

pub struct EnvStatus {
    pub uv_available: bool,        // 實作後：只代表「檔案存在且有 exec bit」，見文末
    pub python_version: Option<String>,
    pub installed: Vec<Profile>,
    pub venv_path: String,
    pub user_interpreter: Option<String>,
    pub index_url: Option<String>,  // 實作後新增，見文末「企業鏡像」
}

pub fn status(app: &AppHandle) -> EnvStatus;
pub async fn ensure(app: &AppHandle, profile: Profile) -> Result<PathBuf, PythonEnvError>;
/// 刪除 venv 與 profile 標記檔；`purge_runtimes` 為 true 時連
/// `python-runtimes/` 一併刪除（對應設定頁的「完全刪除」）。
pub async fn reset(app: &AppHandle, purge_runtimes: bool) -> Result<(), PythonEnvError>;
```

（`set_interpreter` 實作在 command 層而非模組層，因為它只是寫 config 加一次驗證，不需要模組內部狀態。`InterpreterSource` 這個列舉沒有實作 —— `Option<String>` 已足夠表達「內建 vs 使用者指定」。）

`ensure` 是唯一入口，依序保證：uv 可用 → Python 可用 → venv 存在 → 該 profile 套件已安裝，回傳 **venv 內的 interpreter 路徑**。呼叫端因此縮成一行：

```rust
let py = python_env::ensure(&app, Profile::DocCore).await?;
```

`runner.rs` 與 `markitdown.rs` 中的偵測段與 pip 段全部移除，這是本次改動最大的簡化。`knowledge_base.rs` 無需修改（走 `markitdown_convert`，自動受益）。

### 落點

| 內容 | 位置 | 理由 |
|---|---|---|
| uv binary | `src-tauri/binaries/uv-<target-triple>[.exe]`，登記於 `tauri.conf.json` 的 `externalBin` | 單一 binary，正是 externalBin 的標準用法（比 DB2 sidecar 自行解析路徑的做法簡單） |
| venv | `{app_data}/python-env/` | 壞掉可整個刪除重建 |
| uv 下載的 Python | `{app_data}/python-runtimes/`，以 `UV_PYTHON_INSTALL_DIR` 指定 | 不寫進 `~/.local/share/uv`，卸載 app 能清乾淨 |
| 已裝 profile 記錄 | `{app_data}/python-env/.aiterm-profiles.json` | 存各 requirements 檔的 sha256；hash 未變即跳過安裝（現況是每次使用都跑一次 pip） |

三個 profile **共用同一個 venv**（每 profile 一個 venv 隔離更乾淨，但磁碟與複雜度都翻倍，不值得）。

### Profile 與 requirements

| Profile | requirements 檔 | 內容 |
|---|---|---|
| `ApiDocs` | `tools/ApiDocFetcher/requirements.txt` | 現況不變 |
| `DocCore` | `tools/MarkItDown/requirements.txt` | 改為 `markitdown[pdf,docx,pptx,xlsx]>=0.1.0` |
| `DocAudio` | `tools/MarkItDown/requirements-audio.txt`（新增） | `markitdown[audio-transcription]>=0.1.0` |

兩個 MarkItDown requirements 檔都要加入 `tauri.conf.json` 的 `resources`。

### 分發

新增 `scripts/setup-uv-mac.sh`、`setup-uv-linux.sh`、`setup-uv-win.ps1`（比照既有 `setup-db2-*`），下載官方 uv release 到 `src-tauri/binaries/`。`binaries/` 維持 gitignored，release CI 加入同一步驟。各平台只打包自己那份。

## 資料流

1. 使用者點「轉換文件」／「抓取 API 文件」／「知識庫匯入檔案」
2. 後端 command 呼叫 `python_env::ensure(app, profile)`
3. `ensure` 期間持續 emit `python-env-log` 事件（`{ level, message }`，形狀與命名比照既有 `api-docs-log`）
4. 前端顯示進度面板（沿用既有 `McpInstallTerminal` 的樣式與 `InstallLogLine` 呈現）
5. `ensure` 回傳 interpreter 路徑後，原有的腳本執行邏輯不變

## UI

- **功能入口 gate**：`ensure` 進行中顯示進度面板；缺 Python 時顯示引導卡，提供三個出口 ——「幫我安裝」（`uv python install`）、「我自己裝好了，重新偵測」、「手動指定路徑」
- **設定頁**（`components/Settings/GeneralPage.tsx`）新增「Python 環境」區塊：狀態、Python 版本、venv 路徑、interpreter 來源，以及兩個動作 ——「重建」（刪除 venv 後重建，保留已下載的 Python runtime）與「完全刪除」（venv 與 `python-runtimes/` 一併清除，供空間回收或徹底重來）。另附舊版 `pip --user` 殘留套件的說明文字
- **候選安裝**：DocConverter 選到需要 `DocAudio` 的檔案而該 profile 尚未安裝時就地提示。判斷依據是副檔名白名單，實作階段須明列（影像：png/jpg/jpeg/gif/bmp/webp；音訊：mp3/wav/m4a/flac），與 `converter.py` 實際支援的格式對齊
- 知識庫匯入路徑也需接 `python-env-log` 事件以顯示同一進度面板

## 錯誤處理

| 情況 | 處置 |
|---|---|
| uv sidecar 不存在 | dev 訊息直接指向 `scripts/setup-uv-*.sh`；production 不應發生（CI 打包），仍須給明確訊息而非 panic |
| uv 可用但取不到 Python（無網路／公司 proxy） | 顯示 uv 原始 stderr，並導向「手動指定路徑」 |
| venv 損壞（檔案被刪、系統升級後 dylib 失效） | **自動重建一次**，再失敗才報錯 —— 此情況常見且可自動修復 |
| 套件安裝失敗 | 顯示尾端輸出 + 重試／檢視完整 log，並**明確早退**（修正 `runner.rs:92` 只 warn 就繼續的行為） |
| 併發觸發（知識庫匯入與文件轉換同時進行） | 以 async Mutex 序列化，後者等前者完成。現況的 pip 有同樣競態但無人處理 |
| wheel 需就地編譯而缺工具鏈 | 訊息須指認出這是編譯失敗並指向平台工具鏈，而非直接倒出整段 compiler 輸出 |

**手動指定的語意**：指定系統 Python 後，仍在 app data 以 `uv venv --python <path>` 建 venv，套件照樣裝進 venv。因此「手動指定」與「uv 自動安裝」的下游完全同構，只有 base interpreter 不同 —— 一條路徑，不是兩套邏輯。

## 測試計畫

- **純函式優先**：將指令組裝抽成回傳 `(program, args, env)` 的純函式，使三平台皆可測 uv 參數（`uv venv --python`、`uv pip install -r`、`UV_PYTHON_INSTALL_DIR`）；另測 profile→requirements 對應、`needs_install` 的三種情況（hash 改變／標記檔不存在／標記檔損壞）、狀態轉換
- **假 uv 整合測試**：以假冒的 uv 可執行檔驗證事件串流與失敗早退（Windows 用 `.cmd`）。若成本過高則退為「僅測純函式 + 手動驗證」，此取捨須在實作計畫中明講，不得默默省略
- **前端 vitest**：狀態 → 引導卡／進度面板／設定頁區塊的呈現，以及候選安裝提示的觸發條件
- **手動驗證表**（本功能風險本質在平台差異，設計者無法在單一平台驗完）：

| 平台 | 已有合格 Python | 完全沒有 Python |
|---|---|---|
| macOS | 手動指定 → venv 建立 → 轉換成功 | 「幫我安裝」→ uv 取得 Python → 轉換成功 |
| Windows | 同上，且需確認不會誤用 WindowsApps stub | 同上，且需確認無 UAC 提示 |
| Linux | 同上 | 同上，且需確認不需 sudo |

## 待驗證假設（不阻擋設計核准）

1. uv 解析 `markitdown[...]` extras 的行為與 pip 一致。若某 wheel 在目標平台無 prebuilt 版本（`curl_cffi` 在較舊的 Windows／Linux 有此風險），uv 同樣需要編譯工具鏈 —— 需在 Windows 實測。
2. uv binary 打包壓縮後對安裝檔的實際增量，需在三平台實測後確認可接受。
3. `UV_PYTHON_INSTALL_DIR` 指向 app data 後，`uv python install` 與 `uv venv --python` 的互動行為需實測確認（尤其 Windows 路徑長度限制）。

---

## 實作後推翻的設計前提（2026-07-31 最終審查）

這份 spec 的部分內容在實作與審查過程中被證實為錯，記錄於此以免誤導後人。上面的內文已就地更正，這一節說明「原本錯在哪、怎麼發現的」。

### 1. `markitdown[image]` 這個 extra 不存在，而且圖片根本不需要它

原設計把 MarkItDown 的 extras 拆成「文件類」與「影像／語音」兩層，前提是 `markitdown[image]` 存在。最終審查查了 PyPI metadata 並實測 uv：

```
warning: The package `markitdown==0.1.7` does not have an extra named `image`
```

markitdown 0.1.7 的 `provides_extra` 沒有 `image`。更關鍵的是**圖片本來就不需要它**：`tools/MarkItDown/converter.py` 對圖片刻意繞過 markitdown、自己用 urllib 打 vision API，唯一的 fallback 用 Pillow，而 Pillow 早就隨 `markitdown[pdf]` → pdfplumber 進了 `DocCore`。

所以原本的行為是：使用者選一張 PNG → 被要求下載 33 MB 的音訊套件 → log 冒出一行紅色警告 → 轉換結果與沒裝完全相同；若他按取消，一個本來會成功的轉換就被中止。

`DocMedia` 已更名 `DocAudio`，`requirements-audio.txt` 只含 `markitdown[audio-transcription]`，副檔名白名單只留 mp3/wav/m4a/flac。順帶發現 `markitdown_pick_file` 的 filter 完全沒有音訊副檔名 —— 那個 profile 的主要入口原本碰不到它。

**教訓**：設計階段假設了一個第三方套件的 extra 名稱卻沒查證，這個錯誤一路帶到實作完成才被發現。查一次 PyPI metadata 的成本是幾秒。

### 2. 「venv 損壞自動重建一次」兜不住安裝中斷

錯誤處理表裡寫「venv 損壞 → 自動重建一次」。實作後審查發現 `ensure()` 的控制流讓那段**永遠到不了**：「venv 不存在」的分支沒有先清空目錄就 `create_venv`，失敗立刻 return。

而審查者實測 uv：目錄存在（不論是不是 venv）它就拒絕，必須 `--clear`。所以任何「venv 目錄在、interpreter 不在」的中斷（app 在 `uv venv` 中被關、防毒隔離 `python.exe`、`reset()` 半途失敗）都會變成永久失敗迴圈。

修法是讓 `create_venv` 一律帶 `--clear --force`（`--force` 讓它也接受非 venv 的髒目錄，並消除 uv 對未來會變 error 的 deprecation 警告）。

### 3. 企業鏡像的路斷了（uv 不讀 `pip.conf`）

原設計沒有考慮到：uv **不讀** `pip.conf`，也不讀 `PIP_INDEX_URL`，只認 `UV_INDEX_URL`。而前兩者正是防火牆後的公司環境能讓舊版（`pip install --user`）成功的原因 —— 也就是說這個改動對企業使用者是**功能退步**。

已補上可設定的 index URL（`AppConfig.python_index_url` → `UV_INDEX_URL`，設定頁有輸入框）。引導卡的文案也拆成兩種情況：GitHub 被封 → 手動指定 interpreter；PyPI 被封 → 設定 index URL（原本把兩者混在一起，而手動指定對 PyPI 被封毫無幫助，因為套件仍要抓 PyPI）。

### 4. `uv_available` 沒有檢查它宣稱的東西

`EnvStatus.uv_available` 原本只檢查檔案存在，但前端用它來判斷「是不是安裝檔壞了」。實作後加上 exec bit 檢查。

**殘留的已知限制**：macOS quarantine 造成的 spawn 失敗不會被這個檢查抓到（實測確認：對加了 `com.apple.quarantine` 的執行檔直接 exec 並不會被 Gatekeeper 擋，所以它其實不是這條路徑的真實風險）。要完全涵蓋得在 `status()` 裡跑一次 `uv --version`，而 `status()` 在設定頁與三個 gate 的熱路徑上，代價不划算。
