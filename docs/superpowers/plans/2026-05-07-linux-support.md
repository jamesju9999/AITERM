# Linux Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Linux (x86_64) as a first-class build target — CI produces `.AppImage` and `.deb` packages, all core features (PTY, AI, FileExplorer, DB, Telegram) work out of the box; DB2 is excluded from Linux (same as current macOS-only scope upgrade path).

**Architecture:** Create `tauri.linux.conf.json` to exclude the DB2 sidecar on Linux (matching the macOS/Windows pattern). Add a `ubuntu-22.04` matrix entry to the CI that installs required WebKitGTK system dependencies, runs the existing version-sync step, and publishes AppImage + .deb to the same GitHub Release. Update docs and website to reflect Linux support.

**Tech Stack:** Tauri 2 (auto-merges `tauri.linux.conf.json`), GitHub Actions `ubuntu-22.04`, `apt` system deps (`libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `patchelf`, `libssl-dev`), Eclipse Temurin 21 JRE (Linux x64, for optional DB2 dev support).

---

## File Map

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/tauri.linux.conf.json` | **Create** | Exclude DB2 sidecar on Linux |
| `scripts/setup-db2-linux.sh` | **Create** | Download Linux x64 JRE + copy jar (dev only) |
| `.github/workflows/release.yml` | **Modify** | Add Ubuntu matrix, system deps, Linux build step |
| `README.md` | **Modify** | Add Linux to platform list + prerequisites |
| `aiterm-site/index.html` | **Modify** | Add Linux platform badge in hero section |

---

## Task 1: Linux Tauri Platform Config

**Files:**
- Create: `src-tauri/tauri.linux.conf.json`

Tauri 2 automatically merges `tauri.linux.conf.json` on Linux builds — no CLI flag needed.
On Linux we ship without DB2 (no JRE bundled), so we override `externalBin` to empty and set no DB2 resources.

- [ ] **Step 1: Create the Linux platform config**

```json
{
  "bundle": {
    "externalBin": []
  }
}
```

Save to `src-tauri/tauri.linux.conf.json`.

- [ ] **Step 2: Verify it parses correctly**

```bash
cd src-tauri
node -e "console.log(JSON.parse(require('fs').readFileSync('tauri.linux.conf.json','utf8')))"
```

Expected: prints the object without error.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tauri.linux.conf.json
git commit -m "feat(linux): add tauri.linux.conf.json — exclude DB2 sidecar"
```

---

## Task 2: DB2 Setup Script for Linux (Dev Use)

**Files:**
- Create: `scripts/setup-db2-linux.sh`

Mirrors `setup-db2-mac.sh` but targets `linux/x64` from the Adoptium API.
On Linux the JRE extracts directly as `jdk-21.*-jre/` (no `Contents/Home` wrapper).

- [ ] **Step 1: Create `scripts/setup-db2-linux.sh`**

```bash
#!/usr/bin/env bash
# Setup DB2 Java sidecar for Linux x86_64
# Run once from the workspace root: bash scripts/setup-db2-linux.sh

set -euo pipefail

DEST="src-tauri/binaries/db2-sidecar-linux-x64"

echo "==> Cleaning output directory: $DEST"
rm -rf "$DEST"
mkdir -p "$DEST"

echo "==> Building db2sidecar.jar via Maven..."
(cd db2-sidecar-java && mvn package -q --no-transfer-progress)
test -f "db2-sidecar-java/target/db2sidecar.jar" || {
  echo "ERROR: mvn package produced no jar at db2-sidecar-java/target/db2sidecar.jar"
  exit 1
}
cp "db2-sidecar-java/target/db2sidecar.jar" "$DEST/db2sidecar.jar"
echo "  Copied db2sidecar.jar"

echo "==> Downloading Eclipse Temurin 21 JRE (Linux x64)..."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

JRE_URL="https://api.adoptium.net/v3/binary/latest/21/ga/linux/x64/jre/hotspot/normal/eclipse"
curl -L --fail "$JRE_URL" -o "$TMP/jre.tar.gz"

echo "==> Extracting JRE..."
tar -xzf "$TMP/jre.tar.gz" -C "$TMP"

# On Linux, Temurin extracts as jdk-21.*-jre/ (no Contents/Home wrapper)
JRE_DIR=$(find "$TMP" -maxdepth 1 -name "jdk-*-jre" -type d | head -1)
if [[ -z "$JRE_DIR" ]]; then
  echo "ERROR: Could not locate extracted JRE directory in $TMP"
  ls "$TMP"
  exit 1
fi

mkdir -p "$DEST/jre"
cp -R "$JRE_DIR/." "$DEST/jre/"

echo "==> Verifying java binary..."
"$DEST/jre/bin/java" -version 2>&1 | head -1

echo ""
echo "Done. DB2 Java sidecar ready at: $DEST/"
echo "  db2sidecar.jar          (IBM JDBC fat jar)"
echo "  jre/bin/java            (Temurin 21 Linux x64)"
echo ""
echo "Run 'npm run tauri:dev' to start the app."
```

- [ ] **Step 2: Make it executable and commit**

```bash
chmod +x scripts/setup-db2-linux.sh
git add scripts/setup-db2-linux.sh
git commit -m "feat(linux): add setup-db2-linux.sh for dev DB2 support"
```

---

## Task 3: CI — Add Linux Build Matrix

**Files:**
- Modify: `.github/workflows/release.yml`

Add `ubuntu-22.04` to the build matrix. Before the Tauri build step, install the required WebKitGTK and system libraries. Share the same unified release body across all three platforms so the last job to finish doesn't overwrite a partial description.

- [ ] **Step 1: Add Linux matrix entry**

In `.github/workflows/release.yml`, replace the `matrix.include` block:

```yaml
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: macos-latest
            target: aarch64-apple-darwin
            rust_targets: aarch64-apple-darwin
            artifact_name: mac
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            rust_targets: x86_64-pc-windows-msvc
            artifact_name: windows
          - os: ubuntu-22.04
            target: x86_64-unknown-linux-gnu
            rust_targets: x86_64-unknown-linux-gnu
            artifact_name: linux
```

- [ ] **Step 2: Add Linux system dependencies step**

After the "Rust cache" step and before the "Setup Java 21" step, add:

```yaml
      # Linux: install WebKitGTK and other Tauri system dependencies
      - name: Install Linux system dependencies
        if: matrix.os == 'ubuntu-22.04'
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev \
            libappindicator3-dev \
            librsvg2-dev \
            patchelf \
            libssl-dev
```

- [ ] **Step 3: Add Linux build step**

After the existing "Build (Windows)" step, add:

```yaml
      - name: Build (Linux x64)
        if: matrix.os == 'ubuntu-22.04'
        uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          tagName: ${{ github.ref_name }}
          releaseName: AITerm ${{ github.ref_name }}
          releaseBody: |
            ## AITerm ${{ github.ref_name }}

            ### 下載
            - **macOS** (Apple Silicon): 下載 `.dmg` 安裝檔
            - **Windows**: 下載 `-setup.exe` 安裝檔
            - **Linux** (x86_64): 下載 `.AppImage`（免安裝）或 `.deb` 套件

            > **macOS 首次開啟提示：** 若出現「已損毀」或「無法驗證開發者」，請在「終端機」執行：
            > ```
            > xattr -cr /Applications/AITerm.app
            > ```
            > **Linux AppImage：** 執行前需加執行權限：
            > ```
            > chmod +x AITerm_*.AppImage && ./AITerm_*.AppImage
            > ```
          releaseDraft: false
          prerelease: false
          args: --target x86_64-unknown-linux-gnu
```

- [ ] **Step 4: Update macOS and Windows release body to match**

Update the `releaseBody` in both "Build (macOS arm64)" and "Build (Windows)" steps to use the same unified three-platform body as the Linux step above. This ensures whichever job finishes last, the release body is correct.

Replace the macOS `releaseBody`:
```yaml
          releaseBody: |
            ## AITerm ${{ github.ref_name }}

            ### 下載
            - **macOS** (Apple Silicon): 下載 `.dmg` 安裝檔
            - **Windows**: 下載 `-setup.exe` 安裝檔
            - **Linux** (x86_64): 下載 `.AppImage`（免安裝）或 `.deb` 套件

            > **macOS 首次開啟提示：** 若出現「已損毀」或「無法驗證開發者」，請在「終端機」執行：
            > ```
            > xattr -cr /Applications/AITerm.app
            > ```
            > **Linux AppImage：** 執行前需加執行權限：
            > ```
            > chmod +x AITerm_*.AppImage && ./AITerm_*.AppImage
            > ```
```

Replace the Windows `releaseBody`:
```yaml
          releaseBody: |
            ## AITerm ${{ github.ref_name }}

            ### 下載
            - **macOS** (Apple Silicon): 下載 `.dmg` 安裝檔
            - **Windows**: 下載 `-setup.exe` 安裝檔
            - **Linux** (x86_64): 下載 `.AppImage`（免安裝）或 `.deb` 套件

            > **macOS 首次開啟提示：** 若出現「已損毀」或「無法驗證開發者」，請在「終端機」執行：
            > ```
            > xattr -cr /Applications/AITerm.app
            > ```
            > **Linux AppImage：** 執行前需加執行權限：
            > ```
            > chmod +x AITerm_*.AppImage && ./AITerm_*.AppImage
            > ```
```

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add Linux x86_64 build matrix (ubuntu-22.04, AppImage + .deb)"
```

---

## Task 4: Update README

**Files:**
- Modify: `README.md`

Add Linux to the platform list, prerequisites, and development section.

- [ ] **Step 1: Update Prerequisites section**

In the English section under "Prerequisites (Development)", add Linux:

```markdown
- _Linux only_: Ubuntu 22.04+ (or equivalent). Install system libraries:
  ```bash
  sudo apt-get install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libssl-dev
  ```
```

- [ ] **Step 2: Update the Architecture description**

Change:
```
in Windows, macOS, and Linux, with OSC 133 shell integration markers support.
```
(Already mentions Linux in the xterm.js line — verify it's there; if not, add it.)

- [ ] **Step 3: Update the Installing section heading and add Linux**

After the Windows install section (which doesn't exist yet — add it), add:

```markdown
### Installing on Linux

Download the `.AppImage` or `.deb` from the [GitHub Releases](https://github.com/jamesju9999/AITERM/releases/latest) page.

**AppImage (no install required):**
```bash
chmod +x AITerm_*.AppImage
./AITerm_*.AppImage
```

**Debian/Ubuntu `.deb`:**
```bash
sudo dpkg -i aiterm_*.deb
```
```

- [ ] **Step 4: Mirror changes in 繁體中文 section**

Add the same Linux content under the zh-TW section in the same positions.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: add Linux install instructions to README"
```

---

## Task 5: Update Website

**Files:**
- Modify: `aiterm-site/index.html` (at `/Users/jamesju/Documents/GitHub/aiterm-site/index.html`)

Add a Linux platform badge in the hero section alongside macOS and Windows.

- [ ] **Step 1: Add Linux platform tag in hero**

In `index.html`, find the `hero-platforms` div and add a Linux badge after the Windows badge:

```html
        <span class="platform-tag">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><path d="M12.504 0c-.155 0-.315.008-.48.021C7.27.488 3.58 4.27 3.58 9.25c0 2.47.947 4.728 2.497 6.43L3.58 24h16.84l-2.497-8.32C19.473 13.978 20.42 11.72 20.42 9.25 20.42 4.13 16.627.218 12.504 0zm0 2c3.454.178 6.416 2.939 6.416 7.25 0 2.046-.773 3.906-2.04 5.308l.002.007L18.916 22H5.084l2.034-7.435.002-.007C5.853 13.156 5.08 11.296 5.08 9.25c0-4.311 2.962-7.072 7.424-7.25z"/></svg>
          Linux x86_64
        </span>
```

- [ ] **Step 2: Update hero subtitle**

Change:
```html
        在 macOS 與 Windows 上提供原生效能。
```
To:
```html
        在 macOS、Windows 與 Linux 上提供原生效能。
```

- [ ] **Step 3: Add Linux install tab to the install section**

After the Windows tab button, add:
```html
        <button class="tab-btn" data-tab="linux">Linux</button>
```

Add the Linux tab content after the Windows tab content:
```html
      <div class="tab-content" id="tab-linux">
        <ol class="install-steps">
          <li>前往 <a href="https://github.com/jamesju9999/AITERM/releases/latest" target="_blank" rel="noopener" class="release-link">GitHub Releases</a> 下載最新 <code>.AppImage</code> 或 <code>.deb</code></li>
          <li><strong>AppImage（免安裝）：</strong>
            <div class="code-block">
              <button class="copy-btn" data-code="chmod +x AITerm_*.AppImage && ./AITerm_*.AppImage">複製</button>
              <pre>chmod +x AITerm_*.AppImage &amp;&amp; ./AITerm_*.AppImage</pre>
            </div>
          </li>
          <li><strong>Debian / Ubuntu .deb：</strong>
            <div class="code-block">
              <button class="copy-btn" data-code="sudo dpkg -i aiterm_*.deb">複製</button>
              <pre>sudo dpkg -i aiterm_*.deb</pre>
            </div>
          </li>
        </ol>
      </div>
```

- [ ] **Step 4: Commit and push website**

```bash
cd /Users/jamesju/Documents/GitHub/aiterm-site
git add index.html
git commit -m "feat: add Linux platform support to website"
git push
```

---

## Self-Review

**Spec coverage:**
- ✅ `tauri.linux.conf.json` — DB2 excluded on Linux
- ✅ CI matrix — ubuntu-22.04 added with system deps and build step
- ✅ Linux `setup-db2-linux.sh` — dev convenience script
- ✅ Release notes — all three platforms documented
- ✅ README — Linux install instructions (EN + zh-TW)
- ✅ Website — Linux badge + install tab

**Placeholder scan:** None found — all steps have exact code.

**Type consistency:** No shared types across tasks — each task is independent file edits.

**Notes:**
- `keyring` with `sync-secret-service` feature already in `Cargo.toml` — works on Linux at runtime with GNOME Keyring / KWallet; CI build doesn't invoke keyring so no CI issue.
- `tiberius` with `native-tls` uses OpenSSL on Linux — covered by `libssl-dev` in the CI apt step.
- `tauri.linux.conf.json` has no `resources` key since we're not bundling DB2 on Linux (same policy as macOS originally had before DB2 was added there).
- The Linux Tauri build produces `AITerm_x.y.z_amd64.AppImage` and `aiterm_x.y.z_amd64.deb` automatically.
