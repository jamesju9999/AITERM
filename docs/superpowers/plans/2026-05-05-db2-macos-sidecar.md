# DB2 macOS Sidecar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the existing Windows-only DB2 sidecar (.NET + IBM.Data.Db2.Core) to also run on macOS Apple Silicon (arm64), using the same JSON stdio architecture and the IBM macOS clidriver.

**Architecture:** Mirror the Windows approach exactly — bundle the compiled sidecar binary and the IBM clidriver directory together, reference them via Tauri `resources`, and resolve the path at runtime using platform-specific `#[cfg]` blocks. The JSON stdio protocol between Rust and the sidecar is unchanged.

**Tech Stack:** Rust (`#[cfg(target_os = "macos")]`), C# `.NET 8` (`IBM.Data.Db2.Core` v3), Tauri 2 `resources` config, IBM DB2 macOS clidriver (ODBC CLI), GitHub Actions for CI, shell script for dev setup.

---

## File Map

| File | Change |
|------|--------|
| `src-tauri/src/db/db2_sidecar.rs` | Add macOS block to set `DYLD_LIBRARY_PATH` + `DB2_CLI_DRIVER_INSTALL_PATH` |
| `src-tauri/src/lib.rs` | Fix macOS path resolution: add production (app bundle) path + correct dev path |
| `src-tauri/tauri.macos.conf.json` | Add `resources` to bundle the macOS sidecar directory |
| `.github/workflows/release.yml` | Add macOS step to download `db2-sidecar-mac-arm64.zip` from the binaries release |
| `scripts/setup-db2-mac.sh` | New: dev setup script to download IBM clidriver and build the sidecar |

**Not changed:** `db2-sidecar/Program.cs`, `db2-sidecar/db2-sidecar.csproj`, `src-tauri/src/db/db2.rs`, `src-tauri/src/db/manager.rs`. The C# source and JSON protocol are already cross-platform.

---

## Background: Directory Layout

```
# Dev (gitignored, populated by setup-db2-mac.sh):
src-tauri/binaries/db2-sidecar-mac-arm64/
  db2-sidecar          ← compiled .NET binary (no extension on macOS)
  clidriver/
    lib/               ← libdb2.dylib + other dylibs
    cfg/
    msg/
    bnd/

# Production (inside .app bundle, placed by Tauri resources):
AITerm.app/Contents/Resources/
  db2-sidecar          ← same binary
  clidriver/           ← same dylibs
```

The Tauri resources entry `{ "binaries/db2-sidecar-mac-arm64": "." }` copies the *contents* of that directory into `Contents/Resources/`. This mirrors Windows where `{ "binaries/db2-sidecar-win-x64": "." }` copies contents into the exe directory.

---

## Task 1: Add macOS env-var block to `db2_sidecar.rs`

**Files:**
- Modify: `src-tauri/src/db/db2_sidecar.rs:36-55`

The Windows block sets `PATH` and `DB2_CLI_DRIVER_INSTALL_PATH` so the CLI driver DLLs are found. macOS needs the same for `DYLD_LIBRARY_PATH` and `clidriver/lib`.

- [ ] **Step 1: Open `src-tauri/src/db/db2_sidecar.rs` and locate the Windows block (lines ~36-55)**

The block looks like:
```rust
#[cfg(target_os = "windows")]
{
    use std::os::windows::process::CommandExt;
    // ...
}
```

- [ ] **Step 2: Add the macOS block immediately after the closing `}` of the Windows block**

Insert this after line 55:
```rust
        #[cfg(target_os = "macos")]
        {
            let clidriver = sidecar_dir.join("clidriver");
            if clidriver.exists() {
                cmd.env("DB2_CLI_DRIVER_INSTALL_PATH", &clidriver);
                let clidriver_lib = clidriver.join("lib");
                if clidriver_lib.exists() {
                    let old = std::env::var_os("DYLD_LIBRARY_PATH").unwrap_or_default();
                    let mut new_path = std::ffi::OsString::new();
                    new_path.push(&clidriver_lib);
                    new_path.push(":");
                    new_path.push(old);
                    cmd.env("DYLD_LIBRARY_PATH", new_path);
                }
            }
        }
```

- [ ] **Step 3: Verify it compiles (macOS only)**

```bash
cd src-tauri && cargo check --target aarch64-apple-darwin
```

Expected: no errors. If you're on Windows, skip — the `#[cfg]` block is excluded from Windows compilation.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/db/db2_sidecar.rs
git commit -m "feat(db2): add macOS DYLD_LIBRARY_PATH setup for clidriver"
```

---

## Task 2: Fix macOS path resolution in `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs:94-105`

Current macOS code only returns `CARGO_MANIFEST_DIR/binaries/db2-sidecar-{arch}-apple-darwin` which is a bare file path (no directory). This works for dev but fails in production where the binary lives inside the `.app` bundle's `Contents/Resources/`.

The Windows code handles both dev candidates and production (`exe_dir`). We need the same for macOS.

- [ ] **Step 1: Understand the macOS app bundle layout**

In production:
- `current_exe()` = `AITerm.app/Contents/MacOS/AITerm`
- `current_exe().parent()` = `AITerm.app/Contents/MacOS/`
- `current_exe().parent().parent()` = `AITerm.app/Contents/`
- Resources dir = `AITerm.app/Contents/Resources/`
- Sidecar binary = `AITerm.app/Contents/Resources/db2-sidecar`

In dev:
- `CARGO_MANIFEST_DIR` = `<workspace>/src-tauri`
- Sidecar binary = `<workspace>/src-tauri/binaries/db2-sidecar-mac-arm64/db2-sidecar`
  (note: the binary is INSIDE the directory, not at the directory path itself)

- [ ] **Step 2: Replace the two macOS `#[cfg]` blocks (lines 94-105) with this**

```rust
        #[cfg(target_os = "macos")]
        {
            let exe_dir = std::env::current_exe()
                .expect("current_exe")
                .parent()
                .expect("parent dir")
                .to_path_buf();

            let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

            #[cfg(target_arch = "aarch64")]
            let dev_subdir = "db2-sidecar-mac-arm64";
            #[cfg(target_arch = "x86_64")]
            let dev_subdir = "db2-sidecar-mac-x64";

            let candidates = [
                // Production: Tauri resources land in Contents/Resources/
                exe_dir.parent()
                    .expect("Contents dir")
                    .join("Resources")
                    .join("db2-sidecar"),
                // Dev: local build output
                manifest_dir
                    .join("binaries")
                    .join(dev_subdir)
                    .join("db2-sidecar"),
            ];

            candidates
                .into_iter()
                .find(|p| p.exists())
                .unwrap_or_else(|| exe_dir.parent().expect("Contents dir")
                    .join("Resources")
                    .join("db2-sidecar"))
        }
```

- [ ] **Step 3: Verify it compiles**

```bash
cd src-tauri && cargo check --target aarch64-apple-darwin
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(db2): fix macOS sidecar path resolution for dev and production"
```

---

## Task 3: Update `tauri.macos.conf.json`

**Files:**
- Modify: `src-tauri/tauri.macos.conf.json`

Currently this file only has `externalBin: []` which disables the sidecar on macOS. We need to add a `resources` entry that bundles the macOS sidecar directory (binary + clidriver) into the app bundle, mirroring the Windows approach.

- [ ] **Step 1: Read the current file**

Current content:
```json
{
  "bundle": {
    "externalBin": []
  }
}
```

Windows equivalent (`tauri.windows.conf.json`) for reference:
```json
{
  "bundle": {
    "externalBin": [],
    "resources": {
      "binaries/db2-sidecar-win-x64": "."
    }
  }
}
```

- [ ] **Step 2: Update `tauri.macos.conf.json`**

```json
{
  "bundle": {
    "externalBin": [],
    "resources": {
      "binaries/db2-sidecar-mac-arm64": "."
    }
  }
}
```

Note: This targets Apple Silicon (arm64). If x86_64 Intel support is needed later, add `"binaries/db2-sidecar-mac-x64": "."` alongside it — Tauri merges arrays/objects from the platform override.

- [ ] **Step 3: Verify Tauri can read the config (run from workspace root)**

```bash
npx tauri info
```

Expected: no JSON parse errors. The command prints platform info; any config error appears as a parse failure.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tauri.macos.conf.json
git commit -m "feat(db2): bundle macOS sidecar directory via Tauri resources"
```

---

## Task 4: Create dev setup script

**Files:**
- Create: `scripts/setup-db2-mac.sh`

This script downloads the IBM macOS clidriver, builds the sidecar binary, and places both in `src-tauri/binaries/db2-sidecar-mac-arm64/` ready for `npm run tauri:dev`.

- [ ] **Step 1: Create `scripts/` directory if it doesn't exist**

```bash
mkdir -p scripts
```

- [ ] **Step 2: Create `scripts/setup-db2-mac.sh`**

```bash
#!/usr/bin/env bash
# Setup DB2 sidecar for macOS development (Apple Silicon)
# Run once from the workspace root: bash scripts/setup-db2-mac.sh

set -euo pipefail

DEST="src-tauri/binaries/db2-sidecar-mac-arm64"
CLIDRIVER_URL="https://public.dhe.ibm.com/ibmdl/export/pub/software/data/db2/drivers/odbc_cli/macos64_odbc_cli.tar.gz"

echo "==> Creating output directory: $DEST"
mkdir -p "$DEST"

echo "==> Building db2-sidecar for osx-arm64..."
(cd db2-sidecar && dotnet publish -r osx-arm64 --self-contained \
  -o "../$DEST" \
  -p:PublishSingleFile=true \
  --nologo -v quiet)

echo "==> Downloading IBM macOS clidriver (~100MB)..."
TMP=$(mktemp -d)
curl -L "$CLIDRIVER_URL" -o "$TMP/macos64_odbc_cli.tar.gz"

echo "==> Extracting clidriver..."
tar -xzf "$TMP/macos64_odbc_cli.tar.gz" -C "$TMP"
# IBM extracts as "clidriver/" at the top level
cp -R "$TMP/clidriver" "$DEST/clidriver"
rm -rf "$TMP"

echo "==> Making sidecar binary executable..."
chmod +x "$DEST/db2-sidecar"

echo ""
echo "Done. DB2 sidecar ready at: $DEST/"
echo "  db2-sidecar       (binary)"
echo "  clidriver/        (IBM dylibs)"
echo ""
echo "Run 'npm run tauri:dev' to start the app."
```

- [ ] **Step 3: Make it executable**

```bash
chmod +x scripts/setup-db2-mac.sh
```

- [ ] **Step 4: Run the script to verify it works (macOS only)**

```bash
bash scripts/setup-db2-mac.sh
```

Expected output:
```
==> Creating output directory: src-tauri/binaries/db2-sidecar-mac-arm64
==> Building db2-sidecar for osx-arm64...
==> Downloading IBM macOS clidriver (~100MB)...
==> Extracting clidriver...
==> Making sidecar binary executable...
Done. DB2 sidecar ready at: src-tauri/binaries/db2-sidecar-mac-arm64/
```

After running, verify the directory exists:
```bash
ls src-tauri/binaries/db2-sidecar-mac-arm64/
# Expected: db2-sidecar  clidriver/
ls src-tauri/binaries/db2-sidecar-mac-arm64/clidriver/lib/ | head -5
# Expected: libdb2.dylib and related files
```

- [ ] **Step 5: Commit**

```bash
git add scripts/setup-db2-mac.sh
git commit -m "feat(db2): add macOS dev setup script for sidecar + clidriver"
```

---

## Task 5: Upload macOS sidecar zip to GitHub release (one-time manual)

**Context:** The Windows CI downloads `db2-sidecar-win-x64.zip` from the `v0.0.0-db2-binaries` release in this repository. The macOS CI will download `db2-sidecar-mac-arm64.zip` from the same release. This task creates and uploads that zip.

**Prerequisite:** Task 4 must be complete and the script must have run successfully (so `src-tauri/binaries/db2-sidecar-mac-arm64/` is populated).

- [ ] **Step 1: Create the zip (run on macOS)**

```bash
cd src-tauri/binaries
zip -r db2-sidecar-mac-arm64.zip db2-sidecar-mac-arm64/
ls -lh db2-sidecar-mac-arm64.zip
# Expected: file exists, size ~100-200MB
```

- [ ] **Step 2: Upload to the `v0.0.0-db2-binaries` GitHub release**

```bash
gh release upload v0.0.0-db2-binaries \
  src-tauri/binaries/db2-sidecar-mac-arm64.zip \
  --repo $GITHUB_REPOSITORY \
  --clobber
```

If `$GITHUB_REPOSITORY` is not set:
```bash
gh release upload v0.0.0-db2-binaries \
  src-tauri/binaries/db2-sidecar-mac-arm64.zip \
  --clobber
```
(gh CLI will infer the repo from git remote)

- [ ] **Step 3: Verify the upload**

```bash
gh release view v0.0.0-db2-binaries --json assets \
  --jq '.assets[].name'
```

Expected: output includes both `db2-sidecar-win-x64.zip` and `db2-sidecar-mac-arm64.zip`.

---

## Task 6: Update CI to download macOS sidecar

**Files:**
- Modify: `.github/workflows/release.yml`

The Windows job already downloads `db2-sidecar-win-x64.zip`. Add an equivalent step for the macOS job. macOS uses `bash` + `unzip` (available by default on macOS runners).

- [ ] **Step 1: Locate the Windows download step in `release.yml` (lines ~56-63)**

```yaml
- name: Download DB2 sidecar binaries (Windows)
  if: matrix.os == 'windows-latest'
  shell: powershell
  env:
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
  run: |
    gh release download v0.0.0-db2-binaries --pattern "db2-sidecar-win-x64.zip" --repo ${{ github.repository }}
    Expand-Archive -Path db2-sidecar-win-x64.zip -DestinationPath src-tauri/binaries/
```

- [ ] **Step 2: Add a macOS step immediately before the Windows step**

```yaml
      # macOS: download pre-built DB2 sidecar binaries (binary + clidriver) from dedicated release
      - name: Download DB2 sidecar binaries (macOS)
        if: matrix.os == 'macos-latest'
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          gh release download v0.0.0-db2-binaries --pattern "db2-sidecar-mac-arm64.zip" --repo ${{ github.repository }}
          mkdir -p src-tauri/binaries
          unzip -q db2-sidecar-mac-arm64.zip -d src-tauri/binaries/
          chmod +x src-tauri/binaries/db2-sidecar-mac-arm64/db2-sidecar
```

- [ ] **Step 3: Remove the `brew install unixodbc` step (macOS, lines ~49-51)**

That step was a placeholder for a future ODBC approach. With the bundled clidriver, unixODBC is not needed. Remove these lines:

```yaml
      # macOS: DB2 adapter requires unixODBC
      - name: Install unixODBC (macOS)
        if: matrix.os == 'macos-latest'
        run: brew install unixodbc
```

- [ ] **Step 4: Verify the YAML is valid**

```bash
python3 -c "import yaml, sys; yaml.safe_load(open('.github/workflows/release.yml'))" && echo "YAML OK"
```

Expected: `YAML OK`

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: download and unzip macOS DB2 sidecar in release workflow"
```

---

## Task 7: End-to-end smoke test (macOS)

This task verifies everything works together. It requires a real DB2 server or the IBM DB2 Community Edition running locally.

- [ ] **Step 1: Run the dev server**

```bash
npm run tauri:dev
```

Expected: app starts without errors in the Tauri devtools console related to DB2.

- [ ] **Step 2: Verify the sidecar spawns correctly**

Open the app, navigate to the DB panel, add a DB2 connection (any hostname, even a fake one). Expected error: `"[IBM][CLI Driver] SQL30081N A communication error was detected"` — this means the sidecar launched and connected to IBM CLI correctly, it just can't reach the server.

If the error is `db2_sidecar_not_found:` — the binary path in Task 2 is wrong. Check `ls src-tauri/binaries/db2-sidecar-mac-arm64/db2-sidecar`.

If the error is about `libdb2.dylib not found` — the `DYLD_LIBRARY_PATH` in Task 1 is not reaching the spawned process. Check that `clidriver/lib/libdb2.dylib` exists.

- [ ] **Step 3: (Optional) Test against a real DB2 instance**

If you have access to a DB2 server, add a connection with valid credentials and run `SELECT CURRENT DATE FROM SYSIBM.SYSDUMMY1`. Expected: one row with today's date.

---

## Self-Review Checklist

- [x] **Spec coverage:** All five items covered — macOS env vars (Task 1), path resolution (Task 2), Tauri bundling (Task 3), dev setup (Task 4), CI (Task 6), binary upload (Task 5)
- [x] **No placeholders:** All steps include exact code, commands, and expected output
- [x] **Type/name consistency:** `db2-sidecar-mac-arm64` used consistently across Tasks 2, 3, 4, 5, 6
- [x] **macOS path calculation:** `exe.parent().parent().join("Resources")` correctly navigates from `Contents/MacOS/AITerm` to `Contents/Resources/`
- [x] **Windows not broken:** Tasks only touch `#[cfg(target_os = "macos")]` blocks and macOS-specific config files; Windows path untouched
