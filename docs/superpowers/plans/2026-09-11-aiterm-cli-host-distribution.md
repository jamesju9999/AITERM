# AITerm CLI Host 發布管道 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓任何人在 macOS／Linux／Windows／容器裡都能用一行指令裝到 `aiterm-host`，而不是只能自己 clone 下來編。

**Architecture:** 在既有的 `release.yml` 上加一個獨立的 `cli-build` job 矩陣（五個目標，含兩個 musl 靜態），把壓縮檔與 `SHA256SUMS` 上傳到同一個 draft release；其餘三條管道（安裝腳本、ghcr 容器映像、Homebrew tap、npm）全部從**同一批 artifact** 衍生，彼此獨立，哪一條卡住都不擋主線。

**Tech Stack:** GitHub Actions、Rust cross-compilation（musl）、Docker／ghcr.io、Homebrew formula、npm optionalDependencies 分平台套件。

**前一份計畫：** `docs/superpowers/plans/2026-09-10-aiterm-cli-host.md`（核心功能，已完成並合併）
**Spec：** `docs/superpowers/specs/2026-09-10-aiterm-cli-host-design.md` 的「發布管道」一節

---

## 動工前已經查證的事實（不要重新懷疑，但也不要當成沒查過就寫進程式碼）

1. **`cargo build -p aiterm-host` 不需要 uv／db2 sidecar。** 實測：把
   `src-tauri/binaries/` 移走後 `cargo check -p aiterm-host` 正常完成，而
   `cargo check -p app` 如預期失敗於 `resource path 'binaries/uv-aarch64-apple-darwin' doesn't exist`。
   所以 CLI 的 job **不要**跑 `scripts/setup-uv-*` 或 `scripts/setup-db2-*`。
2. **npm 的 `aiterm` 已被別人佔用**（有人發過 0.0.3）。本計畫用 **`aiterm-host`**
   （實測可用），分平台子套件叫 `aiterm-host-<platform>`。
3. **`release.yml` 的結構**：`create-release`（建 draft，輸出 `release_id`）→
   `build`（六個目標的 app 矩陣，`tauri-action` 上傳到那個 draft）→
   `finalize`（`needs: build`，組 updater 的 `latest.json`，然後
   `gh release edit --draft=false --latest` 把 release 發佈出去）。
4. **`scripts/` 已經有 python unittest 的先例**：`scripts/test_release_notes.py`，
   而且 `create-release` job 會跑
   `python3 -m unittest discover -s scripts -p 'test_*.py'`。新腳本的測試照這個模式寫，
   會自動被 CI 跑到。
5. **musl 能不能編是唯一還沒驗的風險**（本機沒有 docker daemon、也沒有 musl 交叉
   工具鏈）。Task 1 就是驗它。

---

## ⚠️ 一個會讓主打功能安靜失效的陷阱

`pty::shell::inject_shell_integration` **只對路徑結尾是 `bash` 或 `zsh` 的 shell
注入 OSC 133 標記**（見 `src-tauri/crates/aiterm-core/src/pty/shell.rs`，
`unix_default_shell()` 的候選清單是 `/bin/zsh`、`/bin/bash`、`/bin/sh`，但只有前兩者
會被注入）。

而遠端 AI agent 迴圈**完全依賴 `OSC 133;D` 判斷「這一步跑完了」**。沒有標記時每一步
都要等 60 秒逾時，功能形同壞掉——**而且不會有任何錯誤訊息**。

`alpine` 預設只有 busybox 的 `/bin/sh`。所以容器映像**必須裝 bash 並把 `SHELL`
指到它**，否則做出來的是一個「連得上但 AI 用不了」的映像。Task 5 的煙霧測試就是
在守這件事。

---

## 檔案結構

### 新建

| 路徑 | 責任 |
|------|------|
| `scripts/install.sh` | Unix 安裝腳本：偵測 OS/arch → 抓 asset → 驗 checksum → 裝進 `~/.local/bin` |
| `scripts/install.ps1` | Windows 對應版本 |
| `scripts/test_install_sh.py` | 用假的 uname 輸出測 `install.sh` 的目標三元組推導 |
| `scripts/bump_homebrew_formula.py` | 從 release asset 產生／更新 formula 的 url + sha256 |
| `scripts/test_bump_homebrew_formula.py` | 上者的測試 |
| `docker/Dockerfile.host` | CLI host 的容器映像 |
| `npm/aiterm-host/package.json` | npm 入口套件（`optionalDependencies` 指向分平台子套件） |
| `npm/aiterm-host/bin/aiterm-host.js` | 挑出對應平台的執行檔並 exec |
| `npm/aiterm-host/test/resolve.test.mjs` | 平台解析邏輯的測試 |
| `npm/build-packages.mjs` | 從 release artifact 組出所有 npm 套件目錄 |
| `.github/workflows/release.yml` 內的 `cli-build` job | 五個目標的 CLI 建置與上傳 |

### 修改

| 路徑 | 改什麼 |
|------|--------|
| `.github/workflows/release.yml` | 新增 `cli-build` 與 `cli-container` job；`finalize` 的 `needs` 要加上它們；版本同步步驟要涵蓋 `aiterm-host` |
| `README.md` | 新增「安裝 aiterm-host」一節 |
| `CHANGELOG.md` | 發版說明 |

---

## Task 1: 驗證 musl 真的編得出來也跑得動（**gating**）

> **✅ 已於 2026-09-11 在本機 docker 驗證通過，這個任務不用再做一次。**
>
> | 目標 | 編譯 | 執行 | `file` 的判定 |
> |------|------|------|--------------|
> | `aarch64-unknown-linux-musl` | ✅ 51.7s | ✅ `--print-connection` 正常 | `statically linked` |
> | `x86_64-unknown-linux-musl` | ✅ 103s（QEMU 模擬） | ✅ | `static-pie linked` |
>
> 兩者都在 `rust:alpine` 裡以 `apk add musl-dev` + `cargo build -p aiterm-host
> --release` 完成。`ring`（rustls 的加密後端）在 musl 上沒有問題。
>
> **結論：發布矩陣的兩格 musl 成立，容器基底可以用 alpine，不需要退回 gnu 或
> debian-slim。** 下面的步驟保留下來，是給「日後升級相依、懷疑 musl 壞掉時」
> 重跑用的。


整份計畫的五個目標裡有兩個是 musl。**這個任務失敗的話，發布矩陣要改成 gnu 動態連結、
容器基底要從 `alpine` 換成 `debian-slim`**——那是範圍變更，要回報使用者，不是自己決定。

**Files:** 無（純驗證）

- [ ] **Step 1: 先試本機 docker**

```bash
docker version --format '{{.Server.Version}}'
```

有回應就走 Step 2；連不上（本機實測是這個狀態，daemon 沒跑）就跳到 Step 3 走 CI。

- [ ] **Step 2: 本機 docker 驗證**

```bash
cd /path/to/AITERM
docker run --rm -v "$PWD":/w -w /w/src-tauri rust:alpine sh -c '
  apk add --no-cache musl-dev &&
  cargo build -p aiterm-host --release --target-dir /tmp/muslbuild &&
  /tmp/muslbuild/release/aiterm-host --print-connection
'
```

預期：印出連線資訊（位址／埠／64 個 hex 的金鑰）。

**注意 `--target-dir /tmp/muslbuild`**：不要讓它寫進 repo 的 `src-tauri/target`，
那個目錄是跟主 checkout 共用的，混進 Linux 產物會拖慢本機建置。

- [ ] **Step 3: 走 CI 驗證（沒有 docker 時）**

建一個暫時的 workflow `.github/workflows/musl-probe.yml`：

```yaml
name: musl probe
on: workflow_dispatch

jobs:
  probe:
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-musl
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-musl
    runs-on: ${{ matrix.os }}
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - run: sudo apt-get update && sudo apt-get install -y musl-tools
      - name: Build
        working-directory: src-tauri
        run: cargo build -p aiterm-host --release --target ${{ matrix.target }}
      - name: 真的執行一次，不是只確認編得過
        working-directory: src-tauri
        run: ./target/${{ matrix.target }}/release/aiterm-host --print-connection
      - name: 確認是靜態連結
        working-directory: src-tauri
        run: |
          file ./target/${{ matrix.target }}/release/aiterm-host
          ldd ./target/${{ matrix.target }}/release/aiterm-host 2>&1 | tee ldd.txt
          grep -q "not a dynamic executable\|statically linked" ldd.txt \
            || { echo "不是靜態連結——alpine/distroless 會跑不起來"; exit 1; }
```

推上去之後 `gh workflow run musl-probe.yml`，等結果。

**「編得過」不算數，一定要真的執行一次**——`ring`（rustls 的加密後端）在 musl 上
是最可能出問題的地方，而連結期的問題有時候要到執行才爆。

- [ ] **Step 4: 判定**

- 兩個目標都綠 → 刪掉 `musl-probe.yml`，繼續 Task 2。
- 任一個紅 → **停下來回報使用者**，附完整錯誤。備案是那一格改用
  `*-unknown-linux-gnu`（動態連結，容器基底改 `debian:stable-slim`），但那要使用者
  同意才做。

- [ ] **Step 5: Commit（只在走了 Step 3 時需要）**

```bash
git add .github/workflows/musl-probe.yml
git commit -m "ci: 暫時的 musl 可行性探測 workflow"
```

驗完之後：

```bash
git rm .github/workflows/musl-probe.yml
git commit -m "ci: 移除 musl 探測 workflow（已驗證可行）"
```

---

## Task 2: 版本同步涵蓋 aiterm-host

`release.yml` 的「Sync version from tag」步驟目前只改 `package.json`、
`src-tauri/tauri.conf.json` 與 `src-tauri/Cargo.toml`。`aiterm-host` 有自己的
`Cargo.toml`，版本停在 `0.1.0`——不處理的話，發出去的執行檔跑 `--version` 會回報
`0.1.0`，跟 release 的 tag 對不起來。

**Files:**
- Modify: `.github/workflows/release.yml`（Sync version from tag 步驟）
- Modify: `src-tauri/crates/aiterm-host/src/main.rs`（加 `--version`）

- [ ] **Step 1: 寫失敗測試——`--version` 要回報 crate 版本**

加到 `src-tauri/crates/aiterm-host/src/main.rs` 的 `mod tests`：

```rust
    #[test]
    fn the_version_flag_reports_the_crate_version() {
        // 發版時 CI 會把 tag 的版本寫進這個 crate 的 Cargo.toml。使用者回報問題時
        // 第一件要問的就是「你跑的是哪一版」，所以這支一定要有，而且要跟 release
        // 的 tag 對得起來。
        let err = Args::try_parse_from(["aiterm-host", "--version"]).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains(env!("CARGO_PKG_VERSION")),
            "--version 要印出 crate 版本，got: {text}"
        );
    }
```

`clap` 的 `--version` 是以 `Err(DisplayVersion)` 的形式回傳的，不是正常輸出——
所以這裡用 `try_parse_from` 接 `unwrap_err`，不是 `parse_from`。

- [ ] **Step 2: 跑測試確認會紅**

```bash
cd src-tauri && cargo test -p aiterm-host the_version_flag_reports_the_crate_version
```

預期：FAIL——`#[command(...)]` 目前沒有 `version`，clap 不認得 `--version`。

- [ ] **Step 3: 加上 version**

`main.rs` 的 `#[command(...)]` 改成：

```rust
#[command(
    name = "aiterm-host",
    version,
    about = "把一個 shell 開放給 AITerm 遠端連線與 AI 操作"
)]
```

- [ ] **Step 4: 跑測試**

```bash
cd src-tauri && cargo test -p aiterm-host
```

預期：全 PASS。

- [ ] **Step 5: 版本同步步驟涵蓋新 crate**

在 `.github/workflows/release.yml` 的「Sync version from tag」那段 node 腳本裡，
`src-tauri/Cargo.toml` 的處理之後加上：

```javascript
          // aiterm-host 是獨立 crate，有自己的版本號。不同步的話發出去的執行檔
          // `--version` 會回報 0.1.0，跟 release 的 tag 對不起來。
          // 用 `^version = ` 搭配 m 旗標、不加 g：只換第一個match，也就是
          // [package] 區塊那一行，不會誤傷任何相依項。
          const hostManifest = 'src-tauri/crates/aiterm-host/Cargo.toml';
          let host = fs.readFileSync(hostManifest, 'utf8');
          host = host.replace(/^version = ".*"/m, `version = "${version}"`);
          fs.writeFileSync(hostManifest, host);
          console.log(`aiterm-host version synced to ${version}`);
```

`aiterm-core` **刻意不同步**：它是內部相依、不對外發佈，版本號沒有對外意義。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/aiterm-host/src/main.rs .github/workflows/release.yml
git commit -m "feat(host): 加上 --version，並讓發版流程同步這個 crate 的版本"
```

---

## Task 3: cli-build job — 五個目標的執行檔與 checksum

**Files:**
- Modify: `.github/workflows/release.yml`

- [ ] **Step 1: 新增 cli-build job**

加在 `build` job 之後、`finalize` 之前：

```yaml
  # CLI host 的執行檔。刻意跟 app 的 `build` job 分開，而不是塞進它的矩陣：
  #
  #   1. 目標不一樣。這裡有兩個 musl（靜態，給 alpine/distroless/老 glibc）和
  #      x86_64-apple-darwin（Intel Mac），app 的矩陣三個都沒有。
  #   2. 這個 job **不需要** uv／db2 sidecar。實測確認：把 src-tauri/binaries
  #      移走後 `cargo check -p aiterm-host` 照樣過，而 `cargo check -p app` 會
  #      失敗於 "resource path 'binaries/uv-...' doesn't exist"。跑 setup 腳本
  #      只是白白多好幾分鐘。
  #   3. 分開之後，CLI 這邊壞掉不會拖垮 app 的發版。
  cli-build:
    needs: create-release
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: macos-latest
            target: aarch64-apple-darwin
            archive: tar.gz
          - os: macos-latest
            target: x86_64-apple-darwin
            archive: tar.gz
          - os: ubuntu-latest
            target: x86_64-unknown-linux-musl
            archive: tar.gz
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-musl
            archive: tar.gz
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            archive: zip
    runs-on: ${{ matrix.os }}
    timeout-minutes: 25
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
          shared-key: cli-${{ matrix.target }}

      - name: Install musl toolchain
        if: contains(matrix.target, 'musl')
        run: sudo apt-get update && sudo apt-get install -y musl-tools

      - name: Sync version from tag
        shell: node {0}
        run: |
          const fs = require('fs');
          const version = process.env.GITHUB_REF_NAME.replace(/^v/, '');
          const p = 'src-tauri/crates/aiterm-host/Cargo.toml';
          let s = fs.readFileSync(p, 'utf8');
          s = s.replace(/^version = ".*"/m, `version = "${version}"`);
          fs.writeFileSync(p, s);
          console.log(`aiterm-host version synced to ${version}`);

      - name: Build
        working-directory: src-tauri
        run: cargo build -p aiterm-host --release --target ${{ matrix.target }}

      - name: 真的執行一次
        # 編得過不代表跑得動——musl 上 ring 的連結問題有時候要到執行才爆。
        # Windows 與 arm64 Linux 都是原生 runner，五個目標裡只有
        # x86_64-apple-darwin 是交叉編譯，跳過它。
        if: matrix.target != 'x86_64-apple-darwin'
        working-directory: src-tauri
        shell: bash
        run: ./target/${{ matrix.target }}/release/aiterm-host${{ matrix.os == 'windows-latest' && '.exe' || '' }} --print-connection

      - name: Package
        working-directory: src-tauri
        shell: bash
        env:
          TAG: ${{ github.ref_name }}
        run: |
          set -euo pipefail
          VERSION="${TAG#v}"
          NAME="aiterm-host-${VERSION}-${{ matrix.target }}"
          BIN="aiterm-host${{ matrix.os == 'windows-latest' && '.exe' || '' }}"
          mkdir -p "dist/$NAME"
          cp "target/${{ matrix.target }}/release/$BIN" "dist/$NAME/"
          cp ../README.md "dist/$NAME/" 2>/dev/null || true
          cd dist
          if [ "${{ matrix.archive }}" = "zip" ]; then
            7z a "$NAME.zip" "$NAME" >/dev/null
          else
            tar czf "$NAME.tar.gz" "$NAME"
          fi
          ls -la

      - name: Upload to the draft release
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
          TAG: ${{ github.ref_name }}
        shell: bash
        run: |
          set -euo pipefail
          VERSION="${TAG#v}"
          NAME="aiterm-host-${VERSION}-${{ matrix.target }}"
          cd src-tauri/dist
          gh release upload "$TAG" "$NAME.${{ matrix.archive }}" --clobber
```

**`gh release upload "$TAG"` 在 draft 上能不能用**：可以——draft release 雖然沒有
真正的 tag ref，但 `gh release upload` 是以 tag 名稱查詢 release 的，`create-release`
建的那個 draft 帶著同一個 tag 名稱。`build` job 走的是 `tauri-action` 的 `releaseId`
路徑，跟這裡不同，但兩者上傳到同一個 release。

- [ ] **Step 2: checksum job**

`cli-build` 之後加：

```yaml
  # 所有執行檔上傳完之後，統一產一份 SHA256SUMS。
  #
  # 刻意獨立一個 job 而不是在每個平台各產一份：五份各自的 .sha256 檔案沒辦法讓
  # 使用者一次驗完，而安裝腳本要的是「一份清單、一次下載、比對其中一行」。
  cli-checksums:
    needs: cli-build
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - name: Download every CLI asset and checksum them
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
          TAG: ${{ github.ref_name }}
        run: |
          set -euo pipefail
          VERSION="${TAG#v}"
          mkdir sums && cd sums
          gh release download "$TAG" --pattern "aiterm-host-${VERSION}-*" --dir .
          # **把 SHA256SUMS 自己排除掉。** 這個 job 重跑時（例如某個平台
          # 重試）release 上已經有一份舊的 SHA256SUMS，而它也符合上面的
          # pattern——不排除的話會把自己的雜湊算進新的清單裡，安裝腳本用
          # grep 找檔名時就會撈到兩行，`sha256sum -c` 直接失敗。
          rm -f "aiterm-host-${VERSION}-SHA256SUMS"
          # 排序讓輸出穩定，方便人工比對兩次發版的差異。
          sha256sum aiterm-host-* | sort -k2 > "aiterm-host-${VERSION}-SHA256SUMS"
          cat "aiterm-host-${VERSION}-SHA256SUMS"
          gh release upload "$TAG" "aiterm-host-${VERSION}-SHA256SUMS" --clobber
```

- [ ] **Step 3: finalize 要等 CLI**

`finalize` 目前是 `needs: build`。改成：

```yaml
  finalize:
    needs: [build, cli-build, cli-checksums]
```

**這一行漏掉的後果**：`finalize` 會在 CLI 的東西上傳完之前就
`gh release edit --draft=false`，使用者看到的是一個缺了 CLI 執行檔的 release，而且
CI 全綠、沒有任何錯誤。

- [ ] **Step 4: 靜態檢查 workflow 語法**

```bash
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('YAML ok')"
```

若 repo 裡有 `scripts/check_ci_matrix.py`，也跑一次：

```bash
python3 scripts/check_ci_matrix.py
```

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: 發版時一併產出 aiterm-host 的五個平台執行檔與 SHA256SUMS"
```

---

## Task 4: 安裝腳本

**Files:**
- Create: `scripts/install.sh`
- Create: `scripts/install.ps1`
- Create: `scripts/test_install_sh.py`

- [ ] **Step 1: 寫失敗測試**

`scripts/test_install_sh.py`（照 `scripts/test_release_notes.py` 的既有模式，會被
`python3 -m unittest discover -s scripts -p 'test_*.py'` 自動跑到）：

```python
"""install.sh 的目標三元組推導測試。

平台偵測寫錯的後果特別難察覺：使用者會下載到一個給別的架構用的執行檔，錯誤訊息
是 "cannot execute binary file" 這種跟根因無關的東西。所以這一段值得單獨釘住。
"""
import subprocess
import unittest
from pathlib import Path

SCRIPT = Path(__file__).parent / "install.sh"


def detect(os_name: str, arch: str) -> str:
    """用假的 uname 值呼叫 install.sh 的 detect_target。"""
    result = subprocess.run(
        ["bash", "-c", f'source "{SCRIPT}" --source-only; detect_target "{os_name}" "{arch}"'],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip())
    return result.stdout.strip()


class DetectTarget(unittest.TestCase):
    def test_apple_silicon(self):
        self.assertEqual(detect("Darwin", "arm64"), "aarch64-apple-darwin")

    def test_intel_mac(self):
        self.assertEqual(detect("Darwin", "x86_64"), "x86_64-apple-darwin")

    def test_linux_x86_64_uses_musl(self):
        # 一律給 musl 靜態版：它在 glibc 系統上也跑得動，反過來不成立。
        self.assertEqual(detect("Linux", "x86_64"), "x86_64-unknown-linux-musl")

    def test_linux_arm64(self):
        self.assertEqual(detect("Linux", "aarch64"), "aarch64-unknown-linux-musl")

    def test_linux_arm64_reported_as_arm64(self):
        # 有些系統的 uname -m 回 arm64 而不是 aarch64。兩種都要接。
        self.assertEqual(detect("Linux", "arm64"), "aarch64-unknown-linux-musl")

    def test_unsupported_arch_is_an_error_not_a_guess(self):
        # 猜錯架構會讓使用者下載到跑不起來的執行檔，錯誤訊息還跟根因無關。
        # 寧可當場說「不支援」。
        with self.assertRaises(RuntimeError):
            detect("Linux", "riscv64")

    def test_unsupported_os_is_an_error(self):
        with self.assertRaises(RuntimeError):
            detect("FreeBSD", "x86_64")


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: 跑測試確認會紅**

```bash
python3 -m unittest scripts.test_install_sh -v
```

預期：FAIL——`scripts/install.sh` 還不存在。

- [ ] **Step 3: 寫 install.sh**

```bash
#!/usr/bin/env sh
# AITerm CLI Host 安裝腳本。
#
#   curl -fsSL https://raw.githubusercontent.com/jamesju9999/AITERM/master/scripts/install.sh | sh
#
# 做四件事：偵測平台 → 抓對應的 asset → 用 SHA256SUMS 驗 checksum → 裝進
# ~/.local/bin。**checksum 不通過就中止**，不給「要不要照樣裝」的選項——這是一個
# 會拿到 shell 的執行檔。
set -eu

REPO="jamesju9999/AITERM"
INSTALL_DIR="${AITERM_HOST_INSTALL_DIR:-$HOME/.local/bin}"

detect_target() {
  os="$1"
  arch="$2"
  case "$os" in
    Darwin)
      case "$arch" in
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) echo "不支援的 macOS 架構：$arch" >&2; return 1 ;;
      esac
      ;;
    Linux)
      # 一律給 musl 靜態版。它在 glibc 系統上也跑得動，反過來不成立——同一個
      # asset 就能涵蓋 alpine、distroless 與 glibc 太舊的老伺服器。
      case "$arch" in
        x86_64|amd64) echo "x86_64-unknown-linux-musl" ;;
        aarch64|arm64) echo "aarch64-unknown-linux-musl" ;;
        *) echo "不支援的 Linux 架構：$arch" >&2; return 1 ;;
      esac
      ;;
    *)
      echo "不支援的作業系統：$os（Windows 請用 install.ps1）" >&2
      return 1
      ;;
  esac
}

main() {
  target="$(detect_target "$(uname -s)" "$(uname -m)")"

  echo "正在查最新版本…"
  version="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -1)"
  if [ -z "$version" ]; then
    echo "查不到最新版本。GitHub API 可能限流了，或這個 repo 還沒有 release。" >&2
    exit 1
  fi
  echo "最新版本：$version"

  name="aiterm-host-${version}-${target}"
  base="https://github.com/$REPO/releases/download/v${version}"

  tmp="$(mktemp -d)"
  # 中途失敗也要清乾淨，不要在 /tmp 留一堆半殘的下載。
  trap 'rm -rf "$tmp"' EXIT

  echo "下載 ${name}.tar.gz…"
  curl -fsSL -o "$tmp/$name.tar.gz" "$base/$name.tar.gz"
  curl -fsSL -o "$tmp/SHA256SUMS" "$base/aiterm-host-${version}-SHA256SUMS"

  # macOS 沒有 sha256sum，那邊叫 shasum -a 256。兩個都找不到就中止——
  # 「驗不了就照樣裝」對一個會拿到 shell 的執行檔是不能接受的。
  if command -v sha256sum >/dev/null 2>&1; then
    sumcmd="sha256sum"
  elif command -v shasum >/dev/null 2>&1; then
    sumcmd="shasum -a 256"
  else
    echo "找不到 sha256sum 或 shasum，無法驗證下載內容——中止。" >&2
    exit 1
  fi

  echo "驗證 checksum…"
  ( cd "$tmp" && grep " $name.tar.gz\$" SHA256SUMS | $sumcmd -c - ) || {
    echo "checksum 不符——中止安裝。下載的檔案可能被竄改或損毀。" >&2
    exit 1
  }

  tar xzf "$tmp/$name.tar.gz" -C "$tmp"
  mkdir -p "$INSTALL_DIR"
  install -m 755 "$tmp/$name/aiterm-host" "$INSTALL_DIR/aiterm-host"

  echo "已安裝：$INSTALL_DIR/aiterm-host"
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "提醒：$INSTALL_DIR 不在 PATH 裡。請加上：export PATH=\"\$PATH:$INSTALL_DIR\"" ;;
  esac
  echo "下一步：aiterm-host --print-connection"
}

# 測試要能只載入函式而不執行安裝。
case "${1:-}" in
  --source-only) return 0 2>/dev/null || exit 0 ;;
esac

main "$@"
```

- [ ] **Step 4: 跑測試**

```bash
chmod +x scripts/install.sh
python3 -m unittest scripts.test_install_sh -v
```

預期：七條全 PASS。

- [ ] **Step 5: MUTATION CHECK**

把 `detect_target` 的 Linux x86_64 分支改成輸出 `aarch64-unknown-linux-musl`。
`test_linux_x86_64_uses_musl` 必須變紅。還原。

再把 `riscv64` 那條 `return 1` 改成 `echo "x86_64-unknown-linux-musl"`。
`test_unsupported_arch_is_an_error_not_a_guess` 必須變紅。還原。

回報哪些測試變紅。任一個突變沒讓測試變紅就大聲說出來。

- [ ] **Step 6: 寫 install.ps1**

```powershell
# AITerm CLI Host 安裝腳本（Windows）。
#
#   irm https://raw.githubusercontent.com/jamesju9999/AITERM/master/scripts/install.ps1 | iex
#
# 跟 install.sh 同樣的四步：偵測 → 下載 → 驗 checksum → 安裝。
# **checksum 不通過就中止**，不給略過的選項。
$ErrorActionPreference = "Stop"

$Repo = "jamesju9999/AITERM"
$InstallDir = if ($env:AITERM_HOST_INSTALL_DIR) { $env:AITERM_HOST_INSTALL_DIR } else { "$env:LOCALAPPDATA\Programs\aiterm-host" }

if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne "X64") {
    throw "目前只提供 x86_64 的 Windows 執行檔，偵測到：$([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture)"
}
$Target = "x86_64-pc-windows-msvc"

Write-Host "正在查最新版本…"
$release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
$version = $release.tag_name -replace '^v', ''
Write-Host "最新版本：$version"

$name = "aiterm-host-$version-$Target"
$base = "https://github.com/$Repo/releases/download/v$version"
$tmp = New-Item -ItemType Directory -Path (Join-Path $env:TEMP ([System.Guid]::NewGuid()))

try {
    Write-Host "下載 $name.zip…"
    Invoke-WebRequest "$base/$name.zip" -OutFile "$tmp\$name.zip"
    Invoke-WebRequest "$base/aiterm-host-$version-SHA256SUMS" -OutFile "$tmp\SHA256SUMS"

    Write-Host "驗證 checksum…"
    $expected = (Get-Content "$tmp\SHA256SUMS" | Where-Object { $_ -match [regex]::Escape("$name.zip") })
    if (-not $expected) { throw "SHA256SUMS 裡找不到 $name.zip 的項目——中止。" }
    $expectedHash = ($expected -split '\s+')[0].ToLower()
    $actualHash = (Get-FileHash "$tmp\$name.zip" -Algorithm SHA256).Hash.ToLower()
    if ($expectedHash -ne $actualHash) {
        throw "checksum 不符——中止安裝。下載的檔案可能被竄改或損毀。"
    }

    Expand-Archive "$tmp\$name.zip" -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item "$tmp\$name\aiterm-host.exe" "$InstallDir\aiterm-host.exe" -Force

    Write-Host "已安裝：$InstallDir\aiterm-host.exe"
    if ($env:PATH -notlike "*$InstallDir*") {
        Write-Host "提醒：$InstallDir 不在 PATH 裡。"
    }
    Write-Host "下一步：aiterm-host --print-connection"
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
```

- [ ] **Step 7: Commit**

```bash
git add scripts/install.sh scripts/install.ps1 scripts/test_install_sh.py
git commit -m "feat(dist): 加上 Unix 與 Windows 的安裝腳本，含強制的 checksum 驗證"
```

---

## Task 5: 容器映像（ghcr.io）

**⚠️ 這個任務的核心不是 Dockerfile，是那個煙霧測試。** 見開頭的陷阱說明：
alpine 只有 busybox 的 `/bin/sh`，而 OSC 133 只對 bash／zsh 注入。做出一個
「連得上但 AI 每步卡 60 秒」的映像是完全可能的，而且不會有任何錯誤訊息。

**Files:**
- Create: `docker/Dockerfile.host`
- Modify: `.github/workflows/release.yml`

- [ ] **Step 1: 寫 Dockerfile**

`docker/Dockerfile.host`：

```dockerfile
# AITerm CLI Host。
#
# 吃 CI 已經建好的 musl 靜態執行檔，不在容器裡編——編的話要拉整個 Rust 工具鏈，
# 而那份執行檔本來就已經是這個 release 的產物了。
FROM alpine:3.20

# **bash 不是可選的。** AITerm 的 shell integration（送出 OSC 133 標記，讓遠端
# AI 判斷每一步跑完沒）**只對路徑結尾是 bash 或 zsh 的 shell 注入**——見
# aiterm-core 的 pty/shell.rs。alpine 預設只有 busybox 的 /bin/sh，用它的話
# AI agent 迴圈的每一步都會退回 60 秒逾時，而且不會有任何錯誤訊息。
RUN apk add --no-cache bash ca-certificates
ENV SHELL=/bin/bash

ARG TARGETARCH
COPY dist/aiterm-host-${TARGETARCH} /usr/local/bin/aiterm-host
RUN chmod +x /usr/local/bin/aiterm-host

# 金鑰用環境變數給——容器裡的檔案系統通常是唯讀或用完即丟的。
# 不給的話 aiterm-host 會在 ~/.config 產生一組新的，而那組會隨容器消失。
ENV AITERM_HOST_KEY=""

EXPOSE 8022
ENTRYPOINT ["/usr/local/bin/aiterm-host"]
CMD ["--bind", "0.0.0.0", "--port", "8022"]
```

- [ ] **Step 2: 加 container job**

```yaml
  # 容器映像。吃 cli-build 已經上傳的 musl 靜態執行檔。
  cli-container:
    needs: cli-build
    runs-on: ubuntu-latest
    timeout-minutes: 20
    permissions:
      contents: read
      packages: write
    steps:
      - uses: actions/checkout@v4

      - name: Fetch the musl binaries from the draft release
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
          TAG: ${{ github.ref_name }}
        run: |
          set -euo pipefail
          VERSION="${TAG#v}"
          mkdir -p dist
          for pair in "x86_64-unknown-linux-musl:amd64" "aarch64-unknown-linux-musl:arm64"; do
            triple="${pair%%:*}"; arch="${pair##*:}"
            gh release download "$TAG" --pattern "aiterm-host-${VERSION}-${triple}.tar.gz" --dir .
            tar xzf "aiterm-host-${VERSION}-${triple}.tar.gz"
            cp "aiterm-host-${VERSION}-${triple}/aiterm-host" "dist/aiterm-host-${arch}"
          done
          ls -la dist

      - uses: docker/setup-qemu-action@v3
      - uses: docker/setup-buildx-action@v3

      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Build and load for smoke testing
        uses: docker/build-push-action@v6
        with:
          context: .
          file: docker/Dockerfile.host
          platforms: linux/amd64
          load: true
          tags: aiterm-host:smoke

      - name: 煙霧測試——**OSC 133 一定要出現**
        run: |
          set -euo pipefail
          # 先確認執行檔本身跑得動
          docker run --rm aiterm-host:smoke --print-connection

          # 再確認映像裡的 shell 真的會送 OSC 133 標記。沒有這一條的話，
          # 做出一個「連得上但 AI 每步卡 60 秒」的映像完全不會被發現。
          docker run --rm --entrypoint bash aiterm-host:smoke -c 'echo $SHELL' | grep -qx '/bin/bash' \
            || { echo "SHELL 沒有指向 bash——OSC 133 不會被注入"; exit 1; }
          docker run --rm --entrypoint sh aiterm-host:smoke -c 'test -x /bin/bash' \
            || { echo "映像裡沒有 bash"; exit 1; }

      - name: Build and push
        uses: docker/build-push-action@v6
        with:
          context: .
          file: docker/Dockerfile.host
          platforms: linux/amd64,linux/arm64
          push: true
          tags: |
            ghcr.io/${{ github.repository_owner }}/aiterm-host:${{ github.ref_name }}
            ghcr.io/${{ github.repository_owner }}/aiterm-host:latest
```

- [ ] **Step 3: finalize 也要等它**

```yaml
  finalize:
    needs: [build, cli-build, cli-checksums, cli-container]
```

- [ ] **Step 4: 本機先驗一次 Dockerfile（有 docker 的話）**

```bash
mkdir -p dist && cp src-tauri/target/release/aiterm-host dist/aiterm-host-arm64
docker build -f docker/Dockerfile.host --build-arg TARGETARCH=arm64 -t aiterm-host:local .
docker run --rm aiterm-host:local --print-connection
docker run --rm --entrypoint sh aiterm-host:local -c 'test -x /bin/bash && echo "bash 在"'
rm -rf dist
```

（本機的執行檔是 macOS 的，在 Linux 容器裡跑不起來——這一步只驗 Dockerfile 語法與
bash 是否存在，執行檔那一行預期會失敗。真正的驗證在 CI。若本機沒有 docker daemon，
整步跳過並在報告裡說明。）

- [ ] **Step 5: YAML 檢查與 Commit**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml')); print('YAML ok')"
git add docker/Dockerfile.host .github/workflows/release.yml
git commit -m "feat(dist): 發佈 aiterm-host 的多架構容器映像到 ghcr.io"
```

---

## Task 6: Homebrew tap

**Files:**
- Create: `scripts/bump_homebrew_formula.py`
- Create: `scripts/test_bump_homebrew_formula.py`
- Modify: `.github/workflows/release.yml`

**前置（使用者要先做，不是這個任務能代勞的）**：在 GitHub 上建一個公開 repo
`jamesju9999/homebrew-tap`，並產生一個有該 repo 寫入權限的 PAT，存成本 repo 的
secret `HOMEBREW_TAP_TOKEN`。**動工前先確認這兩樣都好了**，沒好就先做完 Task 7 的
其他部分，不要卡在這裡。

- [ ] **Step 1: 寫失敗測試**

`scripts/test_bump_homebrew_formula.py`：

```python
"""Homebrew formula 產生器的測試。

formula 寫錯的失效模式很安靜：`brew install` 會下載到錯的 URL 或裝到舊版，
而使用者只會覺得「怎麼裝起來不是新的」。
"""
import unittest

from bump_homebrew_formula import render_formula


SUMS = """\
aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111  aiterm-host-1.25.0-aarch64-apple-darwin.tar.gz
bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222  aiterm-host-1.25.0-x86_64-apple-darwin.tar.gz
cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333  aiterm-host-1.25.0-x86_64-unknown-linux-musl.tar.gz
dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444  aiterm-host-1.25.0-aarch64-unknown-linux-musl.tar.gz
eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555  aiterm-host-1.25.0-x86_64-pc-windows-msvc.zip
"""


class RenderFormula(unittest.TestCase):
    def setUp(self):
        self.out = render_formula("1.25.0", SUMS, "jamesju9999/AITERM")

    def test_version_appears(self):
        self.assertIn('version "1.25.0"', self.out)

    def test_each_platform_gets_its_own_sha(self):
        # 四個 sha 必須各就各位。貼錯位置的話 brew 會在下載後才驗出不符，
        # 錯誤訊息完全不會指向 formula。
        self.assertIn("aaaa1111", self.out)
        self.assertIn("bbbb2222", self.out)
        self.assertIn("cccc3333", self.out)
        self.assertIn("dddd4444", self.out)

    def test_the_windows_zip_is_not_included(self):
        # Homebrew 不裝 Windows 的東西。把它寫進去只會讓 formula 多一段死程式碼。
        self.assertNotIn("eeee5555", self.out)
        self.assertNotIn("windows", self.out)

    def test_arm_mac_sha_is_paired_with_the_arm_mac_url(self):
        # 這是真正會出錯的地方：四個 url 與四個 sha 配對錯位。只斷言「sha 有出現」
        # 抓不到對調——要斷言它跟正確的 url 在同一段裡。
        idx_url = self.out.index("aarch64-apple-darwin.tar.gz")
        idx_sha = self.out.index("aaaa1111")
        self.assertLess(
            abs(idx_sha - idx_url), 200,
            "arm mac 的 sha 沒有緊跟著它的 url——四組配對可能錯位了",
        )

    def test_a_missing_platform_is_an_error_not_a_silent_omission(self):
        partial = "\n".join(SUMS.splitlines()[:2]) + "\n"
        with self.assertRaises(KeyError):
            render_formula("1.25.0", partial, "jamesju9999/AITERM")


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: 跑測試確認會紅**

```bash
python3 -m unittest scripts.test_bump_homebrew_formula -v
```

預期：FAIL（模組不存在）。

- [ ] **Step 3: 寫產生器**

`scripts/bump_homebrew_formula.py`：

```python
#!/usr/bin/env python3
"""從一份 SHA256SUMS 產生 aiterm-host 的 Homebrew formula。

用法：
    python3 scripts/bump_homebrew_formula.py <version> <sums-file> <repo> > aiterm-host.rb
"""
import sys

# formula 需要的四個平台。Windows 的 zip 刻意不列——Homebrew 不裝那個。
PLATFORMS = {
    "arm_mac": "aarch64-apple-darwin",
    "intel_mac": "x86_64-apple-darwin",
    "arm_linux": "aarch64-unknown-linux-musl",
    "intel_linux": "x86_64-unknown-linux-musl",
}


def parse_sums(sums_text: str) -> dict:
    """把 `<sha>  <filename>` 每行解析成 {filename: sha}。"""
    out = {}
    for line in sums_text.splitlines():
        line = line.strip()
        if not line:
            continue
        sha, _, name = line.partition("  ")
        out[name.strip()] = sha.strip()
    return out


def render_formula(version: str, sums_text: str, repo: str) -> str:
    sums = parse_sums(sums_text)

    def sha_for(triple: str) -> str:
        key = f"aiterm-host-{version}-{triple}.tar.gz"
        # KeyError 而不是預設值：少一個平台就該當場失敗。悄悄漏掉的話，
        # 那個平台的使用者會拿到一個裝不起來的 formula。
        return sums[key]

    base = f"https://github.com/{repo}/releases/download/v{version}"

    def block(triple: str) -> str:
        return (
            f'      url "{base}/aiterm-host-{version}-{triple}.tar.gz"\n'
            f'      sha256 "{sha_for(triple)}"\n'
        )

    return f'''# 這個檔案由 scripts/bump_homebrew_formula.py 產生，不要手改。
class AitermHost < Formula
  desc "Headless host that shares a shell to AITerm for AI-driven remote control"
  homepage "https://github.com/{repo}"
  version "{version}"
  license "Apache-2.0"

  on_macos do
    on_arm do
{block(PLATFORMS["arm_mac"])}    end
    on_intel do
{block(PLATFORMS["intel_mac"])}    end
  end

  on_linux do
    on_arm do
{block(PLATFORMS["arm_linux"])}    end
    on_intel do
{block(PLATFORMS["intel_linux"])}    end
  end

  def install
    bin.install "aiterm-host"
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/aiterm-host --version")
  end
end
'''


def main() -> int:
    if len(sys.argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    version, sums_path, repo = sys.argv[1], sys.argv[2], sys.argv[3]
    with open(sums_path, encoding="utf-8") as f:
        print(render_formula(version, f.read(), repo), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

注意 formula 的 `test do` 區塊裡 `#{bin}` 在 python f-string 裡要寫成 `#{{bin}}`。

- [ ] **Step 4: 跑測試**

```bash
python3 -m unittest scripts.test_bump_homebrew_formula -v
```

預期：五條全 PASS。

- [ ] **Step 5: MUTATION CHECK**

把 `PLATFORMS` 裡 `arm_mac` 與 `intel_mac` 的值對調。
`test_arm_mac_sha_is_paired_with_the_arm_mac_url` 必須變紅——這正是最容易犯、
而且最難察覺的錯。

再把 `sha_for` 的 `sums[key]` 改成 `sums.get(key, "")`。
`test_a_missing_platform_is_an_error_not_a_silent_omission` 必須變紅。

兩個都還原，回報哪些測試變紅。

- [ ] **Step 6: 加 tap 更新 job**

```yaml
  # 更新 Homebrew tap。
  #
  # **`needs: finalize` 不是 `cli-checksums`。** `finalize` 掛著
  # `environment: release-approval`，會停下來等人審核——那正是「這一版到底要不要
  # 發出去」的決定點。對外發佈是不可逆的（Homebrew 使用者一 `brew update` 就會
  # 拿到），所以必須等在人點頭之後，不能跟審核並行。
  #
  # 這個 job 失敗不該擋住 release：tap 是額外的便利管道，而 release 本身
  # （含 Releases 的執行檔與安裝腳本）已經是完整可用的。
  cli-homebrew:
    needs: finalize
    runs-on: ubuntu-latest
    timeout-minutes: 10
    continue-on-error: true
    steps:
      - uses: actions/checkout@v4

      - name: Render the formula
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
          TAG: ${{ github.ref_name }}
        run: |
          set -euo pipefail
          VERSION="${TAG#v}"
          gh release download "$TAG" --pattern "aiterm-host-${VERSION}-SHA256SUMS" --dir .
          python3 scripts/bump_homebrew_formula.py \
            "$VERSION" "aiterm-host-${VERSION}-SHA256SUMS" "${{ github.repository }}" \
            > aiterm-host.rb
          cat aiterm-host.rb

      - name: Push to the tap
        env:
          TAP_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}
          TAG: ${{ github.ref_name }}
        run: |
          set -euo pipefail
          if [ -z "${TAP_TOKEN:-}" ]; then
            echo "沒有設定 HOMEBREW_TAP_TOKEN，跳過 tap 更新。"
            echo "（secret 若設在 environment 而不是 repo 層，這裡會展開成空字串——"
            echo "  用 gh secret list 確認它在 repo 層。）"
            exit 0
          fi
          git clone "https://x-access-token:${TAP_TOKEN}@github.com/jamesju9999/homebrew-tap.git" tap
          mkdir -p tap/Formula
          cp aiterm-host.rb tap/Formula/aiterm-host.rb
          cd tap
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git add Formula/aiterm-host.rb
          git diff --cached --quiet && { echo "formula 沒有變化"; exit 0; }
          git commit -m "aiterm-host ${TAG#v}"
          git push
```

**空的 secret 會展開成空字串而不是報錯**——這個 repo 過去被這件事咬過（secret 設在
environment 而非 repo 層）。所以上面要明確檢查空字串並印出提示，不能假設它一定有值。

- [ ] **Step 7: Commit**

```bash
git add scripts/bump_homebrew_formula.py scripts/test_bump_homebrew_formula.py .github/workflows/release.yml
git commit -m "feat(dist): 發版時自動更新 Homebrew tap 的 formula"
```

---

## Task 7: npm 套件

套件名是 **`aiterm-host`**（`aiterm` 已被別人佔用，實測 `aiterm-host` 可用）。

**授權是 `Apache-2.0`，不是 MIT。** repo 根目錄的 `LICENSE` 是 Apache License 2.0
（`src-tauri/Cargo.toml` 的 `license` 是空字串、`package.json` 沒有這個欄位，但那不代表
沒有授權——`LICENSE` 檔才是作者實際選擇的）。這份計畫的初版寫成 MIT，Task 6 實作時
才發現。宣稱一個作者沒選過的授權是實質問題，不是字面問題。
走 esbuild 那套：一個入口套件用 `optionalDependencies` 指向五個分平台子套件，
npm 只會裝符合當前平台的那一個。

**Files:**
- Create: `npm/aiterm-host/package.json`
- Create: `npm/aiterm-host/bin/aiterm-host.js`
- Create: `npm/aiterm-host/lib/resolve.mjs`
- Create: `npm/aiterm-host/test/resolve.test.mjs`
- Create: `npm/build-packages.mjs`
- Modify: `.github/workflows/release.yml`

**前置（使用者要先做）**：註冊 npm 帳號並產生一個 automation token，存成本 repo 的
secret `NPM_TOKEN`。沒有的話這個 job 會安靜跳過（跟 Homebrew 同一個處理方式）。

- [ ] **Step 1: 寫失敗測試**

`npm/aiterm-host/test/resolve.test.mjs`：

```js
// 平台解析的測試。用 node 內建的 test runner，不拉任何相依——這個套件
// 本身要盡量輕，它只是一個下載器的殼。
//
// 執行：node --test npm/aiterm-host/test/*.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { packageForPlatform, binaryName } from "../lib/resolve.mjs";

test("Apple Silicon", () => {
  assert.equal(packageForPlatform("darwin", "arm64"), "aiterm-host-darwin-arm64");
});

test("Intel Mac", () => {
  assert.equal(packageForPlatform("darwin", "x64"), "aiterm-host-darwin-x64");
});

test("Linux x64", () => {
  assert.equal(packageForPlatform("linux", "x64"), "aiterm-host-linux-x64");
});

test("Linux arm64", () => {
  assert.equal(packageForPlatform("linux", "arm64"), "aiterm-host-linux-arm64");
});

test("Windows x64", () => {
  assert.equal(packageForPlatform("win32", "x64"), "aiterm-host-win32-x64");
});

test("不支援的平台要丟出可讀的錯誤，不是回 undefined", () => {
  // 回 undefined 的話，後面的 require 會噴一個跟根因無關的
  // "Cannot find module undefined"，使用者完全不知道發生什麼事。
  assert.throws(
    () => packageForPlatform("freebsd", "x64"),
    /freebsd.*x64/,
    "錯誤訊息要包含實際的平台與架構",
  );
});

test("Windows 上的執行檔名要帶 .exe", () => {
  assert.equal(binaryName("win32"), "aiterm-host.exe");
  assert.equal(binaryName("linux"), "aiterm-host");
  assert.equal(binaryName("darwin"), "aiterm-host");
});
```

- [ ] **Step 2: 跑測試確認會紅**

```bash
node --test npm/aiterm-host/test/*.test.mjs
```

預期：FAIL（`lib/resolve.mjs` 不存在）。

- [ ] **Step 3: 寫實作**

`npm/aiterm-host/lib/resolve.mjs`：

```js
const PACKAGES = {
  "darwin arm64": "aiterm-host-darwin-arm64",
  "darwin x64": "aiterm-host-darwin-x64",
  "linux x64": "aiterm-host-linux-x64",
  "linux arm64": "aiterm-host-linux-arm64",
  "win32 x64": "aiterm-host-win32-x64",
};

/** 這個平台該用哪個子套件。不支援就丟錯，不回 undefined。 */
export function packageForPlatform(platform, arch) {
  const name = PACKAGES[`${platform} ${arch}`];
  if (!name) {
    throw new Error(
      `aiterm-host 沒有提供 ${platform} ${arch} 的執行檔。` +
        `支援的平台：${Object.keys(PACKAGES).join(", ")}`,
    );
  }
  return name;
}

/** 執行檔的檔名。Windows 要帶 .exe。 */
export function binaryName(platform) {
  return platform === "win32" ? "aiterm-host.exe" : "aiterm-host";
}
```

`npm/aiterm-host/bin/aiterm-host.js`：

```js
#!/usr/bin/env node
// 挑出這個平台的執行檔並交棒給它。
//
// 用 spawnSync 而不是 exec：aiterm-host 是互動式的長時間行程，要把 stdio
// 直接接通，而且結束碼要原樣傳回去。
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { packageForPlatform, binaryName } from "../lib/resolve.mjs";

const require = createRequire(import.meta.url);
const pkg = packageForPlatform(process.platform, process.arch);

let binPath;
try {
  binPath = require.resolve(`${pkg}/bin/${binaryName(process.platform)}`);
} catch {
  // optionalDependencies 在某些情況下會被整批跳過（例如 --no-optional，
  // 或安裝當下網路壞掉）。這時錯誤訊息要直接說出怎麼修，而不是丟一個
  // 裸的 MODULE_NOT_FOUND。
  console.error(
    `找不到 ${pkg}。這通常是安裝時跳過了 optional dependencies。\n` +
      `請重裝：npm install -g aiterm-host --include=optional`,
  );
  process.exit(1);
}

const result = spawnSync(binPath, process.argv.slice(2), { stdio: "inherit" });
process.exit(result.status ?? 1);
```

`npm/aiterm-host/package.json`：

```json
{
  "name": "aiterm-host",
  "version": "0.0.0",
  "description": "Headless host that shares a shell to AITerm for AI-driven remote control",
  "type": "module",
  "bin": { "aiterm-host": "bin/aiterm-host.js" },
  "files": ["bin", "lib"],
  "license": "Apache-2.0",
  "repository": { "type": "git", "url": "git+https://github.com/jamesju9999/AITERM.git" },
  "optionalDependencies": {
    "aiterm-host-darwin-arm64": "0.0.0",
    "aiterm-host-darwin-x64": "0.0.0",
    "aiterm-host-linux-x64": "0.0.0",
    "aiterm-host-linux-arm64": "0.0.0",
    "aiterm-host-win32-x64": "0.0.0"
  }
}
```

`version` 與 `optionalDependencies` 的版本都寫 `0.0.0` 佔位，發版時由
`build-packages.mjs` 統一改寫——五處必須完全一致，手動維護遲早會漏一個。

- [ ] **Step 4: 跑測試**

```bash
node --test npm/aiterm-host/test/*.test.mjs
```

預期：七條全 PASS。

- [ ] **Step 5: MUTATION CHECK**

把 `packageForPlatform` 的 `throw` 改成 `return undefined`。
「不支援的平台要丟出可讀的錯誤」那條必須變紅。還原。

把 `binaryName` 的 `win32` 判斷拿掉（永遠回 `aiterm-host`）。
「Windows 上的執行檔名要帶 .exe」那條必須變紅。還原。

回報哪些測試變紅。

- [ ] **Step 6: 寫打包腳本**

`npm/build-packages.mjs`：

```js
#!/usr/bin/env node
// 從已下載的 release artifact 組出所有 npm 套件目錄。
//
// 用法：node npm/build-packages.mjs <version> <artifact-dir> <out-dir>
//
// <artifact-dir> 裡要有解開後的 aiterm-host-<version>-<triple>/ 目錄。
import fs from "node:fs";
import path from "node:path";

const [version, artifactDir, outDir] = process.argv.slice(2);
if (!version || !artifactDir || !outDir) {
  console.error("用法：node npm/build-packages.mjs <version> <artifact-dir> <out-dir>");
  process.exit(2);
}

const TARGETS = [
  { pkg: "aiterm-host-darwin-arm64", triple: "aarch64-apple-darwin", os: "darwin", cpu: "arm64", bin: "aiterm-host" },
  { pkg: "aiterm-host-darwin-x64", triple: "x86_64-apple-darwin", os: "darwin", cpu: "x64", bin: "aiterm-host" },
  { pkg: "aiterm-host-linux-x64", triple: "x86_64-unknown-linux-musl", os: "linux", cpu: "x64", bin: "aiterm-host" },
  { pkg: "aiterm-host-linux-arm64", triple: "aarch64-unknown-linux-musl", os: "linux", cpu: "arm64", bin: "aiterm-host" },
  { pkg: "aiterm-host-win32-x64", triple: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64", bin: "aiterm-host.exe" },
];

fs.mkdirSync(outDir, { recursive: true });

for (const t of TARGETS) {
  const dir = path.join(outDir, t.pkg);
  fs.mkdirSync(path.join(dir, "bin"), { recursive: true });

  const src = path.join(artifactDir, `aiterm-host-${version}-${t.triple}`, t.bin);
  if (!fs.existsSync(src)) {
    // 少一個平台就整個失敗。悄悄跳過的話，入口套件的 optionalDependencies
    // 會指向一個不存在的版本，那個平台的使用者安裝時才會爆。
    console.error(`找不到執行檔：${src}`);
    process.exit(1);
  }
  fs.copyFileSync(src, path.join(dir, "bin", t.bin));
  fs.chmodSync(path.join(dir, "bin", t.bin), 0o755);

  fs.writeFileSync(
    path.join(dir, "package.json"),
    JSON.stringify(
      {
        name: t.pkg,
        version,
        description: `aiterm-host binary for ${t.os} ${t.cpu}`,
        os: [t.os],
        cpu: [t.cpu],
        files: ["bin"],
        license: "Apache-2.0",
        repository: { type: "git", url: "git+https://github.com/jamesju9999/AITERM.git" },
      },
      null,
      2,
    ) + "\n",
  );
  console.log(`已組好 ${t.pkg}`);
}

// 入口套件：改寫版本，以及五個 optionalDependencies 的版本。
const entry = path.join(outDir, "aiterm-host");
fs.cpSync("npm/aiterm-host", entry, { recursive: true });
const pkgPath = path.join(entry, "package.json");
const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
pkg.version = version;
for (const t of TARGETS) pkg.optionalDependencies[t.pkg] = version;
fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
console.log(`已組好入口套件 aiterm-host@${version}`);
```

- [ ] **Step 7: 加 npm 發佈 job**

```yaml
  # npm。
  #
  # **`needs: finalize`，理由跟 Homebrew 同一個，而且這裡更嚴重**：npm 的
  # unpublish 有嚴格限制（超過 72 小時基本上收不回來），所以絕對不能在人還沒
  # 核准這一版之前就把它推出去。
  #
  # 跟 Homebrew 一樣 continue-on-error：這是額外的便利管道，壞掉不該讓整個
  # release 失敗。
  cli-npm:
    needs: finalize
    runs-on: ubuntu-latest
    timeout-minutes: 15
    continue-on-error: true
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          registry-url: https://registry.npmjs.org

      - name: Download and unpack every CLI artifact
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
          TAG: ${{ github.ref_name }}
        run: |
          set -euo pipefail
          VERSION="${TAG#v}"
          mkdir -p artifacts && cd artifacts
          gh release download "$TAG" --pattern "aiterm-host-${VERSION}-*" --dir .
          for f in *.tar.gz; do tar xzf "$f"; done
          for f in *.zip; do unzip -q "$f"; done
          ls -la

      - name: Build the packages
        env:
          TAG: ${{ github.ref_name }}
        run: node npm/build-packages.mjs "${TAG#v}" artifacts npm-dist

      - name: Publish
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
          TAG: ${{ github.ref_name }}
        run: |
          set -euo pipefail
          if [ -z "${NODE_AUTH_TOKEN:-}" ]; then
            echo "沒有設定 NPM_TOKEN，跳過發佈。"
            echo "（secret 若設在 environment 而不是 repo 層，這裡會展開成空字串——"
            echo "  用 gh secret list 確認它在 repo 層。）"
            exit 0
          fi
          # **子套件一定要先發。** 入口套件的 optionalDependencies 指向它們，
          # 順序反過來的話，入口套件發出去的瞬間指向的是還不存在的版本。
          for p in npm-dist/aiterm-host-*; do
            (cd "$p" && npm publish --access public)
          done
          (cd npm-dist/aiterm-host && npm publish --access public)
```

- [ ] **Step 8: Commit**

```bash
git add npm/ .github/workflows/release.yml
git commit -m "feat(dist): 用分平台子套件把 aiterm-host 發到 npm"
```

---

## Task 8: 文件與整體驗證

**Files:**
- Modify: `README.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: README 加安裝一節**

在 README 裡新增：

````markdown
## aiterm-host（遠端主機端）

在一台沒有桌面環境的伺服器上開一個 shell，讓桌面版 AITerm 連進去、用 AI 操作它。

### 安裝

```bash
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/jamesju9999/AITERM/master/scripts/install.sh | sh

# Homebrew
brew install jamesju9999/tap/aiterm-host

# npm（不想裝東西的話用 npx）
npx aiterm-host --print-connection

# 容器
docker run -it --rm -p 8022:8022 -e AITERM_HOST_KEY=<你的金鑰> \
  ghcr.io/jamesju9999/aiterm-host:latest
```

```powershell
# Windows
irm https://raw.githubusercontent.com/jamesju9999/AITERM/master/scripts/install.ps1 | iex
```

### 使用

```bash
aiterm-host --bind 0.0.0.0 --port 8022
```

會印出位址、埠與一組金鑰。在 AITerm 的「連線到遠端終端機」裡展開手動位址，
填入位址、埠與金鑰（短碼留空）即可。

金鑰存在 `~/.config/aiterm-host/key`，重啟不變——所以存下來的連線永遠有效。
跨網段時位址請填這台機器對觀看端可達的位址（Tailscale / VPN / SSH tunnel）。
````

- [ ] **Step 2: 跑全部既有測試**

```bash
python3 -m unittest discover -s scripts -p 'test_*.py' -v
node --test npm/aiterm-host/test/*.test.mjs
npx tsc -b
npm run test
cd src-tauri && cargo test --workspace
```

`cargo test` **一定要加 `--workspace`**：`src-tauri/Cargo.toml` 同時是 package 與
workspace root，bare `cargo test` 只會跑 `app`。

**已知的既有 flaky（兩邊各一類，都跟這份計畫無關）：**

- **Rust：PTY 資源耗盡。** 不是單一測試，是一整類——實測看過
  `pty::session::tests::last_exit_code_is_none_for_a_fresh_session` 與
  `pty::session::tests::marker_count_starts_at_zero_for_a_fresh_session`，後者的
  錯誤是 `openpty: Os { code: -6 }`。成因是整套並行跑時同時 spawn 大量 PTY 撞到
  系統上限，**不是測試邏輯問題**（單獨跑 25 次全過，而且這些檔案與 master 逐位元組
  相同）。頻率約每 12 次一次。
- **前端：** `MailView > refetch on tab reactivation > falls back to the first
  account when the selected one was removed`。

遇到就重跑一次確認。**但不要因為「反正它會偶爾紅」就忽略真的回歸**——只有出現在
上面這份清單裡的名字才算既有 flaky，其他任何紅燈都要當真。

- [ ] **Step 3: Commit**

```bash
git add README.md CHANGELOG.md
git commit -m "docs: 加上 aiterm-host 的安裝說明"
```

---

## Task 9: 用 pre-release tag 做一次真的發版演練

**這一步不能跳。** 上面所有東西都只在本機或靜態檢查過——整條管線只有真的跑一次
才知道對不對，而 release workflow 只有 tag 才會觸發。

- [ ] **Step 1: 先確認 secret 在 repo 層**

```bash
gh secret list
```

要看到 `HOMEBREW_TAP_TOKEN` 與 `NPM_TOKEN`（沒有的話那兩條會安靜跳過，是預期行為）。

**空的 secret 會展開成空字串而不是報錯**，而且這個 repo 過去被「secret 設在
environment 而不是 repo 層」咬過——錯誤訊息會誤導成別的問題。所以要用這個指令確認，
不要只看網頁 UI。

- [ ] **Step 2: 推一個 pre-release tag**

照這個 repo 既有的慣例，用 `v<版本>-<主題><n>` 的形式：

```bash
git tag v1.25.0-dist1
git push origin v1.25.0-dist1
```

**這一步要先問過使用者**——推 tag 會觸發完整的 release build。

- [ ] **Step 3: 看結果**

run 常常卡在 `waiting`，不要一直等。直接查 draft release 的 asset：

```bash
gh release view v1.25.0-dist1 --json assets --jq '.assets[].name'
```

預期看到六個 CLI 相關的檔案：五個平台的壓縮檔 + 一份 `SHA256SUMS`。

- [ ] **Step 4: 真的裝一次**

```bash
# 從 release 直接抓，驗證 asset 名稱與 checksum 對得起來
curl -fsSL https://raw.githubusercontent.com/jamesju9999/AITERM/master/scripts/install.sh | sh
aiterm-host --version
aiterm-host --print-connection
```

- [ ] **Step 5: 驗容器**

```bash
docker run --rm ghcr.io/jamesju9999/aiterm-host:v1.25.0-dist1 --print-connection
docker run --rm --entrypoint sh ghcr.io/jamesju9999/aiterm-host:v1.25.0-dist1 -c 'test -x /bin/bash && echo bash ok'
```

- [ ] **Step 6: 清掉演練用的 tag 與 release**

```bash
gh release delete v1.25.0-dist1 --yes
git push --delete origin v1.25.0-dist1
git tag -d v1.25.0-dist1
```

ghcr 上那個 tag 也要刪（在 GitHub 的 Packages 頁面，或 `gh api` 刪 package version）。

---

## 完成後

用 `superpowers:finishing-a-development-branch` 收尾。正式發版時照既有流程推
`v<版本>` tag——**但依這個 repo 的規矩，推 tag 前一定要先問過使用者**。
