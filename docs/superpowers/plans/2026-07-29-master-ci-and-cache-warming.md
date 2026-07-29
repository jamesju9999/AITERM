# master CI 與快取暖機 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在預設分支建立可被 tag 執行還原的 Rust 快取，並在推 tag 之前擋下型別與測試錯誤。

**Architecture:** 單一新檔 `.github/workflows/ci.yml`，內含兩個互不相干的 job：`test` 快速把關、`warm-cache` 以六個對齊 `release.yml` 的 leg 產生快取。不修改 `release.yml`。

**Tech Stack:** GitHub Actions、`dtolnay/rust-toolchain`、`swatinem/rust-cache`。

---

## 設計依據

規格：`docs/superpowers/specs/2026-07-29-master-ci-and-cache-warming-design.md`

實作者必須知道的既有事實（皆已查證，不要重新推導）：

- `release.yml` 的 Rust 快取在 v1.2.5 輸出 `No cache found.`，且三個版本的金鑰**完全相同**、只有 ref 不同。GitHub 的快取依 ref 隔離，而 `release.yml` 只在 tag push 觸發，因此結構上不可能命中。
- `swatinem/rust-cache` 一旦指定 `shared-key`，**不會**把 job 名稱納入金鑰。觀察到的金鑰 `v0-rust-linux-x64-appimage-Linux-x64-db7c195c-439e96dc` 中沒有 `build`，證實可跨 workflow 共用。
- `CARGO_INCREMENTAL=0` 由 `dtolnay/rust-toolchain` 自行寫入 `$GITHUB_ENV`，**不是** `release.yml` 設的（`release.yml` 完全沒有 `CARGO_*`）。使用相同 action 即得到相同環境。
- `tauri-build` **不驗證** `externalBin`。已實測：移走 `src-tauri/binaries/` 後 `cargo check` 仍 exit 0。因此不需要建 Java sidecar。
- 根 `tsconfig.json` 是 solution 檔（`"files": []`），`npx tsc --noEmit` 什麼都不檢查且永遠 exit 0。
- `npm run lint` 有約 181 個既有問題，不可當作關卡。

## File Structure

| 檔案 | 責任 |
|---|---|
| `.github/workflows/ci.yml`（新增） | `test` 與 `warm-cache` 兩個 job |
| `scripts/check_ci_matrix.py`（新增） | 比對 `ci.yml` 與 `release.yml` 的矩陣是否逐項對齊 |

`release.yml` **不修改**。

---

## Task 1: 建立 `ci.yml`

**Files:** Create `.github/workflows/ci.yml`

- [ ] **Step 1: 建立檔案**

完整內容如下：

```yaml
name: CI

# The release workflow only runs on tag pushes, and GitHub scopes caches by ref,
# so a cache written by one tag can never be read by the next. Caches written on
# the default branch are the only ones a tag run can restore — that is what
# warm-cache below exists to produce.
on:
  push:
    branches: [master]
  pull_request:
    branches: [master]
  # Caches are evicted after 7 days without access. Without this, a quiet week
  # loses everything warm-cache built.
  schedule:
    - cron: '0 3 * * 1'
  workflow_dispatch:

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  # Fast gate. Kept separate from warm-cache so a type error is reported in two
  # minutes rather than after a six-platform build.
  test:
    if: github.event_name != 'schedule'
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: npm

      - run: npm install

      # `tsc -b`, never `tsc --noEmit`: the root tsconfig.json is a solution file
      # ("files": []), so --noEmit type-checks nothing and always exits 0. That is
      # exactly how a type error reached CI and failed all six release legs of
      # v1.2.4. This line is the reason this job exists.
      - name: Type check
        run: npx tsc -b

      - name: Frontend tests
        run: npm run test

      - name: Release-notes helper tests
        run: python3 -m unittest discover -s scripts -p 'test_*.py'

  # Produces the caches that release.yml restores. Every field below is aligned
  # with release.yml's build matrix on purpose: swatinem/rust-cache derives its
  # key from the runner OS, the shared-key and the Cargo.lock hash, so any
  # mismatch silently yields a cache no release run will ever read.
  warm-cache:
    # A PR's cache would be scoped to the PR ref and unreadable by anything else.
    if: github.event_name != 'pull_request'
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: macos-latest
            target: aarch64-apple-darwin
            shared_key: mac
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            shared_key: windows
          - os: ubuntu-22.04
            target: x86_64-unknown-linux-gnu
            shared_key: linux-x64-appimage
          - os: ubuntu-24.04
            target: x86_64-unknown-linux-gnu
            shared_key: linux-x64-deb
          - os: ubuntu-22.04-arm
            target: aarch64-unknown-linux-gnu
            shared_key: linux-arm64-appimage
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
            shared_key: linux-arm64-deb
    runs-on: ${{ matrix.os }}
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: npm

      - run: npm install

      # tauri-build resolves frontendDist ("../dist"), so the directory has to
      # exist before cargo runs.
      - name: Build frontend
        run: npm run build

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - uses: swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
          shared-key: ${{ matrix.shared_key }}

      - name: Install Linux system dependencies
        if: startsWith(matrix.os, 'ubuntu')
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev \
            libappindicator3-dev \
            librsvg2-dev \
            patchelf \
            libssl-dev

      # --release, matching the profile `tauri build` uses. A debug target dir
      # would fill the cache with artifacts the release run cannot reuse.
      #
      # cargo build rather than the full tauri build: this skips bundling and
      # signing, which the cache does not need, while still compiling every
      # dependency — the part that costs 8 minutes.
      #
      # No Java sidecar: tauri-build does not verify externalBin (measured —
      # cargo check exits 0 with src-tauri/binaries/ moved away).
      - name: Build (release profile)
        working-directory: src-tauri
        run: cargo build --release --target ${{ matrix.target }}
```

- [ ] **Step 2: 驗證 YAML 可解析**

Run:
```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml')); print('yaml ok')"
```
Expected: `yaml ok`

- [ ] **Step 3: 確認 job 觸發條件**

Run:
```bash
python3 - <<'PY'
import yaml
wf = yaml.safe_load(open('.github/workflows/ci.yml'))
# PyYAML parses the bare key `on` as the boolean True.
trig = wf.get('on', wf.get(True))
print("triggers:", sorted(trig))
for name, job in wf['jobs'].items():
    print(f"{name}: runs-on={job.get('runs-on')} if={job.get('if')}")
PY
```

Expected: 四個觸發（`push`、`pull_request`、`schedule`、`workflow_dispatch`）；`test` 帶 `github.event_name != 'schedule'`；`warm-cache` 帶 `github.event_name != 'pull_request'`。

- [ ] **Step 4: 在本機跑 `test` job 的四條指令**

```bash
npm install
npx tsc -b
npm run test
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Expected: 全部乾淨。**不要**用 `npx tsc --noEmit`。**不要**跑 `npm run lint`。

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add a master gate and a cache-warming matrix"
```

---

## Task 2: 矩陣對齊的自動比對

**Files:** Create `scripts/check_ci_matrix.py`

六組三元組中任一組寫錯，該 leg 就永遠暖不到，而症狀是「發版時只有部分 leg 變快」——極易誤判為正常波動。肉眼比對不可靠，改用程式。

- [ ] **Step 1: 寫比對腳本**

建立 `scripts/check_ci_matrix.py`：

```python
"""Fail if ci.yml's warm-cache matrix drifts from release.yml's build matrix.

swatinem/rust-cache derives its key from the runner OS, the shared-key and the
Cargo.lock hash. If any of the six (os, target, shared-key) triples disagree,
that leg's cache is written under a key no release run will ever look up — and
the only symptom is that some legs stay slow, which reads as normal variance.
"""

import pathlib
import sys

import yaml

ROOT = pathlib.Path(__file__).resolve().parent.parent


def release_triples():
    wf = yaml.safe_load((ROOT / ".github/workflows/release.yml").read_text())
    return {
        (m["os"], m["rust_targets"], m["artifact_name"])
        for m in wf["jobs"]["build"]["strategy"]["matrix"]["include"]
    }


def ci_triples():
    wf = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
    return {
        (m["os"], m["target"], m["shared_key"])
        for m in wf["jobs"]["warm-cache"]["strategy"]["matrix"]["include"]
    }


def main():
    release = release_triples()
    ci = ci_triples()
    if release == ci:
        print(f"matrices aligned ({len(ci)} legs)")
        return 0
    for triple in sorted(release - ci):
        print(f"in release.yml but not ci.yml: {triple}", file=sys.stderr)
    for triple in sorted(ci - release):
        print(f"in ci.yml but not release.yml: {triple}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 2: 執行，確認通過**

Run: `python3 scripts/check_ci_matrix.py`
Expected: `matrices aligned (6 legs)`，exit 0。

- [ ] **Step 3: 驗證它真的會抓錯（mutation testing）**

暫時把 `ci.yml` 中 `linux-x64-deb` 的 `shared_key` 改成 `linux-x64-debs`，重跑腳本。

Expected: exit 1，且 stderr 同時列出缺少的與多出來的三元組。改回來後重跑應回到 exit 0。

**若腳本在此改動下仍 exit 0，代表比對邏輯無效**——停下來回報，不要繼續。

- [ ] **Step 4: 接進 `test` job**

在 `.github/workflows/ci.yml` 的 `test` job 中，於「Release-notes helper tests」之後加入：

```yaml
      # Guards the whole point of warm-cache: a drifted matrix produces caches
      # under keys no release run looks up, and the only symptom is that some
      # legs stay slow.
      - name: Check the cache matrix matches release.yml
        run: |
          pip install --quiet pyyaml
          python3 scripts/check_ci_matrix.py
```

`ubuntu-latest` 的 python3 不保證帶 PyYAML，故明確安裝。

- [ ] **Step 5: 重新驗證 YAML 與本機執行**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml')); print('yaml ok')"
python3 scripts/check_ci_matrix.py
```
Expected: `yaml ok`、`matrices aligned (6 legs)`

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/ci.yml scripts/check_ci_matrix.py
git commit -m "ci: fail the build when the cache matrix drifts from release.yml"
```

---

## Task 3: 推上 master 並觀察首次執行

**Files:** 無

- [ ] **Step 1: 推送**

```bash
git push origin master
```

- [ ] **Step 2: 觀察 `test`**

```bash
gh run list --workflow=ci.yml --limit 1 --json databaseId -q '.[0].databaseId'
```
取得 run id 後等待完成，確認 `test` 為 success 且耗時在數分鐘內。

- [ ] **Step 3: 觀察 `warm-cache` 首次執行**

六個 leg 應全部 success。**每個 leg 的 rust-cache 都會顯示 `No cache found.`——這是預期行為**，master 上尚無快取可還原。

- [ ] **Step 4: 確認快取已寫入 master 的 ref**

```bash
gh cache list --limit 40 --json key,ref -q '.[] | select(.ref | contains("master")) | "\(.ref)\t\(.key)"' | sort
```

Expected: 六筆 `v0-rust-<shared-key>-...`，ref 指向 master。

**若一筆都沒有，代表快取未寫入**，後續全部無意義——停下來查明原因。

- [ ] **Step 5: 第二次推送以確認 master 內部命中**

在 master 上做一次無關緊要的提交（例如更新文件）並推送，確認 `warm-cache` 這次出現 `Restored from cache`（而非 `No cache found.`），且建置步驟明顯縮短。

這證明快取本身可用。**它仍不證明 tag 執行讀得到**——那是下一項。

---

## Task 4: 下次發版時驗證核心假設

**Files:** 無（實機驗證）

**tag 執行能否還原 master 建立的快取，是本設計唯一未經實證的假設。**

- [ ] **Step 1: 發版時檢查 log**

下次推 tag 後，取任一 build leg 的 log：

```bash
gh run view --job <JOB_ID> --log | grep -F -e "No cache found" -e "Restored from cache"
```

- **出現 `Restored from cache`** → 假設成立，暖機有效。
- **仍是 `No cache found.`** → 假設不成立。此時 `warm-cache` 對發版毫無幫助，應改為只保留 `test` job，並在 spec 中記錄該結論。

- [ ] **Step 2: 比對耗時**

```bash
gh api "repos/jamesju9999/AITERM/actions/jobs/<JOB_ID>" -q '.steps[] | select(.name|startswith("Build")) | "\(.started_at) \(.completed_at)"'
```

基準：v1.2.5 的 Linux x64 AppImage leg 建置步驟為 **508 秒**。命中後應明顯低於此值。

- [ ] **Step 3: 記錄結果**

在 `docs/superpowers/specs/2026-07-29-master-ci-and-cache-warming-design.md` 末尾附上「驗證結果」章節，寫明假設成立與否、實測耗時，以及**明確列出未驗證的項目**。

- [ ] **Step 4: 假設成立時才做的後續優化**

僅在 Step 1 確認命中後，才考慮在 `release.yml` 的 rust-cache 加上 `save-if: false`，停止每次發版寫入 3.8 GB 永遠無人讀取的快取。

**假設不成立時不可加**——那會讓「同一個 tag 重跑」失去唯一的快取好處。

---

## Notes for the implementer

- **不要修改 `release.yml`。** 本計畫刻意不動它，理由見 Task 4 Step 4。
- **不要用 `npx tsc --noEmit`。** 根 `tsconfig.json` 是 solution 檔，該指令永遠 exit 0。用 `npx tsc -b`。
- **不要跑 `npm run lint`** 作為關卡——約 181 個既有問題與此無關。
- **不要推 tag。**
- **`docs/` 被 gitignore**（`.gitignore:47`），但 specs 與 plans 是追蹤的。用 `git add -f`。
- 首次 `warm-cache` 執行顯示 `No cache found.` 是正確的，不是失敗。
