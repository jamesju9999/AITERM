# 拆開更新的下載與安裝 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓三平台的更新流程完全一致，破壞性動作一律發生在使用者看過警告並按下按鈕之後。

**Architecture:** 把 `useUpdater.ts` 的 `downloadAndInstall()` 拆成【立即更新】時 `download()`、【重新啟動以完成更新】時 `install()` → `relaunch()`。不需平台偵測。

**Tech Stack:** React 19、`@tauri-apps/plugin-updater` 2.10.1、Vitest。

---

## 設計依據

規格：`docs/superpowers/specs/2026-07-29-split-download-and-install-design.md`

實作者必須知道的既有事實（皆已查證）：

- `@tauri-apps/plugin-updater` 2.10.1 的 `index.d.ts` 已提供 `download(onEvent?, options?)` 與 `install()` 兩個獨立方法。
- Windows 的 `install()` 在啟動 NSIS 安裝程式後執行 `std::process::exit(0)`，因此其後的程式碼不會執行。這是 plugin 的設計，非缺陷。
- `runCheck` 於 `installingRef.current || stagedRef.current` 為真時提早返回（`useUpdater.ts:101`），因此下載完成後 `pendingRef` 不會被 `check()` 清掉。
- `errorMessage(e)` 已存在於同檔，為本專案慣例。

## File Structure

| 檔案 | 責任 |
|---|---|
| `src/hooks/useUpdater.ts`（修改） | 狀態機：`install` 只下載，`relaunch` 負責安裝與重啟 |
| `src/hooks/useUpdater.test.ts`（修改） | 28 個既有測試改 mock，新增五項行為測試 |

`UpdateModal.tsx` **不修改**——它呼叫的 `install` / `relaunch` 介面不變，文案「更新已下載完成」在改後更貼切。

---

## Task 1: 狀態機拆分

**Files:** Modify `src/hooks/useUpdater.ts`

- [ ] **Step 1: 改介面定義**

把 `PendingUpdate` 介面中的：

```typescript
  downloadAndInstall: (onEvent: (event: DownloadEvent) => void) => Promise<void>;
```

改為：

```typescript
  download: (onEvent: (event: DownloadEvent) => void) => Promise<void>;
  install: () => Promise<void>;
```

- [ ] **Step 2: `install` 改為只下載**

在 `const install = useCallback(...)` 中，把：

```typescript
      await update.downloadAndInstall((event) => {
```

改為：

```typescript
      await update.download((event) => {
```

並把其後的：

```typescript
      stagedRef.current = true;
      pendingRef.current = null;
      set({ status: "ready", version: update.version });
```

改為：

```typescript
      stagedRef.current = true;
      // pendingRef is deliberately NOT cleared: relaunch() needs this same
      // Update object to call install(). runCheck returns early while
      // stagedRef is set, so nothing overwrites it.
      set({ status: "ready", version: update.version });
```

`Finished` 事件分支中的 `set({ status: "ready", ... })` 維持不變。

- [ ] **Step 3: `relaunch` 負責安裝**

把：

```typescript
  const relaunch = useCallback(async () => {
    await processRelaunch();
  }, []);
```

改為：

```typescript
  const relaunch = useCallback(async () => {
    const update = pendingRef.current;
    if (!update) {
      // Unreachable through the UI — `ready` is only set after a download that
      // keeps pendingRef — but the type is nullable and relaunching without
      // installing would restart into the same version with no sign anything
      // went wrong. Fail loudly instead.
      set({ status: "error", phase: "install", message: "no downloaded update to install" });
      return;
    }
    try {
      // On Windows this launches the NSIS installer and calls exit(0), so
      // nothing below runs — by design, and now it happens only after the user
      // has seen the warning and pressed the button. On macOS and Linux it
      // swaps the bundle in place and returns, and the relaunch completes it.
      await update.install();
      await processRelaunch();
    } catch (e) {
      set({ status: "error", phase: "install", message: errorMessage(e) });
    }
  }, [set]);
```

- [ ] **Step 4: 型別檢查**

Run: `npx tsc -b`
Expected: exit 0。**不要用 `npx tsc --noEmit`**——根 `tsconfig.json` 是 solution 檔（`"files": []`），該指令什麼都不檢查且永遠 exit 0。

此時 `useUpdater.test.ts` 尚未更新，`npm run test` 會失敗，屬預期。

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useUpdater.ts
git commit -m "fix(update): download and install as separate user-approved steps"
```

---

## Task 2: 測試

**Files:** Modify `src/hooks/useUpdater.test.ts`

- [ ] **Step 1: 更新 mock 形狀**

檔內 `fakeUpdate` 的預設值目前含 `downloadAndInstall: vi.fn().mockResolvedValue(undefined)`。改為：

```typescript
    download: vi.fn().mockResolvedValue(undefined),
    install: vi.fn().mockResolvedValue(undefined),
```

所有 `fakeUpdate({ downloadAndInstall: download.fn })` 改為 `fakeUpdate({ download: download.fn })`。

原本斷言 `downloadAndInstall` 被拒絕的測試（`downloadAndInstall: vi.fn().mockRejectedValue(new Error("signature mismatch"))`）改為 `download: vi.fn().mockRejectedValue(new Error("signature mismatch"))`——那個情境測的是下載失敗，語意不變。

- [ ] **Step 2: 執行，確認既有測試回到綠燈**

Run: `npx vitest run src/hooks/useUpdater.test.ts`
Expected: 28 個測試通過。**數字若不符，以實際為準並回報**，不要假設計畫寫的一定對。

- [ ] **Step 3: 新增五項行為測試**

加入 `describe` 內：

```typescript
  it("downloads without installing when the user presses update", async () => {
    // The whole point of this change: on Windows install() exits the process,
    // so calling it here would kill the app before the restart warning is ever
    // shown. If this assertion is removed, reverting to downloadAndInstall
    // breaks nothing visible on macOS while silently regressing Windows.
    const update = fakeUpdate();
    checkMock.mockResolvedValue(update);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    await act(async () => { await result.current.install(); });

    expect(update.download).toHaveBeenCalledTimes(1);
    expect(update.install).not.toHaveBeenCalled();
    expect(result.current.state.status).toBe("ready");
  });

  it("installs before relaunching", async () => {
    const update = fakeUpdate();
    checkMock.mockResolvedValue(update);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));
    await act(async () => { await result.current.install(); });

    await act(async () => { await result.current.relaunch(); });

    expect(update.install).toHaveBeenCalledTimes(1);
    expect(relaunchMock).toHaveBeenCalledTimes(1);
    expect(update.install.mock.invocationCallOrder[0])
      .toBeLessThan(relaunchMock.mock.invocationCallOrder[0]);
  });

  it("reports an install failure instead of relaunching", async () => {
    const update = fakeUpdate({
      install: vi.fn().mockRejectedValue(new Error("signature error")),
    });
    checkMock.mockResolvedValue(update);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));
    await act(async () => { await result.current.install(); });

    await act(async () => { await result.current.relaunch(); });

    expect(result.current.state).toMatchObject({
      status: "error",
      phase: "install",
      message: "signature error",
    });
    expect(relaunchMock).not.toHaveBeenCalled();
  });

  it("keeps the downloaded update usable after a later check", async () => {
    // stagedRef guards this; if it regressed, the update would download fine and
    // then fail to install with no obvious cause.
    const update = fakeUpdate();
    checkMock.mockResolvedValue(update);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));
    await act(async () => { await result.current.install(); });

    await act(async () => { await result.current.check(); });
    await act(async () => { await result.current.relaunch(); });

    expect(update.install).toHaveBeenCalledTimes(1);
  });
```

若 `fakeUpdate` 目前不回傳可供斷言的 mock 物件（例如它回傳的是純物件而 `download` 是每次新建的 `vi.fn()`），請調整 `fakeUpdate` 讓測試能取得同一個 mock 參考，並在回報中說明你怎麼改的。

- [ ] **Step 4: 執行**

Run: `npx vitest run src/hooks/useUpdater.test.ts`
Expected: 32 個測試通過（28 + 4）。**以實際數量為準。**

- [ ] **Step 5: mutation testing（驗收條件）**

逐一套用，執行測試，確認**有測試失敗**，然後還原：

| # | 改動 | 必須失敗的測試 |
|---|---|---|
| M1 | `install` 內的 `update.download(...)` 改回 `update.downloadAndInstall(...)`（並在 mock 中補上該方法） | downloads without installing when the user presses update |
| M2 | `relaunch` 中拿掉 `await update.install();` | installs before relaunching |
| M3 | `relaunch` 的 `catch` 改成只 `console.error(e)` | reports an install failure instead of relaunching |
| M4 | `install` 內恢復 `pendingRef.current = null;` | installs before relaunching（會落入 no-downloaded-update 分支） |

**M1 是本任務的驗收關鍵。** 若它存活，這次改動沒有任何保護——外觀完全正常，而 Windows 使用者依舊會在毫無警告下被關掉 App。

任何 mutation 存活即停止並回報，不要自行補測試。

- [ ] **Step 6: 完整驗證**

```bash
npx tsc -b
npm run test
npx eslint src/hooks/useUpdater.ts src/hooks/useUpdater.test.ts
```

Expected: 全部乾淨。**不要跑 `npm run lint`**——約 181 個既有問題與本次無關。

- [ ] **Step 7: Commit**

```bash
git add src/hooks/useUpdater.test.ts
git commit -m "test(update): pin download-only install and install-before-relaunch"
```

---

## Task 3: 實機驗證（需發版）

**Files:** 無

**Windows 的實際行為無法在 macOS 或 CI 驗證。**

- [ ] **Step 1: 發版前先問**

**未經使用者明確同意不得推 tag。**

- [ ] **Step 2: Windows 實機**

1. 按【立即更新】後 App **不會**結束，出現「更新已下載完成」與重啟警告
2. 按【重新啟動以完成更新】後安裝程式啟動、App 結束
3. 安裝完成後 App 自動重新啟動

第 3 項若不成立屬 NSIS 行為，非本設計缺陷，但須據實記錄——使用者按下的按鈕寫著「重新啟動以完成更新」。

- [ ] **Step 3: macOS 與 Linux 無回歸**

兩平台的更新流程與改動前一致。

- [ ] **Step 4: 記錄結果**

在 `docs/superpowers/specs/2026-07-29-split-download-and-install-design.md` 末尾附上「驗證結果」章節，明確列出未驗證項目。

```bash
git add -f docs/superpowers/specs/2026-07-29-split-download-and-install-design.md
git commit -m "docs: record the split download/install verification"
```

---

## Notes for the implementer

- **不要修改 `UpdateModal.tsx`。** 它的介面不變。
- **不要用 `npx tsc --noEmit`。** 用 `npx tsc -b`。
- **不要跑 `npm run lint`** 作為關卡。
- **不要推 tag。**
- **`docs/` 被 gitignore**（`.gitignore:47`），specs 與 plans 用 `git add -f`。
- `src/lib/i18n.ts` 是 CRLF 檔案。若需編輯，用二進位模式讀寫，否則整檔行尾會被改掉。本計畫不需要動它。
