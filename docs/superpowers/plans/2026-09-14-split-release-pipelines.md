# 桌面版與 aiterm-host 發佈流程分離 — 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 `aiterm`（桌面版）與 `aiterm-host`（CLI）各自用自己的 tag、版本號、
changelog 與 workflow 發佈，互不牽動。

**Architecture:** 拆成兩個 workflow 檔案——`release.yml` 收 `v*`（桌面）、
`release-host.yml` 收 `host-v*`（host）。兩者共用的「建立或重用 draft release」
邏輯抽成 `.github/actions/create-draft-release` 複合 action。
`releases/latest` 明確歸屬桌面版，host 的安裝腳本改成自己挑最新的 `host-v*` release。

**Tech Stack:** GitHub Actions（YAML、composite action、`actions/github-script`）、
POSIX sh、PowerShell、Python 3 `unittest`、Cargo。

**Spec:** `docs/superpowers/specs/2026-09-14-split-release-pipelines-design.md`

---

## 檔案結構

| 檔案 | 職責 |
|---|---|
| `src-tauri/crates/aiterm-host/Cargo.toml` | host 版本的**唯一真實來源**（改成 `0.2.0`）。CI 只驗證、不再改寫。 |
| `CHANGELOG-host.md`（新增） | host 的使用者可見更新項目。餵給 `release-host.yml`。 |
| `CHANGELOG.md` | 只留桌面版。host 條目搬走。 |
| `scripts/host_tag.py`（新增） | 從 `host-v*` tag 推導版本號、判斷是否為彩排版。**單一來源**，讓三條對外管道的 guard 不會各寫一份而漂移。 |
| `scripts/test_host_tag.py`（新增） | 上者的測試。 |
| `scripts/install.sh` | 新增 `pick_host_version`，改為挑最新的正式 `host-v*` release。 |
| `scripts/test_install_sh.py` | 補 `pick_host_version` 的測試。 |
| `scripts/install.ps1` | 同 `install.sh` 的改動，用 PowerShell 的 JSON 物件。 |
| `scripts/bump_homebrew_formula.py` | formula 的下載網址改用 `host-v` 前綴。 |
| `scripts/test_bump_homebrew_formula.py` | 補一條測試釘住下載網址。 |
| `.github/actions/create-draft-release/action.yml`（新增） | 建立或重用 draft release，回傳 `release_id`。兩個 workflow 共用。 |
| `.github/workflows/release.yml` | 只管桌面版。移除 `cli-*` 五個 job、移除 host 版本改寫、`finalize` 改成 `needs: build`。 |
| `.github/workflows/release-host.yml`（新增） | 只管 host。 |

### 為什麼 guard 要抽成 `scripts/host_tag.py`

npm / Homebrew / 容器三條管道現在各自寫了一份

```bash
if [[ "$TAG" == *-* ]]; then skip=true; fi
```

`host-v0.2.0` 本身就含 `-`，照搬過去的話**每一次正式 host 發佈都會被判成彩排、
三條管道全部安靜跳過，而且 job 是綠的**。`release.yml` 的註解自己就寫過
「改一邊忘了另一邊，就會有一條管道把演練版發出去」。抽成一個被測試釘住的
helper，並在 `create-release` 算一次、當成 job output 給三個 job 用，
讓這個判斷只有一份、而且測得到。

---

## Task 1: host 版本號歸零到 0.2.0

**Files:**
- Modify: `src-tauri/crates/aiterm-host/Cargo.toml:3`

- [ ] **Step 1: 確認目前的值**

Run: `sed -n '1,5p' src-tauri/crates/aiterm-host/Cargo.toml`

Expected:
```
[package]
name = "aiterm-host"
version = "0.1.0"
edition = "2021"
rust-version = "1.88"
```

- [ ] **Step 2: 改成 0.2.0**

把 `src-tauri/crates/aiterm-host/Cargo.toml` 的第 3 行

```toml
version = "0.1.0"
```

改成

```toml
version = "0.2.0"
```

- [ ] **Step 3: 確認 workspace 還鎖得住**

Run: `cd src-tauri && cargo metadata --no-deps --format-version 1 | python3 -c "import json,sys;print([p['version'] for p in json.load(sys.stdin)['packages'] if p['name']=='aiterm-host'][0])"`
Expected: `0.2.0`

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/aiterm-host/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore(host): 版本改由 repo 決定，重新起算為 0.2.0"
```

若 `git status` 顯示 `src-tauri/Cargo.lock` 沒有變動，就只 add `Cargo.toml`。

---

## Task 2: 拆出 `CHANGELOG-host.md`

**Files:**
- Create: `CHANGELOG-host.md`
- Modify: `CHANGELOG.md`（移除 v1.25.1 整段、移除 v1.25.0 裡的 host 條目）

- [ ] **Step 1: 建立 `CHANGELOG-host.md`**

完整內容：

```markdown
# aiterm-host 變更記錄

這裡的文字會被 `.github/workflows/release-host.yml` 直接放進 GitHub Release 的
「更新項目」。**寫給使用者看，不是寫給開發者看**——講他們會遇到什麼、變成
怎樣，不要講模組名稱或函式名稱。

桌面版 AITerm 的變更記錄在 `CHANGELOG.md`，兩者的版本號從 aiterm-host 0.2.0
起各自獨立。

發版前在最上面新增一段 `## v<版本>`，內容就是那一版的更新項目。沒有對應段落
時，workflow 會退回用 commit 標題產生草稿（那份草稿長得像開發者日誌，通常需要
手動改寫）。

## v0.2.0

**版本號改成獨立計算，從 0.2.0 重新起算**
`aiterm-host` 之前跟著桌面版 AITerm 共用版本號，所以第一次公開就是 1.25.0——
那個數字是桌面版累積下來的，跟這個工具自己的成熟度沒有關係。從這一版起
`aiterm-host` 有自己的版本號與更新記錄，桌面版改版不會再推著它跳號，
反過來也一樣。你現在裝的如果是 1.25.x，直接裝 0.2.0 就是最新版。

**安裝腳本改成只找 `aiterm-host` 自己的版本**
以前安裝腳本抓的是整個專案「最新的一版」，桌面版發版之後那一版裡沒有
`aiterm-host` 的檔案，安裝就會失敗。現在腳本會明確去找最新的 `aiterm-host`
版本，桌面版怎麼發都不影響安裝。

**用 npm 或 Homebrew 安裝的人要注意**
`npx aiterm-host` 會自動拿到 0.2.0。如果你之前用 `npm install -g aiterm-host`
裝了 1.25.x，`npm update` 不會幫你換成版號比較小的 0.2.0，要手動
`npm install -g aiterm-host@latest`。
```

- [ ] **Step 2: 從 `CHANGELOG.md` 移除 v1.25.1 整段**

刪掉 `## v1.25.1` 那一行到 `## v1.25.0` 前一行為止的所有內容（含空行）。
那一整段都是 host 的修正，桌面版沒有變動——段落自己第一句就寫著這件事。

刪除後 `CHANGELOG.md` 的開頭應該是說明文字之後直接接 `## v1.25.0`。

- [ ] **Step 3: 從 `CHANGELOG.md` 的 v1.25.0 移除 host 條目**

在 `## v1.25.0` 段落裡刪掉這三塊（從 `**新功能：用 AITerm 的 AI 操作一台沒有桌面的機器**`
開始，到 `目前還不會記住你輸入過的金鑰，每次連線需要重新貼上——之後會改進。` 為止）：

1. `**新功能：用 AITerm 的 AI 操作一台沒有桌面的機器**` 及其內文
2. `**連線方式：一組固定的金鑰，不用每次唸驗證碼**` 及其內文
3. `**安裝方式**` 的清單、`安裝腳本會先核對…` 那段、以及 `目前還不會記住你輸入過的金鑰…` 那行

刪除後 `## v1.25.0` 的第一項應該是
`**新功能：工作看板可以選擇不要隔離到獨立分支**`。

- [ ] **Step 4: 確認 `release_notes.py` 在兩個檔案上都抓得到正確段落**

Run:
```bash
echo "" | python3 scripts/release_notes.py draft v0.2.0 CHANGELOG-host.md | head -3
```
Expected: 印出 `**版本號改成獨立計算，從 0.2.0 重新起算**` 開頭的段落，
stderr 沒有「沒有 v0.2.0 的段落」。

Run:
```bash
echo "" | python3 scripts/release_notes.py draft v1.25.0 CHANGELOG.md | grep -c "aiterm-host"
```
Expected: `0`（桌面版的段落裡不該再出現 host 的內容）

Run:
```bash
echo "" | python3 scripts/release_notes.py draft v1.25.1 CHANGELOG.md 2>&1 >/dev/null
```
Expected: stderr 出現 `CHANGELOG.md 沒有 v1.25.1 的段落`（該段已移走）

- [ ] **Step 5: Commit**

```bash
git add CHANGELOG.md CHANGELOG-host.md
git commit -m "docs: aiterm-host 的變更記錄獨立成 CHANGELOG-host.md"
```

---

## Task 3: `scripts/host_tag.py` — tag 推導與彩排版判斷（TDD）

**Files:**
- Create: `scripts/host_tag.py`
- Test: `scripts/test_host_tag.py`

- [ ] **Step 1: 先寫會紅的測試**

建立 `scripts/test_host_tag.py`：

```python
"""host-v* tag 的推導規則。

這一段值得單獨釘住，因為它的失敗是靜默的：npm／Homebrew／容器三條對外管道
都靠 is_prerelease 決定要不要發佈，判錯的結果是三條管道全部安靜跳過而 job
全綠——沒有任何訊號告訴你這一版根本沒發出去。
"""
import unittest

from host_tag import InvalidTag, is_prerelease, version_for_tag


class VersionForTag(unittest.TestCase):
    def test_strips_the_host_prefix(self):
        self.assertEqual(version_for_tag("host-v0.2.0"), "0.2.0")

    def test_keeps_the_rehearsal_suffix(self):
        # 彩排版的 asset 檔名要跟著帶後綴，否則兩次彩排會互相覆蓋。
        self.assertEqual(version_for_tag("host-v0.2.0-dist1"), "0.2.0-dist1")

    def test_a_desktop_tag_is_rejected(self):
        # 桌面版的 tag 誤觸發 host workflow 時要當場炸掉，不要安靜地發出一版
        # 版本號是 "1.26.0" 的 host。
        with self.assertRaises(InvalidTag):
            version_for_tag("v1.26.0")

    def test_a_bare_version_is_rejected(self):
        with self.assertRaises(InvalidTag):
            version_for_tag("0.2.0")


class IsPrerelease(unittest.TestCase):
    def test_a_release_tag_is_not_a_prerelease(self):
        # **這就是那個 bug。** 舊的判斷是 `[[ "$TAG" == *-* ]]`，而
        # "host-v0.2.0" 本身就含有 "-"，會讓每一次正式發佈都被跳過。
        self.assertFalse(is_prerelease("host-v0.2.0"))

    def test_a_rehearsal_tag_is_a_prerelease(self):
        self.assertTrue(is_prerelease("host-v0.2.0-dist1"))

    def test_a_semver_prerelease_is_a_prerelease(self):
        self.assertTrue(is_prerelease("host-v1.0.0-rc1"))

    def test_a_four_part_version_is_not_a_prerelease(self):
        # 版本號裡的點不是分隔符，不該被誤判。
        self.assertFalse(is_prerelease("host-v0.2.0.1"))


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: 跑測試確認它紅**

Run: `cd scripts && python3 -m unittest test_host_tag -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'host_tag'`

- [ ] **Step 3: 寫出最小實作**

建立 `scripts/host_tag.py`：

```python
"""host-v* tag 的推導規則，給 .github/workflows/release-host.yml 用。

抽成一個模組而不是在 workflow 裡各寫一份 shell 判斷：npm、Homebrew、容器
三條對外管道都要知道「這一版是不是彩排版」，三份複製品一旦漂移，就會有一條
管道把彩排版發給所有使用者（或反過來，把正式版全部跳過）。而且寫在 YAML 裡
的判斷沒有任何辦法在發版之前測到。
"""

import sys

PREFIX = "host-v"


class InvalidTag(Exception):
    """The tag is not a host release tag."""


def version_for_tag(tag):
    """host-v0.2.0 -> 0.2.0。不是 host tag 就丟 InvalidTag。"""
    if not tag.startswith(PREFIX):
        raise InvalidTag(f"{tag!r} 不是 host 的 release tag（要以 {PREFIX!r} 開頭）")
    version = tag[len(PREFIX):]
    if not version:
        raise InvalidTag(f"{tag!r} 的前綴後面沒有版本號")
    return version


def is_prerelease(tag):
    """彩排／預發版本嗎？

    判斷的是**版本號裡**有沒有 "-"，不是整個 tag 裡有沒有 "-"。tag 的前綴
    host-v 自己就帶一個 "-"，用整個 tag 判斷會讓每一次正式發佈都被當成彩排。
    """
    return "-" in version_for_tag(tag)


def main(argv):
    if len(argv) != 3 or argv[1] not in ("version", "is-prerelease"):
        print(
            "usage: host_tag.py version <tag>\n"
            "       host_tag.py is-prerelease <tag>",
            file=sys.stderr,
        )
        return 2
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    command, tag = argv[1], argv[2]
    try:
        if command == "version":
            print(version_for_tag(tag))
        else:
            # shell 端直接拿去跟字串 'true' 比對，所以印小寫。
            print("true" if is_prerelease(tag) else "false")
    except InvalidTag as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
```

- [ ] **Step 4: 跑測試確認它綠**

Run: `cd scripts && python3 -m unittest test_host_tag -v`
Expected: `Ran 8 tests` / `OK`

- [ ] **Step 5: 確認 CLI 介面（workflow 真正會呼叫的形式）**

Run:
```bash
python3 scripts/host_tag.py version host-v0.2.0
python3 scripts/host_tag.py is-prerelease host-v0.2.0
python3 scripts/host_tag.py is-prerelease host-v0.2.0-dist1
python3 scripts/host_tag.py version v1.26.0; echo "exit=$?"
```
Expected:
```
0.2.0
false
true
'v1.26.0' 不是 host 的 release tag（要以 'host-v' 開頭）
exit=1
```

- [ ] **Step 6: 確認整包 discover 也跑得到**

Run: `python3 -m unittest discover -s scripts -p 'test_*.py'`
Expected: `OK`（workflow 用的就是這一行）

- [ ] **Step 7: Commit**

```bash
git add scripts/host_tag.py scripts/test_host_tag.py
git commit -m "feat(release): host tag 推導與彩排版判斷抽成可測的 helper"
```

---

## Task 4: `install.sh` 改挑最新的正式 `host-v*` release（TDD）

**Files:**
- Modify: `scripts/install.sh:41-62`
- Test: `scripts/test_install_sh.py`

- [ ] **Step 1: 先寫會紅的測試**

在 `scripts/test_install_sh.py` 的 `detect()` 函式下方加入這個 helper
（放在 `class DetectTarget` 之前）：

```python
def pick_version(releases_json: str) -> str:
    """把 releases API 的 JSON 餵給 install.sh 的 pick_host_version。"""
    result = subprocess.run(
        ["bash", "-c", f'source "{SCRIPT}" --source-only; pick_host_version'],
        input=releases_json,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()
```

在檔案最後、`if __name__ == "__main__":` 之前加入：

```python
# 依 GitHub API 的順序：最新的在最前面。刻意把桌面版的 v1.26.0 放在第一個、
# 把彩排版 host-v0.2.1-dist1 放在正式版前面——只取「第一個 tag_name」或
# 只剝前綴不看後綴的實作都會在這份資料上拿到錯的答案。
RELEASES_JSON = """[
  {"tag_name": "v1.26.0", "draft": false, "prerelease": false},
  {"tag_name": "host-v0.2.1-dist1", "draft": false, "prerelease": true},
  {"tag_name": "host-v0.2.0", "draft": false, "prerelease": false},
  {"tag_name": "v1.25.1", "draft": false, "prerelease": false},
  {"tag_name": "host-v0.1.9", "draft": false, "prerelease": false}
]"""


class PickHostVersion(unittest.TestCase):
    def test_skips_desktop_releases(self):
        # 桌面版的 release 沒有 aiterm-host 的資產，抓到它的話下載會 404。
        self.assertEqual(pick_version(RELEASES_JSON), "0.2.0")

    def test_skips_rehearsal_tags(self):
        # host-v0.2.1-dist1 比 host-v0.2.0 新，但它是彩排版，不該裝給使用者。
        self.assertNotIn("dist1", pick_version(RELEASES_JSON))

    def test_takes_the_newest_not_just_any(self):
        # host-v0.1.9 也符合「正式 host 版本」，但它比較舊。
        self.assertNotEqual(pick_version(RELEASES_JSON), "0.1.9")

    def test_no_host_release_yields_nothing(self):
        only_desktop = '[{"tag_name": "v1.26.0", "draft": false, "prerelease": false}]'
        self.assertEqual(pick_version(only_desktop), "")


if __name__ == "__main__":
    unittest.main()
```

（原本檔案結尾的 `if __name__ == "__main__": unittest.main()` 只留一份。）

- [ ] **Step 2: 跑測試確認它紅**

Run: `cd scripts && python3 -m unittest test_install_sh -v`
Expected: FAIL — `PickHostVersion` 的四個測試全部拿到空字串
（`bash: pick_host_version: command not found`）

- [ ] **Step 3: 在 `install.sh` 加入 `pick_host_version`**

在 `scripts/install.sh` 的 `detect_target()` 結尾 `}` 之後、`main()` 之前插入：

```sh
# 從 /repos/{repo}/releases 的 JSON 裡挑出最新的正式 aiterm-host 版本號。
#
# 不能用 releases/latest：一個 repo 只有一個 latest，而它屬於桌面版 AITerm。
# 桌面版發一次版，latest 就是一則沒有任何 aiterm-host 資產的 release，
# 照著抓只會 404。
#
# 不解析 draft／prerelease 欄位，改用 tag 的形狀判斷：未登入的 API 本來就
# 看不到 draft，而這個 repo 的彩排 tag 一律是 host-v<版本>-<主題><n>，
# 所以「版本號只有數字和點」就等於「正式版」。grep 的樣式把結尾的引號一起
# 吃進去，host-v0.2.0-dist1 因此不會match——在 sh 裡手刻 JSON 解析比這脆弱得多。
#
# API 回傳的順序是新到舊，所以第一個符合的就是最新的正式版。
pick_host_version() {
  grep -o '"tag_name"[[:space:]]*:[[:space:]]*"host-v[0-9][0-9.]*"' \
    | head -1 \
    | sed 's/.*"host-v\([0-9][0-9.]*\)"$/\1/'
}
```

- [ ] **Step 4: 跑測試確認它綠**

Run: `cd scripts && python3 -m unittest test_install_sh -v`
Expected: `Ran 11 tests` / `OK`

- [ ] **Step 5: 改 `main()` 用新的查法**

把 `scripts/install.sh` 的 `main()` 裡這一段

```sh
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
```

換成

```sh
  echo "正在查最新版本…"
  version="$(curl -fsSL "https://api.github.com/repos/$REPO/releases?per_page=30" \
    | pick_host_version)"
  if [ -z "$version" ]; then
    echo "查不到 aiterm-host 的版本。GitHub API 可能限流了，或還沒有任何" >&2
    echo "host-v* 的正式 release。" >&2
    exit 1
  fi
  echo "最新版本：$version"

  name="aiterm-host-${version}-${target}"
  base="https://github.com/$REPO/releases/download/host-v${version}"
```

- [ ] **Step 6: 確認整個腳本的語法還通**

Run: `sh -n scripts/install.sh && bash -n scripts/install.sh && echo ok`
Expected: `ok`

- [ ] **Step 7: 對真的 API 做一次唯讀驗證**

Run:
```bash
curl -fsSL "https://api.github.com/repos/jamesju9999/AITERM/releases?per_page=30" \
  | (. scripts/install.sh --source-only; pick_host_version)
```
Expected: 現在還沒有 `host-v*` release，所以印出空行。這正是預期——
Task 9 發出 `host-v0.2.0` 之後同一條指令會印 `0.2.0`。

- [ ] **Step 8: Commit**

```bash
git add scripts/install.sh scripts/test_install_sh.py
git commit -m "fix(install): 安裝腳本改挑最新的 host-v* release，不再用 releases/latest"
```

---

## Task 5: `install.ps1` 做同樣的改動

**Files:**
- Modify: `scripts/install.ps1:28-34`

- [ ] **Step 1: 換掉版本查詢與下載基底**

把 `scripts/install.ps1` 的

```powershell
Write-Host "正在查最新版本…"
$release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
$version = $release.tag_name -replace '^v', ''
Write-Host "最新版本：$version"

$name = "aiterm-host-$version-$Target"
$base = "https://github.com/$Repo/releases/download/v$version"
```

換成

```powershell
# 不能用 releases/latest：一個 repo 只有一個 latest，而它屬於桌面版 AITerm。
# 桌面版發一次版，latest 就是一則沒有任何 aiterm-host 資產的 release。
# 這裡列出 releases（API 回傳新到舊），挑第一個正式的 host-v 版本。
# 正規表示式把結尾釘死（$），所以彩排版 host-v0.2.0-dist1 不會被選中。
Write-Host "正在查最新版本…"
$releases = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases?per_page=30"
$hostRelease = $releases |
    Where-Object { -not $_.draft -and -not $_.prerelease -and $_.tag_name -match '^host-v[0-9][0-9.]*$' } |
    Select-Object -First 1
if (-not $hostRelease) {
    throw "查不到 aiterm-host 的版本。GitHub API 可能限流了，或還沒有任何 host-v* 的正式 release。"
}
$version = $hostRelease.tag_name -replace '^host-v', ''
Write-Host "最新版本：$version"

$name = "aiterm-host-$version-$Target"
$base = "https://github.com/$Repo/releases/download/host-v$version"
```

- [ ] **Step 2: 確認它解析得過**

macOS 上如果有 `pwsh`：

Run:
```bash
pwsh -NoProfile -Command '$e=$null; [System.Management.Automation.Language.Parser]::ParseFile("'"$PWD"'/scripts/install.ps1", [ref]$null, [ref]$e) | Out-Null; if ($e.Count) { $e | % { $_.ToString() }; exit 1 }; "parses cleanly"'
```
Expected: `parses cleanly`

沒有 `pwsh` 就跳過——`release-host.yml` 的 `cli-build` 在 windows runner 上
有同一項檢查（`install.ps1 至少要解析得過`），會在彩排時把語法錯誤擋下來。

- [ ] **Step 3: Commit**

```bash
git add scripts/install.ps1
git commit -m "fix(install): Windows 安裝腳本改挑最新的 host-v* release"
```

---

## Task 5b: Homebrew formula 的下載網址要帶 `host-v`（TDD）

`scripts/bump_homebrew_formula.py:39` 寫死了 `releases/download/v{version}`。
tag 改成 `host-v0.2.0` 之後那個網址會 404，而且**沒有任何測試釘住它**——
`brew install` 會下載失敗，錯誤訊息跟 formula 完全扯不上關係。

**Files:**
- Modify: `scripts/bump_homebrew_formula.py:39`
- Test: `scripts/test_bump_homebrew_formula.py`

- [ ] **Step 1: 先寫會紅的測試**

在 `scripts/test_bump_homebrew_formula.py` 的 `class RenderFormula` 裡，
`test_version_appears` 下方加入：

```python
    def test_urls_point_at_the_host_tag(self):
        # tag 是 host-v1.25.0，不是 v1.25.0。少了前綴的話每個 url 都 404，
        # 而 brew 的錯誤訊息只會說下載失敗，不會指向 formula。
        self.assertIn(
            "https://github.com/jamesju9999/AITERM/releases/download/host-v1.25.0/",
            self.out,
        )
        self.assertNotIn("/releases/download/v1.25.0/", self.out)
```

- [ ] **Step 2: 跑測試確認它紅**

Run: `cd scripts && python3 -m unittest test_bump_homebrew_formula -v`
Expected: FAIL — `test_urls_point_at_the_host_tag`，
訊息是找不到 `.../download/host-v1.25.0/`

- [ ] **Step 3: 改下載基底**

把 `scripts/bump_homebrew_formula.py` 的

```python
    base = f"https://github.com/{repo}/releases/download/v{version}"
```

改成

```python
    # tag 前綴是 host-v：aiterm-host 與桌面版 AITerm 各有自己的 tag 系列。
    base = f"https://github.com/{repo}/releases/download/host-v{version}"
```

- [ ] **Step 4: 跑測試確認它綠**

Run: `cd scripts && python3 -m unittest test_bump_homebrew_formula -v`
Expected: `OK`

- [ ] **Step 5: Commit**

```bash
git add scripts/bump_homebrew_formula.py scripts/test_bump_homebrew_formula.py
git commit -m "fix(release): Homebrew formula 的下載網址改用 host-v tag"
```

---

## Task 6: 抽出 `create-draft-release` 複合 action（行為不變）

這一步刻意**不改變任何行為**，只把邏輯搬家，好讓它能單獨驗證。
`release.yml` 改用之後應該跟以前產出完全一樣的結果。

**Files:**
- Create: `.github/actions/create-draft-release/action.yml`
- Modify: `.github/workflows/release.yml:26-146`

- [ ] **Step 1: 建立複合 action**

建立 `.github/actions/create-draft-release/action.yml`：

```yaml
name: Create or reuse a draft release
description: >
  建立這個 tag 的 draft release，如果已經有一則就重用它，並回傳 release id。
  桌面版與 aiterm-host 兩條發佈線共用——這段邏輯是踩過重複 draft
  （repo 裡還留著 v0.1.77、v0.1.94 兩組）才長成現在這樣，複製第二份等於等它漂移。

inputs:
  tag:
    description: The tag to create or reuse the release for.
    required: true
  name:
    description: The release title.
    required: true
  body:
    description: The release body (markdown).
    required: true
  github-token:
    description: A token with contents:write.
    required: true

outputs:
  release_id:
    description: The numeric id of the draft (or existing) release.
    value: ${{ steps.create.outputs.result }}

runs:
  using: composite
  steps:
    - id: create
      uses: actions/github-script@v7
      env:
        TAG: ${{ inputs.tag }}
        RELEASE_NAME: ${{ inputs.name }}
        RELEASE_BODY: ${{ inputs.body }}
      with:
        github-token: ${{ inputs.github-token }}
        result-encoding: string
        script: |
          const tag = process.env.TAG;
          // A draft has no tag ref, so getReleaseByTag cannot see it — scan
          // instead. This also makes a re-run reuse the previous attempt's
          // draft rather than adding a second one.
          const releases = await github.paginate(github.rest.repos.listReleases, {
            owner: context.repo.owner,
            repo: context.repo.repo,
            per_page: 100,
          });
          const matches = releases.filter((r) => r.tag_name === tag);
          // Prefer a published release: `gh` resolves a tag via
          // /releases/tags/{tag}, which returns the published one, so picking a
          // leftover draft here would have the builds upload to one release
          // while finalize reads another.
          const existing = matches.find((r) => !r.draft) ?? matches[0];
          if (existing) {
            core.info(`Reusing release ${existing.id} (draft=${existing.draft})`);
            return String(existing.id);
          }
          const { data } = await github.rest.repos.createRelease({
            owner: context.repo.owner,
            repo: context.repo.repo,
            tag_name: tag,
            target_commitish: context.sha,
            name: process.env.RELEASE_NAME,
            body: process.env.RELEASE_BODY,
            draft: true,
            prerelease: false,
          });
          core.info(`Created draft release ${data.id}`);
          return String(data.id);
```

- [ ] **Step 2: 讓 `release.yml` 改用它**

在 `.github/workflows/release.yml` 裡，把 `create-release` job 的 `outputs`
從

```yaml
    outputs:
      release_id: ${{ steps.create.outputs.result }}
```

改成

```yaml
    outputs:
      release_id: ${{ steps.create.outputs.release_id }}
```

然後把整個 `- name: Create or reuse the draft release` 步驟
（`id: create`、`uses: actions/github-script@v7`、它的 `env:` 與 `with:` 全部）
換成下面這一段。原本寫在 `env.RELEASE_BODY` 的那份 markdown 原封不動搬進
`with.body`：

```yaml
      - name: Create or reuse the draft release
        id: create
        uses: ./.github/actions/create-draft-release
        with:
          tag: ${{ github.ref_name }}
          name: AITerm ${{ github.ref_name }}
          github-token: ${{ secrets.GITHUB_TOKEN }}
          body: |
            ## AITerm ${{ github.ref_name }}

            ### 更新項目
            <!-- changelog:start -->
            ${{ steps.changelog.outputs.list }}
            <!-- changelog:end -->

            ### 下載
            - **macOS** (Apple Silicon): 下載 `.dmg` 安裝檔
            - **Windows**: 下載 `-setup.exe` 安裝檔
            - **Linux x86_64 AppImage** (Ubuntu 22.04+): 下載 `_amd64.AppImage`
            - **Linux x86_64 .deb** (Ubuntu 24.04+): 下載 `_amd64.deb`
            - **Linux ARM64 AppImage** (Ubuntu 22.04+ ARM): 下載 `_aarch64.AppImage`
            - **Linux ARM64 .deb** (Ubuntu 24.04+ ARM): 下載 `_arm64.deb`

            > **macOS 首次開啟提示（僅限手動安裝 `.dmg`）：** 若出現「已損毀」或「無法驗證開發者」，請在「終端機」執行：
            > ```
            > xattr -cr /Applications/AITerm.app
            > ```
            > 之後改用 App 內建的自動更新升級時就不會再遇到——更新檔由 App 直接下載，不經瀏覽器，因此不會被加上 quarantine 屬性。
            > **Linux AppImage：** 執行前需加執行權限：
            > ```
            > chmod +x AITerm_*.AppImage && ./AITerm_*.AppImage
            > ```
```

- [ ] **Step 3: 確認兩個 YAML 都還 parse 得過**

Run:
```bash
python3 -c "
import yaml
for p in ['.github/workflows/release.yml', '.github/actions/create-draft-release/action.yml']:
    yaml.safe_load(open(p))
    print('ok', p)
"
```
Expected:
```
ok .github/workflows/release.yml
ok .github/actions/create-draft-release/action.yml
```

若 `yaml` 模組不存在，先 `python3 -m pip install --user pyyaml`。

- [ ] **Step 4: 確認 `release_id` 沒有被接錯**

Run: `grep -n "release_id\|steps.create.outputs" .github/workflows/release.yml`
Expected: `outputs.release_id` 指向 `steps.create.outputs.release_id`，
`build` job 裡引用的仍是 `needs.create-release.outputs.release_id`。

- [ ] **Step 5: Commit**

```bash
git add .github/actions/create-draft-release/action.yml .github/workflows/release.yml
git commit -m "refactor(ci): draft release 的建立邏輯抽成共用複合 action"
```

---

## Task 7: `release.yml` 只留桌面版

**Files:**
- Modify: `.github/workflows/release.yml`

- [ ] **Step 1: 把 host 相關的 job 完整複製出來備用**

Run:
```bash
sed -n '/^  cli-build:/,/^  finalize:/p' .github/workflows/release.yml > /tmp/cli-jobs.yml
sed -n '/^  cli-container-push:/,$p' .github/workflows/release.yml >> /tmp/cli-jobs.yml
wc -l /tmp/cli-jobs.yml
```
Expected: 檔案有 500 行上下。Task 8 會照著它改寫成 `release-host.yml`。

- [ ] **Step 2: 從 `release.yml` 刪掉五個 host job**

刪除這五個 job 的完整內容（含各自上方的註解區塊）：
`cli-build`、`cli-checksums`、`cli-container`、`cli-container-push`、
`cli-homebrew`、`cli-npm`。

（共六個 job——`cli-container` 與 `cli-container-push` 是分開的兩個。）

刪除後 `release.yml` 只剩三個 job：`create-release`、`build`、`finalize`。

- [ ] **Step 3: `finalize` 只等桌面版的 build**

把

```yaml
    needs: [build, cli-build, cli-checksums, cli-container]
```

改成

```yaml
    needs: build
```

- [ ] **Step 4: 刪掉改寫 host 版本的那一段**

在 `build` job 的 `Sync version from tag` 步驟裡，刪掉這一段
（`aiterm-host` 現在有自己的版本，由 `release-host.yml` 驗證）：

```javascript
          // aiterm-host 是獨立 crate，有自己的版本號。不同步的話發出去的執行檔
          // `--version` 會回報 0.1.0，跟 release 的 tag 對不起來。
          // 用 `^version = ` 搭配 m 旗標、不加 g：只換第一個 match，也就是
          // [package] 區塊那一行，不會誤傷任何相依項。
          const hostManifest = 'src-tauri/crates/aiterm-host/Cargo.toml';
          let host = fs.readFileSync(hostManifest, 'utf8');
          host = host.replace(/^version = ".*"/m, `version = "${version}"`);
          fs.writeFileSync(hostManifest, host);
          console.log(`aiterm-host version synced to ${version}`);
```

保留其後的 `console.log(\`Version synced to ${version}\`);`。

- [ ] **Step 5: 把 npm launcher 的測試步驟移走**

從 `create-release` job 刪掉整個 `- name: Test the npm launcher` 步驟——
那是 host 的東西，Task 8 會把它放進 `release-host.yml`。
保留 `- name: Test the release-notes helpers`（`release_notes.py` 兩邊都用）。

- [ ] **Step 6: 驗證結果**

Run:
```bash
python3 -c "
import yaml
w = yaml.safe_load(open('.github/workflows/release.yml'))
print(sorted(w['jobs']))
print('finalize needs:', w['jobs']['finalize']['needs'])
"
```
Expected:
```
['build', 'create-release', 'finalize']
finalize needs: build
```

Run: `grep -c "aiterm-host" .github/workflows/release.yml`
Expected: `0`

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "refactor(ci): release.yml 只負責桌面版，移除 aiterm-host 的所有 job"
```

---

## Task 8: 新增 `release-host.yml`

**Files:**
- Create: `.github/workflows/release-host.yml`

- [ ] **Step 1: 建立檔案**

完整內容：

```yaml
name: Release Host

# aiterm-host 有自己的 tag 前綴、版本號與變更記錄，與桌面版 AITerm 完全分開。
# 桌面版走 release.yml（tag v*），這裡走 host-v*。
#
# 兩者的相容性靠 aiterm-core 的 PROTOCOL_VERSION 在握手第一步互檢，不是靠
# 版本號相等——所以版本號分家在協定上是安全的。
on:
  push:
    tags:
      - 'host-v*'
  workflow_dispatch:

permissions:
  contents: write

jobs:
  create-release:
    runs-on: ubuntu-latest
    timeout-minutes: 10
    outputs:
      release_id: ${{ steps.create.outputs.release_id }}
      version: ${{ steps.ver.outputs.version }}
      is_prerelease: ${{ steps.ver.outputs.is_prerelease }}
    steps:
      # git log 要完整歷史與所有 tag 才解得出上一版。
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - uses: actions/setup-node@v4
        with:
          node-version: 20

      # 這些測試沒有別的 CI 關卡——這個 repo 沒有 PR 層級的 workflow。
      # 放在這裡表示 release_notes.py 或 host_tag.py 壞掉會在發版一開始就擋下來。
      - name: Test the release helpers
        run: python3 -m unittest discover -s scripts -p 'test_*.py'

      # npm 入口套件的平台解析。
      # 用 glob 讓 shell 展開，不要把目錄丟給 node：`node --test <目錄>` 在
      # Node 22 之後會把目錄當成模組路徑、直接 MODULE_NOT_FOUND。
      - name: Test the npm launcher
        run: node --test npm/aiterm-host/test/*.test.mjs

      # 版本與「是不是彩排版」只在這裡算一次，當成 job output 給後面所有 job 用。
      #
      # **絕對不要在各個 job 裡各寫一份 `[[ "$TAG" == *-* ]]`。** host-v0.2.0
      # 這個 tag 本身就含有 "-"，那種寫法會讓每一次正式發佈都被判成彩排，
      # npm／Homebrew／容器三條管道全部安靜跳過，而且每個 job 都是綠的。
      # scripts/host_tag.py 判斷的是版本號裡有沒有 "-"，而且有測試釘住。
      - name: Derive the version
        id: ver
        env:
          TAG: ${{ github.ref_name }}
        run: |
          set -euo pipefail
          VERSION=$(python3 scripts/host_tag.py version "$TAG")
          IS_PRE=$(python3 scripts/host_tag.py is-prerelease "$TAG")
          echo "version=$VERSION" >> "$GITHUB_OUTPUT"
          echo "is_prerelease=$IS_PRE" >> "$GITHUB_OUTPUT"
          echo "version=$VERSION is_prerelease=$IS_PRE"

      # 版本號的真實來源是 Cargo.toml，CI 只驗證、不改寫。改寫的話 repo 裡的
      # 值永遠是假的，發出去的執行檔 --version 跟原始碼對不起來。
      # 彩排 tag（0.2.0-dist1）比對的是 "-" 前面那一段。
      - name: The tag and Cargo.toml must agree
        env:
          VERSION: ${{ steps.ver.outputs.version }}
        run: |
          set -euo pipefail
          BASE="${VERSION%%-*}"
          MANIFEST=src-tauri/crates/aiterm-host/Cargo.toml
          IN_FILE=$(python3 -c "import re,sys;print(re.search(r'^version = \"(.*)\"', open(sys.argv[1], encoding='utf-8').read(), re.M).group(1))" "$MANIFEST")
          if [ "$IN_FILE" != "$BASE" ]; then
            echo "::error::$MANIFEST 寫的是 $IN_FILE，tag 要的是 $BASE。先改 Cargo.toml 再重打 tag。"
            exit 1
          fi
          echo "$MANIFEST = $IN_FILE，與 tag 相符"

      # 更新項目優先取 CHANGELOG-host.md 裡對應版本的段落——那份文字隨程式碼
      # 一起 review，不必每次發版在 GitHub 網頁編輯器裡重打。沒有對應段落時
      # 退回 commit 標題，發版不會因此被擋住。
      - name: Generate the changelog draft
        id: changelog
        env:
          TAG: ${{ github.ref_name }}
          VERSION: ${{ steps.ver.outputs.version }}
        run: |
          set -euo pipefail
          PREV=$(git describe --tags --abbrev=0 --match 'host-v*' "$TAG^" 2>/dev/null || true)
          RANGE=${PREV:+$PREV..}$TAG
          echo "range: $RANGE"
          {
            echo "list<<CHANGELOG_EOF"
            git log --pretty=format:%s "$RANGE" \
              | python3 scripts/release_notes.py draft "v$VERSION" CHANGELOG-host.md
            echo "CHANGELOG_EOF"
          } >> "$GITHUB_OUTPUT"

      - name: Create or reuse the draft release
        id: create
        uses: ./.github/actions/create-draft-release
        with:
          tag: ${{ github.ref_name }}
          name: AITerm Host ${{ steps.ver.outputs.version }}
          github-token: ${{ secrets.GITHUB_TOKEN }}
          body: |
            ## AITerm Host ${{ steps.ver.outputs.version }}

            `aiterm-host` 是 AITerm 的 headless 主控端：丟到雲端主機、容器或 NAS 上跑起來，
            就能從桌面版 AITerm 連進去操作它。桌面版 AITerm 的更新在
            [另一則 release](https://github.com/${{ github.repository }}/releases/latest)。

            ### 更新項目
            <!-- changelog:start -->
            ${{ steps.changelog.outputs.list }}
            <!-- changelog:end -->

            ### 安裝
            - **macOS / Linux**：`curl -fsSL https://raw.githubusercontent.com/${{ github.repository }}/master/scripts/install.sh | sh`
            - **Windows**：`irm https://raw.githubusercontent.com/${{ github.repository }}/master/scripts/install.ps1 | iex`
            - **Homebrew**：`brew install jamesju9999/tap/aiterm-host`
            - **npm**：`npx aiterm-host`
            - **容器**：`docker run --rm ghcr.io/${{ github.repository_owner }}/aiterm-host:${{ steps.ver.outputs.version }}`

            安裝腳本會自己核對 checksum，對不上就中止。

            ### 手動下載與驗證
            下載對應平台的壓縮檔與 `aiterm-host-${{ steps.ver.outputs.version }}-SHA256SUMS`，然後：
            ```
            sha256sum -c --ignore-missing aiterm-host-${{ steps.ver.outputs.version }}-SHA256SUMS
            ```
            macOS 沒有 `sha256sum`，用 `shasum -a 256 -c --ignore-missing` 代替。

            Linux 版是完全靜態的 musl 執行檔，Alpine、精簡容器與 glibc 較舊的伺服器都能直接跑。

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

      - name: Build
        working-directory: src-tauri
        run: cargo build -p aiterm-host --release --target ${{ matrix.target }}

      - name: 真的執行一次
        # 編得過不代表跑得動——musl 上 ring 的連結問題有時候要到執行才爆。
        # 五個目標裡只有 x86_64-apple-darwin 是交叉編譯（runner 是 arm mac），
        # 跳過它。
        if: matrix.target != 'x86_64-apple-darwin'
        working-directory: src-tauri
        shell: bash
        run: ./target/${{ matrix.target }}/release/aiterm-host${{ matrix.os == 'windows-latest' && '.exe' || '' }} --print-connection

      - name: install.ps1 至少要解析得過
        # install.ps1 是給 Windows 使用者 `irm ... | iex` 的，但寫它的機器上沒有
        # pwsh。這一格剛好跑在有 pwsh 的 runner 上，只做語法解析、不執行
        # （執行會去抓 releases 清單，那時這一版還是 draft）。
        if: matrix.os == 'windows-latest'
        shell: pwsh
        run: |
          $errors = $null
          [System.Management.Automation.Language.Parser]::ParseFile(
            "$PWD\scripts\install.ps1", [ref]$null, [ref]$errors) | Out-Null
          if ($errors.Count -gt 0) {
            $errors | ForEach-Object { Write-Error $_.ToString() }
            exit 1
          }
          Write-Host "install.ps1 parses cleanly"

      - name: Package
        working-directory: src-tauri
        shell: bash
        env:
          VERSION: ${{ needs.create-release.outputs.version }}
        run: |
          set -euo pipefail
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
          VERSION: ${{ needs.create-release.outputs.version }}
        shell: bash
        run: |
          set -euo pipefail
          NAME="aiterm-host-${VERSION}-${{ matrix.target }}"
          cd src-tauri/dist
          gh release upload "$TAG" "$NAME.${{ matrix.archive }}" --clobber

  # 所有執行檔上傳完之後，統一產一份 SHA256SUMS。
  #
  # 刻意獨立一個 job 而不是在每個平台各產一份：五份各自的 .sha256 檔案沒辦法讓
  # 使用者一次驗完，而安裝腳本要的是「一份清單、一次下載、比對其中一行」。
  cli-checksums:
    needs: [create-release, cli-build]
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - name: Download every CLI asset and checksum them
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
          TAG: ${{ github.ref_name }}
          VERSION: ${{ needs.create-release.outputs.version }}
        run: |
          set -euo pipefail
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

  # 容器映像——**只建置與煙霧測試，不推**。真正推到 ghcr 的是
  # cli-container-push，掛在 finalize 之後。一次彩排就把公開的 :latest 蓋掉
  # 是不能接受的。
  cli-container:
    needs: [create-release, cli-build]
    runs-on: ubuntu-latest
    timeout-minutes: 20
    # **一定要 contents: write，read 不夠。** 這個 job 要從 draft release 抓
    # 執行檔，而 draft release 只有具寫入權限的 token 看得到——contents: read
    # 的 token 查 draft 會拿到 404，gh 把它回報成 "release not found"。
    # 這是 v1.25.0-dist1 彩排時實際撞到的。
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4

      - name: Fetch the musl binaries from the draft release
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
          TAG: ${{ github.ref_name }}
          VERSION: ${{ needs.create-release.outputs.version }}
        run: |
          set -euo pipefail
          mkdir -p dist
          for pair in "x86_64-unknown-linux-musl:amd64" "aarch64-unknown-linux-musl:arm64"; do
            triple="${pair%%:*}"; arch="${pair##*:}"
            gh release download "$TAG" --pattern "aiterm-host-${VERSION}-${triple}.tar.gz" --dir .
            tar xzf "aiterm-host-${VERSION}-${triple}.tar.gz"
            cp "aiterm-host-${VERSION}-${triple}/aiterm-host" "dist/aiterm-host-${arch}"
          done
          ls -la dist

      - uses: docker/setup-buildx-action@v3

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
          docker run --rm aiterm-host:smoke --print-connection
          # 沒有這兩條的話，做出一個「連得上但 AI 每步卡 60 秒」的映像完全
          # 不會被發現。
          docker run --rm --entrypoint bash aiterm-host:smoke -c 'echo $SHELL' | grep -qx '/bin/bash' \
            || { echo "SHELL 沒有指向 bash——OSC 133 不會被注入"; exit 1; }
          docker run --rm --entrypoint sh aiterm-host:smoke -c 'test -x /bin/bash' \
            || { echo "映像裡沒有 bash"; exit 1; }
          # 沒給金鑰時要能自己產生一組並印出來，不能因為 Dockerfile 裡
          # `ENV AITERM_HOST_KEY=""` 這個空字串預設值而啟動失敗。
          docker run --rm aiterm-host:smoke --print-connection 2>&1 | grep -q "金鑰：" \
            || { echo "沒給金鑰時應該退回產生一組，而不是啟動失敗"; exit 1; }

  finalize:
    needs: [create-release, cli-build, cli-checksums, cli-container]
    runs-on: ubuntu-latest
    # 停在這裡等審核，那段時間就是人工改寫 release body 裡 changelog 區塊的窗口。
    # WARNING: 這個 environment 若不存在、或存在但沒有設必要審核者，GitHub 會
    # 直接跑掉而且完全不報錯。用
    # `gh api repos/<owner>/<repo>/environments/release-approval` 確認，不要看網頁。
    environment: release-approval
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4

      - name: Validate the changelog block and publish
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
          TAG: ${{ github.ref_name }}
        run: |
          set -euo pipefail
          # 現在才讀 body，不是在 create-release 時讀：上面那道審核關卡的意義
          # 就是中間有人改過它。
          gh release view "$TAG" --json body -q .body > body.md
          # 區塊缺漏或沒改寫時 exit 1，這個 job 就失敗、release 留在 draft。
          python3 scripts/release_notes.py extract < body.md > /dev/null
          # **不加 --latest。** 一個 repo 只有一個 releases/latest，它屬於桌面版
          # AITerm：tauri.conf.json 的更新器端點直接指向
          # releases/latest/download/latest.json。host 搶走它的話，每一個桌面
          # 使用者的自動更新都會去讀一則沒有 latest.json 的 release。
          # host 的安裝腳本不靠 latest，它自己去找最新的 host-v* release。
          gh release edit "$TAG" --draft=false
          echo "已發佈 $TAG（未指派 latest，latest 保留給桌面版）"

  # 以下三個對外管道都 needs: finalize——對外發佈是不可逆的，必須等人核准之後。
  # 三者都 continue-on-error：它們是額外的便利管道，壞掉不該讓整個 release 失敗
  # （Releases 上的執行檔與安裝腳本已經是完整可用的）。
  #
  # 彩排版的判斷一律讀 needs.create-release.outputs.is_prerelease，**不要**
  # 在這裡各寫一份 shell 判斷。

  cli-container-push:
    needs: [create-release, finalize]
    if: needs.create-release.outputs.is_prerelease != 'true'
    runs-on: ubuntu-latest
    timeout-minutes: 20
    continue-on-error: true
    permissions:
      contents: read
      packages: write
    steps:
      - uses: actions/checkout@v4

      - name: Fetch the musl binaries from the release
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
          TAG: ${{ github.ref_name }}
          VERSION: ${{ needs.create-release.outputs.version }}
        run: |
          set -euo pipefail
          mkdir -p dist
          for pair in "x86_64-unknown-linux-musl:amd64" "aarch64-unknown-linux-musl:arm64"; do
            triple="${pair%%:*}"; arch="${pair##*:}"
            gh release download "$TAG" --pattern "aiterm-host-${VERSION}-${triple}.tar.gz" --dir .
            tar xzf "aiterm-host-${VERSION}-${triple}.tar.gz"
            cp "aiterm-host-${VERSION}-${triple}/aiterm-host" "dist/aiterm-host-${arch}"
          done

      - uses: docker/setup-qemu-action@v3
      - uses: docker/setup-buildx-action@v3

      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      # 映像 tag 用純版本號，不帶 host-v 前綴——`docker pull ...:host-v0.2.0`
      # 讀起來像是 tag 打錯了。
      - name: Build and push
        uses: docker/build-push-action@v6
        with:
          context: .
          file: docker/Dockerfile.host
          platforms: linux/amd64,linux/arm64
          push: true
          tags: |
            ghcr.io/${{ github.repository_owner }}/aiterm-host:${{ needs.create-release.outputs.version }}
            ghcr.io/${{ github.repository_owner }}/aiterm-host:latest

  cli-homebrew:
    needs: [create-release, finalize]
    if: needs.create-release.outputs.is_prerelease != 'true'
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
          VERSION: ${{ needs.create-release.outputs.version }}
        run: |
          set -euo pipefail
          gh release download "$TAG" --pattern "aiterm-host-${VERSION}-SHA256SUMS" --dir .
          python3 scripts/bump_homebrew_formula.py \
            "$VERSION" "aiterm-host-${VERSION}-SHA256SUMS" "${{ github.repository }}" \
            > aiterm-host.rb
          cat aiterm-host.rb

      - name: Push to the tap
        env:
          TAP_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}
          VERSION: ${{ needs.create-release.outputs.version }}
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
          git commit -m "aiterm-host ${VERSION}"
          git push

  # npm 的 unpublish 有嚴格限制（超過 72 小時基本上收不回來），所以絕對不能在
  # 人還沒核准之前就推出去。
  cli-npm:
    needs: [create-release, finalize]
    if: needs.create-release.outputs.is_prerelease != 'true'
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
          VERSION: ${{ needs.create-release.outputs.version }}
        run: |
          set -euo pipefail
          mkdir -p artifacts && cd artifacts
          gh release download "$TAG" --pattern "aiterm-host-${VERSION}-*" --dir .
          for f in *.tar.gz; do tar xzf "$f"; done
          for f in *.zip; do unzip -q "$f"; done
          ls -la

      - name: Build the packages
        env:
          VERSION: ${{ needs.create-release.outputs.version }}
        run: node npm/build-packages.mjs "$VERSION" artifacts npm-dist

      - name: Publish
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
        run: |
          set -euo pipefail
          if [ -z "${NODE_AUTH_TOKEN:-}" ]; then
            echo "沒有設定 NPM_TOKEN，跳過發佈。"
            echo "（secret 若設在 environment 而不是 repo 層，這裡會展開成空字串——"
            echo "  用 gh secret list 確認它在 repo 層。）"
            exit 0
          fi
          # 已經發過的版本就跳過，讓這一步可以安全重跑。npm 的版本是不可變的，
          # 所以「已存在就跳過」不會掩蓋任何問題。
          publish_if_new() {
            local dir="$1"
            local name version
            name=$(node -p "require('./$dir/package.json').name")
            version=$(node -p "require('./$dir/package.json').version")
            if npm view "$name@$version" version >/dev/null 2>&1; then
              echo "$name@$version 已經在 registry 上，跳過。"
              return 0
            fi
            (cd "$dir" && npm publish --access public)
          }

          # **子套件一定要先發。** 入口套件的 optionalDependencies 指向它們，
          # 順序反過來的話，入口套件發出去的瞬間指向的是還不存在的版本。
          for p in npm-dist/aiterm-host-*; do
            publish_if_new "$p"
          done
          publish_if_new npm-dist/aiterm-host
```

- [ ] **Step 2: YAML 要 parse 得過，job 圖要正確**

Run:
```bash
python3 -c "
import yaml
w = yaml.safe_load(open('.github/workflows/release-host.yml'))
for name, job in w['jobs'].items():
    print(name, '<-', job.get('needs'), '| if:', job.get('if', '-'))
print('trigger:', w[True]['push']['tags'])
"
```
Expected:
```
create-release <- None | if: -
cli-build <- create-release | if: -
cli-checksums <- ['create-release', 'cli-build'] | if: -
cli-container <- ['create-release', 'cli-build'] | if: -
finalize <- ['create-release', 'cli-build', 'cli-checksums', 'cli-container'] | if: -
cli-container-push <- ['create-release', 'finalize'] | if: needs.create-release.outputs.is_prerelease != 'true'
cli-homebrew <- ['create-release', 'finalize'] | if: needs.create-release.outputs.is_prerelease != 'true'
cli-npm <- ['create-release', 'finalize'] | if: needs.create-release.outputs.is_prerelease != 'true'
trigger: ['host-v*']
```

（`w[True]` 不是筆誤：YAML 把沒有引號的 `on` 解析成布林 `True`。）

- [ ] **Step 3: 確認沒有任何殘留的舊 guard**

Run: `grep -n 'TAG" == \*-\*' .github/workflows/release-host.yml; echo "exit=$?"`
Expected: `exit=1`（grep 找不到東西＝沒有殘留的 `[[ "$TAG" == *-* ]]`）

- [ ] **Step 4: 確認沒有任何地方還在用 `${TAG#v}`**

Run: `grep -n 'TAG#v' .github/workflows/release-host.yml; echo "exit=$?"`
Expected: `exit=1`。版本一律來自 `needs.create-release.outputs.version`。

- [ ] **Step 5: 用 `act` 或 GitHub 的 lint 都做不到的部分，改用手動推演**

Run:
```bash
python3 - <<'PY'
# 把 host_tag.py 的判斷跟 workflow 的 if: 條件接起來走一遍，確認
# 正式版會發、彩排版會跳過。這是那個 silent bug 唯一測得到的地方。
import sys
sys.path.insert(0, 'scripts')
from host_tag import is_prerelease, version_for_tag

for tag in ("host-v0.2.0", "host-v0.2.0-dist1"):
    out = "true" if is_prerelease(tag) else "false"
    runs = out != "true"          # workflow 的 if: ... != 'true'
    print(f"{tag}: version={version_for_tag(tag)} is_prerelease={out} 三條管道會執行={runs}")
PY
```
Expected:
```
host-v0.2.0: version=0.2.0 is_prerelease=false 三條管道會執行=True
host-v0.2.0-dist1: version=0.2.0-dist1 is_prerelease=true 三條管道會執行=False
```

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/release-host.yml
git commit -m "feat(ci): aiterm-host 獨立的發佈 workflow（host-v* tag）"
```

---

## Task 9: 彩排 `host-v0.2.0-dist1`

**Files:** 無（只推 tag 與觀察）

> **推 tag 會觸發實際建置，必須先取得使用者同意。**
> 這一步開始之前先問過，不要自己推。

- [ ] **Step 1: 確認全部改動都推上 master**

Run: `git status --short && git log --oneline origin/master..HEAD`
Expected: 工作區乾淨、且沒有未推送的 commit（若有就先 `git push`）。

- [ ] **Step 2: 確認審核關卡真的存在**

Run: `gh api repos/jamesju9999/AITERM/environments/release-approval --jq '.protection_rules[].type'`
Expected: 輸出含 `required_reviewers`。若指令回 404，表示 environment 不存在，
`finalize` 會直接跑過去不等人——先在 repo 設定裡建立它再繼續。

- [ ] **Step 3: 推彩排 tag**

```bash
git tag host-v0.2.0-dist1
git push origin host-v0.2.0-dist1
```

- [ ] **Step 4: 確認桌面版的 workflow 沒有被觸發**

Run: `gh run list --limit 5 --json workflowName,headBranch,status`
Expected: 只有 `Release Host` 且 `headBranch` 是 `host-v0.2.0-dist1`，
**沒有** `Release`。

- [ ] **Step 5: 等建置完，檢查 draft release 的資產**

Run:
```bash
gh release view host-v0.2.0-dist1 --json isDraft,assets \
  --jq '.isDraft, (.assets[].name)'
```
Expected: `true`，然後是六個檔名——五個平台的壓縮檔加上一份 SHA256SUMS：
```
aiterm-host-0.2.0-dist1-aarch64-apple-darwin.tar.gz
aiterm-host-0.2.0-dist1-x86_64-apple-darwin.tar.gz
aiterm-host-0.2.0-dist1-x86_64-unknown-linux-musl.tar.gz
aiterm-host-0.2.0-dist1-aarch64-unknown-linux-musl.tar.gz
aiterm-host-0.2.0-dist1-x86_64-pc-windows-msvc.zip
aiterm-host-0.2.0-dist1-SHA256SUMS
```

- [ ] **Step 6: 核准 finalize，然後確認三條管道「確實被跳過」**

在 GitHub 網頁上核准 `release-approval`。等 run 結束後：

Run: `gh run list --workflow release-host.yml --limit 1 --json databaseId --jq '.[0].databaseId' | xargs -I{} gh run view {} --json jobs --jq '.jobs[] | "\(.name) \(.conclusion)"'`
Expected: `cli-container-push`、`cli-homebrew`、`cli-npm` 三個的 conclusion 是
`skipped`。

**這一項是整個計畫最重要的驗證。** 如果它們顯示 `success`，代表彩排版被
發到了 npm／Homebrew／ghcr，必須立刻停下來處理。

- [ ] **Step 7: 確認 `releases/latest` 還是桌面版**

Run: `gh api repos/jamesju9999/AITERM/releases/latest --jq .tag_name`
Expected: `v1.25.1`（或當時最新的桌面版 tag），**不是** `host-v0.2.0-dist1`。

- [ ] **Step 8: 刪掉彩排的 release 與 tag**

```bash
gh release delete host-v0.2.0-dist1 --yes --cleanup-tag
```

---

## Task 10: 正式發佈 `host-v0.2.0`

**Files:** 無

> **同樣需要先取得使用者同意才能推 tag。**

- [ ] **Step 1: 推正式 tag**

```bash
git tag host-v0.2.0
git push origin host-v0.2.0
```

- [ ] **Step 2: 確認 draft release 的更新項目來自 `CHANGELOG-host.md`**

Run: `gh release view host-v0.2.0 --json body --jq .body | head -20`
Expected: `<!-- changelog:start -->` 之後是 `**版本號改成獨立計算，從 0.2.0 重新起算**`，
不是一串 commit 標題。

- [ ] **Step 3: 核准 finalize，確認三條管道「確實沒有被跳過」**

Run: `gh run list --workflow release-host.yml --limit 1 --json databaseId --jq '.[0].databaseId' | xargs -I{} gh run view {} --json jobs --jq '.jobs[] | "\(.name) \(.conclusion)"'`
Expected: `cli-container-push`、`cli-homebrew`、`cli-npm` 三個都是 `success`
（`cli-homebrew` 可能因為 tap repo 不存在而失敗——那是既有的待辦，
`continue-on-error` 會讓它不擋住發版）。

彩排（Task 9 Step 6，三個 skipped）與正式版（這一步，三個 success）
**兩個方向都要驗過**。只驗一邊的話，前綴 bug 正好會通過。

- [ ] **Step 4: 確認 `releases/latest` 仍然是桌面版**

Run: `gh api repos/jamesju9999/AITERM/releases/latest --jq .tag_name`
Expected: 桌面版的 tag，不是 `host-v0.2.0`。

- [ ] **Step 5: 真的跑一次安裝腳本**

在 macOS 上：

```bash
AITERM_HOST_INSTALL_DIR=/tmp/aiterm-host-check sh -c "$(curl -fsSL https://raw.githubusercontent.com/jamesju9999/AITERM/master/scripts/install.sh)"
/tmp/aiterm-host-check/aiterm-host --version
```
Expected: 安裝過程印出「最新版本：0.2.0」、checksum 驗證通過，
`--version` 印出 `aiterm-host 0.2.0`。

- [ ] **Step 6: 確認 npm 拿到的是 0.2.0**

Run: `npm view aiterm-host version && npm view aiterm-host dist-tags`
Expected: `0.2.0`，且 `latest: '0.2.0'`。

- [ ] **Step 7: 確認容器映像的 tag**

Run: `docker run --rm ghcr.io/jamesju9999/aiterm-host:0.2.0 --print-connection | head -3`
Expected: 正常印出連線資訊（位址／埠／金鑰）。

---

## 完成後的狀態

- 桌面版：`git tag v1.26.0 && git push` → `release.yml` → 3 個 job 群
  （create-release / 6 個 build / finalize）→ `releases/latest` + `latest.json`
- host：`git tag host-v0.2.1 && git push` → `release-host.yml` → 5 個 job 群
  → npm / Homebrew / ghcr，不碰 `latest`
- 兩邊的更新項目分別寫在 `CHANGELOG.md` 與 `CHANGELOG-host.md`
- host 版本號的真實來源是 `src-tauri/crates/aiterm-host/Cargo.toml`，CI 驗證它
