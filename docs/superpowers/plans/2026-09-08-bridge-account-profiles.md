# Claude Bridge 帳號組合（Profiles）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Claude Bridge 設定頁加一個「帳號組合」區塊，讓使用者把 opus/sonnet/haiku 三個 tier 的 provider 映射存成具名快照，之後一鍵套用（立即生效，不用重啟或重新登入 `claude` CLI），並能更新／重新命名／刪除。

**Architecture:** 純前端功能。新增 `src/components/Settings/bridgeProfiles.ts`（型別 + localStorage 讀寫 + tier 比對的純函式，獨立於 React 之外可測）。`ClaudeBridgePage.tsx` 讀寫這個模組維護 profile 清單狀態，「套用」動作沿用既有的 `bridgeSetConfig` IPC（已確認橋接 server 每個請求都重新讀設定，存檔即生效）。不動 Rust 後端、不動 `ClaudeBridgeConfig` 的 IPC 型別。

**Tech Stack:** React 19 + TypeScript、Vitest + React Testing Library（既有）、`@tauri-apps/plugin-dialog` 的 `confirm()`（刪除確認——**不可用 `window.confirm`**，Tauri webview 沒有 JS dialog panel，會直接無聲跳過）。

**規格：** `docs/superpowers/specs/2026-09-08-bridge-account-profiles-design.md`

---

## Task 1: `bridgeProfiles.ts` — 型別、localStorage 讀寫、tier 比對

**Files:**
- Create: `src/components/Settings/bridgeProfiles.ts`
- Test: `src/components/Settings/bridgeProfiles.test.ts`

- [ ] **Step 1: 寫失敗測試**

```ts
// src/components/Settings/bridgeProfiles.test.ts
import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  loadBridgeProfiles,
  saveBridgeProfiles,
  tiersEqual,
  type BridgeProfile,
} from "./bridgeProfiles";

const PROFILE: BridgeProfile = {
  id: "p1",
  name: "個人帳號",
  opus: { provider_id: "acct-a", model: "claude-opus-4" },
  sonnet: { provider_id: "acct-a", model: "claude-sonnet-4" },
  haiku: null,
};

beforeEach(() => {
  localStorage.clear();
});

describe("loadBridgeProfiles / saveBridgeProfiles", () => {
  it("尚未存過時回傳空陣列", () => {
    expect(loadBridgeProfiles()).toEqual([]);
  });

  it("存了之後讀得回一模一樣的內容", () => {
    saveBridgeProfiles([PROFILE]);
    expect(loadBridgeProfiles()).toEqual([PROFILE]);
  });

  it("localStorage 內容是壞掉的 JSON 時回傳空陣列而不是拋例外", () => {
    localStorage.setItem("aiterm.bridgeProfiles", "{not json");
    expect(loadBridgeProfiles()).toEqual([]);
  });

  it("localStorage 內容不是陣列時回傳空陣列", () => {
    localStorage.setItem("aiterm.bridgeProfiles", JSON.stringify({ not: "an array" }));
    expect(loadBridgeProfiles()).toEqual([]);
  });

  it("讀取拋例外時回傳空陣列而不是讓呼叫端炸掉", () => {
    const spy = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(loadBridgeProfiles()).toEqual([]);
    spy.mockRestore();
  });

  it("寫入拋例外時不拋出，呼叫端可以繼續", () => {
    const spy = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota exceeded");
    });
    expect(() => saveBridgeProfiles([PROFILE])).not.toThrow();
    spy.mockRestore();
  });
});

describe("tiersEqual", () => {
  it("三層都是 null 時視為相等", () => {
    expect(tiersEqual({ opus: null, sonnet: null, haiku: null }, { opus: null, sonnet: null, haiku: null })).toBe(true);
  });

  it("provider_id 與 model 都相同時視為相等", () => {
    const a = { opus: { provider_id: "x", model: "m" }, sonnet: null, haiku: null };
    const b = { opus: { provider_id: "x", model: "m" }, sonnet: null, haiku: null };
    expect(tiersEqual(a, b)).toBe(true);
  });

  it("model 不同就不相等", () => {
    const a = { opus: { provider_id: "x", model: "m1" }, sonnet: null, haiku: null };
    const b = { opus: { provider_id: "x", model: "m2" }, sonnet: null, haiku: null };
    expect(tiersEqual(a, b)).toBe(false);
  });

  it("一邊 null 一邊不是 null 就不相等", () => {
    const a = { opus: { provider_id: "x", model: "m" }, sonnet: null, haiku: null };
    const b = { opus: null, sonnet: null, haiku: null };
    expect(tiersEqual(a, b)).toBe(false);
  });
});
```

- [ ] **Step 2: 執行測試確認全部失敗（模組還不存在）**

Run: `npx vitest run src/components/Settings/bridgeProfiles.test.ts`
Expected: FAIL — `Cannot find module './bridgeProfiles'`

- [ ] **Step 3: 寫最小實作**

```ts
// src/components/Settings/bridgeProfiles.ts
import type { TierMapping } from "../../ipc/bridge";

export interface BridgeProfile {
  id: string;
  name: string;
  opus: TierMapping | null;
  sonnet: TierMapping | null;
  haiku: TierMapping | null;
}

const STORAGE_KEY = "aiterm.bridgeProfiles";

/** localStorage 壞掉、內容損毀或被瀏覽器擋下時一律視為「沒有存過」，不讓設定頁掛掉。 */
export function loadBridgeProfiles(): BridgeProfile[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as BridgeProfile[]) : [];
  } catch {
    return [];
  }
}

/** 寫入失敗（例如私密模式關閉 storage）不拋出——頂多這次操作不會持久化。 */
export function saveBridgeProfiles(profiles: BridgeProfile[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(profiles));
  } catch {
    // 忽略：呼叫端已經更新了記憶體中的 state，使用者這次 session 還是看得到。
  }
}

type TierTriple = Pick<BridgeProfile, "opus" | "sonnet" | "haiku">;

function tierEqual(a: TierMapping | null, b: TierMapping | null): boolean {
  if (a === null || b === null) return a === b;
  return a.provider_id === b.provider_id && a.model === b.model;
}

/** 「使用中」徽章的判斷依據：目前表格的三個 tier 是否跟某個 profile 逐一相符。 */
export function tiersEqual(a: TierTriple, b: TierTriple): boolean {
  return tierEqual(a.opus, b.opus) && tierEqual(a.sonnet, b.sonnet) && tierEqual(a.haiku, b.haiku);
}
```

- [ ] **Step 4: 執行測試確認全部通過**

Run: `npx vitest run src/components/Settings/bridgeProfiles.test.ts`
Expected: PASS（15 個 it）

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings/bridgeProfiles.ts src/components/Settings/bridgeProfiles.test.ts
git commit -m "$(cat <<'EOF'
feat(bridge): add BridgeProfile storage + tier comparison helpers

Pure functions backing the upcoming account-profile switcher in
ClaudeBridgePage — localStorage read/write and tier-triple equality,
independently testable without mounting the settings page.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

## Task 2: i18n 字串

**Files:**
- Modify: `src/lib/i18n.ts:713-714`（zh-TW，`bridge_copy_command` / `bridge_copied` 後面）
- Modify: `src/lib/i18n.ts:2198-2199`（en，對應位置）

- [ ] **Step 1: 在 zh-TW 區塊插入新字串**

在 `src/lib/i18n.ts` 第 714 行（`bridge_copied: "已複製",`）之後插入：

```ts
    bridge_section_profiles: "帳號組合",
    bridge_section_profiles_desc: "把目前的三個層級設定存成一份預設值，之後一鍵切換整組，不用逐一重選、不用重啟 claude。",
    bridge_profile_empty: "還沒有存過任何組合。先在下面把三個層級設定好，再存成一組。",
    bridge_profile_new_placeholder: "組合名稱，例如「個人帳號」",
    bridge_profile_new: "另存目前設定為新組合",
    bridge_profile_active: "使用中",
    bridge_profile_apply: "套用",
    bridge_profile_update: "更新",
    bridge_profile_rename: "重新命名",
    bridge_profile_delete: "刪除",
    bridge_profile_delete_confirm: (name: string) => `確定要刪除帳號組合「${name}」嗎？`,
    bridge_profile_rename_placeholder: "新名稱",
```

- [ ] **Step 2: 在 en 區塊插入對應字串**

在 `src/lib/i18n.ts` 的 en 區塊裡，`bridge_copied: "Copied",`（第 2199 行附近，`grep -n 'bridge_copied' src/lib/i18n.ts` 確認行號）之後插入：

```ts
    bridge_section_profiles: "Account profiles",
    bridge_section_profiles_desc: "Save the current three tier mappings as a named preset, then switch the whole group in one click — no re-picking each tier, no restarting claude.",
    bridge_profile_empty: "No profiles saved yet. Set up the three tiers below, then save them as one.",
    bridge_profile_new_placeholder: "Profile name, e.g. \"Personal\"",
    bridge_profile_new: "Save current as new profile",
    bridge_profile_active: "Active",
    bridge_profile_apply: "Apply",
    bridge_profile_update: "Update",
    bridge_profile_rename: "Rename",
    bridge_profile_delete: "Delete",
    bridge_profile_delete_confirm: (name: string) => `Delete account profile "${name}"?`,
    bridge_profile_rename_placeholder: "New name",
```

- [ ] **Step 3: 型別檢查確認 zh-TW / en 兩份 key 集合一致**

Run: `npx tsc -b`
Expected: 無錯誤（`Translations` 型別是從 `zh-TW` 物件推導出來的——見 `src/lib/i18n.ts:2890-2892`——若 en 漏掉任一 key，`translations.en` 對不上 `Translations` 型別會在別處報型別錯誤；若兩邊 key 名稱打錯字也會在這裡現形）

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "$(cat <<'EOF'
feat(i18n): add bridge_profile_* strings for account-profile switcher

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

## Task 3: `ClaudeBridgePage.tsx` — 「帳號組合」區塊（狀態、新增、清單、使用中徽章）

**Files:**
- Modify: `src/components/Settings/ClaudeBridgePage.tsx`
- Test: `src/components/Settings/ClaudeBridgePage.test.tsx`

這個 task 先做「新增組合」與「顯示清單＋使用中徽章」，套用／更新／改名／刪除留給 Task 4、5。

- [ ] **Step 1: 寫失敗測試（新增到既有測試檔尾端，`describe("ClaudeBridgePage")` 區塊內最後一個 `it` 之後）**

```tsx
// 加在 src/components/Settings/ClaudeBridgePage.test.tsx 既有的
// describe("ClaudeBridgePage", () => { ... }) 區塊內、最後一個既有 it 之後，
// beforeEach 已有的 vi.clearAllMocks() 不會清 localStorage，所以這裡自己清。
beforeEach(() => {
  localStorage.clear();
});

it("沒有任何組合時顯示空狀態提示", async () => {
  render(<ClaudeBridgePage />);
  expect(await screen.findByText(/還沒有存過任何組合|No profiles saved yet/)).toBeInTheDocument();
});

it("另存目前設定為新組合後出現在清單裡，且標示使用中", async () => {
  const user = userEvent.setup();
  render(<ClaudeBridgePage />);

  await user.selectOptions(await screen.findByLabelText(/Opus/), "qwen");

  const nameInput = await screen.findByPlaceholderText(/組合名稱|Profile name/);
  await user.type(nameInput, "個人帳號");
  await user.click(screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }));

  expect(await screen.findByText("個人帳號")).toBeInTheDocument();
  expect(screen.getByText(/使用中|Active/)).toBeInTheDocument();
});

it("改動表格之後，原本標示使用中的組合就不再標示", async () => {
  const user = userEvent.setup();
  render(<ClaudeBridgePage />);

  await user.selectOptions(await screen.findByLabelText(/Opus/), "qwen");
  await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
  await user.click(screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }));
  await screen.findByText(/使用中|Active/);

  await user.selectOptions(await screen.findByLabelText(/Opus/), "cdx");

  await waitFor(() => {
    expect(screen.queryByText(/使用中|Active/)).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 執行測試確認新增的三個測試失敗**

Run: `npx vitest run src/components/Settings/ClaudeBridgePage.test.tsx`
Expected: FAIL（找不到 placeholder / 找不到「另存目前設定為新組合」按鈕 / 找不到清單文字）

- [ ] **Step 3: 在 `ClaudeBridgePage.tsx` 加入 profile 狀態與新增/清單 UI**

在檔案頂部 import 區塊（第 14 行 `import "./ClaudeBridgePage.css";` 之前）加入：

```tsx
import {
  loadBridgeProfiles,
  saveBridgeProfiles,
  tiersEqual,
  type BridgeProfile,
} from "./bridgeProfiles";
```

在 `ClaudeBridgePage` 函式內，既有的 `const [saved, setSaved] = useState(false);`（第 86 行）之後加入：

```tsx
  const [profiles, setProfiles] = useState<BridgeProfile[]>(() => loadBridgeProfiles());
  const [newProfileName, setNewProfileName] = useState("");

  const persistProfiles = useCallback((next: BridgeProfile[]) => {
    setProfiles(next);
    saveBridgeProfiles(next);
  }, []);

  const createProfile = useCallback(() => {
    if (!cfg) return;
    const name = newProfileName.trim();
    if (!name) return;
    const profile: BridgeProfile = {
      id: crypto.randomUUID(),
      name,
      opus: cfg.opus,
      sonnet: cfg.sonnet,
      haiku: cfg.haiku,
    };
    persistProfiles([...profiles, profile]);
    setNewProfileName("");
  }, [cfg, newProfileName, profiles, persistProfiles]);
```

在既有的「Tier 對應」`<section className="bridge-section">`（第 222 行 `<h3>{t.bridge_section_tiers}</h3>` 那個 section）**之前**插入新 section：

```tsx
      <section className="bridge-section">
        <h3>{t.bridge_section_profiles}</h3>
        <p className="bridge-section-desc">{t.bridge_section_profiles_desc}</p>

        {profiles.length === 0 ? (
          <p className="bridge-profile-empty">{t.bridge_profile_empty}</p>
        ) : (
          <ul className="bridge-profile-list">
            {profiles.map((p) => {
              const isActive = cfg ? tiersEqual(cfg, p) : false;
              return (
                <li key={p.id} className="bridge-profile-row">
                  <span className="bridge-profile-name">{p.name}</span>
                  {isActive && <span className="bridge-profile-badge">{t.bridge_profile_active}</span>}
                  <div className="bridge-profile-actions">
                    <button type="button">{t.bridge_profile_apply}</button>
                    <button type="button">{t.bridge_profile_update}</button>
                    <button type="button">{t.bridge_profile_rename}</button>
                    <button type="button">{t.bridge_profile_delete}</button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}

        <div className="bridge-profile-new-row">
          <input
            value={newProfileName}
            onChange={(e) => setNewProfileName(e.target.value)}
            placeholder={t.bridge_profile_new_placeholder}
          />
          <button type="button" onClick={createProfile} disabled={!newProfileName.trim()}>
            {t.bridge_profile_new}
          </button>
        </div>
      </section>

```

（四個動作按鈕先放空殼，Task 4、5 會補上對應的 `onClick`。）

- [ ] **Step 4: 執行測試確認通過**

Run: `npx vitest run src/components/Settings/ClaudeBridgePage.test.tsx`
Expected: PASS（含既有測試與新增的 3 個，共 10 個）

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings/ClaudeBridgePage.tsx src/components/Settings/ClaudeBridgePage.test.tsx
git commit -m "$(cat <<'EOF'
feat(bridge): add account-profile list with save-current and active badge

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

## Task 4: 套用（立即生效）與更新

**Files:**
- Modify: `src/components/Settings/ClaudeBridgePage.tsx`
- Test: `src/components/Settings/ClaudeBridgePage.test.tsx`

- [ ] **Step 1: 寫失敗測試**

```tsx
it("套用組合時立即呼叫 bridgeSetConfig，帶入該組合的三個 tier", async () => {
  const user = userEvent.setup();
  render(<ClaudeBridgePage />);

  // 先用 qwen 存一組
  await user.selectOptions(await screen.findByLabelText(/Opus/), "qwen");
  await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
  await user.click(screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }));

  // 換成 cdx，離開「個人帳號」目前這組
  await user.selectOptions(await screen.findByLabelText(/Opus/), "cdx");
  vi.mocked(bridgeSetConfig).mockClear();

  // 套用「個人帳號」應該把 opus 換回 qwen 並立刻存檔——不用另外按 Save
  await user.click(screen.getByRole("button", { name: t.bridge_profile_apply }));

  await waitFor(() => expect(bridgeSetConfig).toHaveBeenCalledTimes(1));
  const payload = vi.mocked(bridgeSetConfig).mock.calls[0][0];
  expect(payload.opus?.provider_id).toBe("qwen");
});

it("套用組合後畫面上的 tier 表格也跟著換", async () => {
  const user = userEvent.setup();
  render(<ClaudeBridgePage />);

  await user.selectOptions(await screen.findByLabelText(/Opus/), "qwen");
  await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
  await user.click(screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }));

  await user.selectOptions(await screen.findByLabelText(/Opus/), "cdx");
  await user.click(screen.getByRole("button", { name: t.bridge_profile_apply }));

  await waitFor(() => {
    expect(screen.getByLabelText(/Opus/)).toHaveValue("qwen");
  });
});

it("更新組合只覆蓋 localStorage，不呼叫 bridgeSetConfig", async () => {
  const user = userEvent.setup();
  render(<ClaudeBridgePage />);

  await user.selectOptions(await screen.findByLabelText(/Opus/), "qwen");
  await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
  await user.click(screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }));

  await user.selectOptions(await screen.findByLabelText(/Opus/), "cdx");
  vi.mocked(bridgeSetConfig).mockClear();

  await user.click(screen.getByRole("button", { name: t.bridge_profile_update }));

  expect(bridgeSetConfig).not.toHaveBeenCalled();
  const stored = JSON.parse(localStorage.getItem("aiterm.bridgeProfiles") ?? "[]");
  expect(stored[0].opus.provider_id).toBe("cdx");
});
```

上面用了 `t.bridge_profile_apply` / `t.bridge_profile_update` 當按鈕名稱比對——測試檔案需要在頂部額外 `import { translations } from "../../lib/i18n"; const t = translations["zh-TW"];`（跟現有測試檔一致用中文斷言時通常直接寫死字串；這裡因為畫面上四個動作按鈕文字很短、容易撞名，改用翻譯物件比對更精準）。在測試檔案 import 區塊補上：

```tsx
import { translations } from "../../lib/i18n";
const t = translations["zh-TW"];
```

- [ ] **Step 2: 執行測試確認新增的三個測試失敗**

Run: `npx vitest run src/components/Settings/ClaudeBridgePage.test.tsx`
Expected: FAIL（套用/更新按鈕目前是空殼，沒有 onClick 效果）

- [ ] **Step 3: 實作套用與更新**

在 `createProfile` 定義之後加入：

```tsx
  const applyProfile = useCallback(
    async (profile: BridgeProfile) => {
      if (!cfg) return;
      const next: ClaudeBridgeConfig = {
        ...cfg,
        opus: profile.opus,
        sonnet: profile.sonnet,
        haiku: profile.haiku,
      };
      setCfg(next);
      setSaving(true);
      try {
        setStatus(await bridgeSetConfig(next));
        setSaved(true);
      } finally {
        setSaving(false);
      }
    },
    [cfg],
  );

  const updateProfile = useCallback(
    (profile: BridgeProfile) => {
      if (!cfg) return;
      persistProfiles(
        profiles.map((p) =>
          p.id === profile.id ? { ...p, opus: cfg.opus, sonnet: cfg.sonnet, haiku: cfg.haiku } : p,
        ),
      );
    },
    [cfg, profiles, persistProfiles],
  );
```

把 Task 3 放的空殼按鈕改成：

```tsx
                    <button type="button" onClick={() => void applyProfile(p)}>
                      {t.bridge_profile_apply}
                    </button>
                    <button type="button" onClick={() => updateProfile(p)}>
                      {t.bridge_profile_update}
                    </button>
```

- [ ] **Step 4: 執行測試確認通過**

Run: `npx vitest run src/components/Settings/ClaudeBridgePage.test.tsx`
Expected: PASS（共 13 個）

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings/ClaudeBridgePage.tsx src/components/Settings/ClaudeBridgePage.test.tsx
git commit -m "$(cat <<'EOF'
feat(bridge): apply/update account profiles

Apply writes the profile's tier mapping straight to the running bridge
server via the existing bridgeSetConfig call — since the server re-reads
config on every request, an already-running claude session picks up the
new account on its very next message, no restart or re-login needed.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

## Task 5: 重新命名與刪除

**Files:**
- Modify: `src/components/Settings/ClaudeBridgePage.tsx`
- Test: `src/components/Settings/ClaudeBridgePage.test.tsx`

刪除必須用 `@tauri-apps/plugin-dialog` 的 `confirm()`，**不可以用 `window.confirm`**——Tauri webview 沒有 JS dialog panel，`window.confirm()` 會直接無聲跳過並回傳 undefined-ish falsy，測試在 jsdom 裡會「看似」呼叫了什麼但正式環境其實是直接刪除、完全不會跳確認框（`NotebookSidebar.tsx` 已經踩過這個坑，見該檔案開頭註解）。

- [ ] **Step 1: 在測試檔頂部加入 dialog mock**

在 `ClaudeBridgePage.test.tsx` 現有的 `vi.mock(...)` 區塊（第 7-12 行）旁邊加一行，並加對應 import：

```tsx
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));
```

```tsx
import { confirm } from "@tauri-apps/plugin-dialog";
```

在既有的 `beforeEach` 裡（第 89 行 `beforeEach(() => {` 內）加一行預設值：

```tsx
  vi.mocked(confirm).mockResolvedValue(true);
```

- [ ] **Step 2: 寫失敗測試**

```tsx
it("重新命名組合", async () => {
  const user = userEvent.setup();
  render(<ClaudeBridgePage />);

  await user.selectOptions(await screen.findByLabelText(/Opus/), "qwen");
  await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
  await user.click(screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }));

  await user.click(screen.getByRole("button", { name: t.bridge_profile_rename }));
  const renameInput = await screen.findByPlaceholderText(/新名稱|New name/);
  await user.clear(renameInput);
  await user.type(renameInput, "公司帳號{Enter}");

  expect(await screen.findByText("公司帳號")).toBeInTheDocument();
  expect(screen.queryByText("個人帳號")).not.toBeInTheDocument();
});

it("刪除組合前會跳確認框，確認後才真的刪除", async () => {
  const user = userEvent.setup();
  render(<ClaudeBridgePage />);

  await user.selectOptions(await screen.findByLabelText(/Opus/), "qwen");
  await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
  await user.click(screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }));

  await user.click(screen.getByRole("button", { name: t.bridge_profile_delete }));

  await waitFor(() => expect(confirm).toHaveBeenCalledWith(t.bridge_profile_delete_confirm("個人帳號"), expect.anything()));
  await waitFor(() => expect(screen.queryByText("個人帳號")).not.toBeInTheDocument());
});

it("取消刪除確認框時保留組合", async () => {
  vi.mocked(confirm).mockResolvedValue(false);
  const user = userEvent.setup();
  render(<ClaudeBridgePage />);

  await user.selectOptions(await screen.findByLabelText(/Opus/), "qwen");
  await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
  await user.click(screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }));

  await user.click(screen.getByRole("button", { name: t.bridge_profile_delete }));

  await waitFor(() => expect(confirm).toHaveBeenCalled());
  expect(screen.getByText("個人帳號")).toBeInTheDocument();
});
```

- [ ] **Step 3: 執行測試確認新增的三個測試失敗**

Run: `npx vitest run src/components/Settings/ClaudeBridgePage.test.tsx`
Expected: FAIL

- [ ] **Step 4: 實作重新命名與刪除**

在檔案頂部 import 加入：

```tsx
import { confirm } from "@tauri-apps/plugin-dialog";
```

在 `updateProfile` 之後加入：

```tsx
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");

  const startRename = useCallback((profile: BridgeProfile) => {
    setRenamingId(profile.id);
    setRenameDraft(profile.name);
  }, []);

  const commitRename = useCallback(() => {
    if (!renamingId) return;
    const trimmed = renameDraft.trim();
    if (trimmed) {
      persistProfiles(profiles.map((p) => (p.id === renamingId ? { ...p, name: trimmed } : p)));
    }
    setRenamingId(null);
  }, [renamingId, renameDraft, profiles, persistProfiles]);

  const deleteProfile = useCallback(
    async (profile: BridgeProfile) => {
      const ok = await confirm(t.bridge_profile_delete_confirm(profile.name), {
        kind: "warning",
        okLabel: t.common_delete,
        cancelLabel: t.common_cancel,
      });
      if (!ok) return;
      persistProfiles(profiles.filter((p) => p.id !== profile.id));
    },
    [profiles, persistProfiles, t],
  );
```

`useState` import 已存在（第 1 行），不用另外加。

把清單裡每列的 JSX 換成（`renamingId === p.id` 時名稱欄改成輸入框）：

```tsx
                  {renamingId === p.id ? (
                    <input
                      className="bridge-profile-rename-input"
                      value={renameDraft}
                      autoFocus
                      placeholder={t.bridge_profile_rename_placeholder}
                      onChange={(e) => setRenameDraft(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") commitRename();
                        if (e.key === "Escape") setRenamingId(null);
                      }}
                      onBlur={commitRename}
                    />
                  ) : (
                    <span className="bridge-profile-name">{p.name}</span>
                  )}
                  {isActive && <span className="bridge-profile-badge">{t.bridge_profile_active}</span>}
                  <div className="bridge-profile-actions">
                    <button type="button" onClick={() => void applyProfile(p)}>
                      {t.bridge_profile_apply}
                    </button>
                    <button type="button" onClick={() => updateProfile(p)}>
                      {t.bridge_profile_update}
                    </button>
                    <button type="button" onClick={() => startRename(p)}>
                      {t.bridge_profile_rename}
                    </button>
                    <button type="button" onClick={() => void deleteProfile(p)}>
                      {t.bridge_profile_delete}
                    </button>
                  </div>
```

- [ ] **Step 5: 執行測試確認通過**

Run: `npx vitest run src/components/Settings/ClaudeBridgePage.test.tsx`
Expected: PASS（共 16 個）

- [ ] **Step 6: Commit**

```bash
git add src/components/Settings/ClaudeBridgePage.tsx src/components/Settings/ClaudeBridgePage.test.tsx
git commit -m "$(cat <<'EOF'
feat(bridge): rename/delete account profiles with confirm dialog

Uses @tauri-apps/plugin-dialog's confirm() for the delete prompt —
window.confirm() is a silent no-op in the Tauri webview and would
delete without ever showing a dialog.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

## Task 6: CSS

**Files:**
- Modify: `src/components/Settings/ClaudeBridgePage.css`

- [ ] **Step 1: 加入樣式（沿用既有色票：`#e6e6e6` 主文字、`#777` 次要、`#1a1a1a`/`#2e2e2e` 輸入框、`#2d5aab` focus、`#34d399` 綠色強調）**

在檔案尾端（第 186 行之後）加入：

```css
.bridge-profile-empty {
  color: #777;
  font-size: 12px;
  margin: 0 0 14px;
}

.bridge-profile-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  list-style: none;
  margin: 0 0 14px;
  padding: 0;
}

.bridge-profile-row {
  align-items: center;
  background: #1a1a1a;
  border: 1px solid #2e2e2e;
  border-radius: 6px;
  display: flex;
  gap: 10px;
  padding: 8px 10px;
}

.bridge-profile-name {
  color: #e6e6e6;
  flex: 1;
  font-size: 13px;
}

.bridge-profile-rename-input {
  background: #111;
  border: 1px solid #2d5aab;
  border-radius: 4px;
  color: #e6e6e6;
  flex: 1;
  font-size: 13px;
  padding: 4px 8px;
}

.bridge-profile-badge {
  background: rgba(52, 211, 153, 0.15);
  border-radius: 4px;
  color: #34d399;
  font-size: 11px;
  padding: 2px 8px;
}

.bridge-profile-actions {
  display: flex;
  gap: 6px;
}

.bridge-profile-actions button {
  background: transparent;
  border: 1px solid #2e2e2e;
  border-radius: 4px;
  color: #ccc;
  cursor: pointer;
  font-size: 12px;
  padding: 4px 10px;
}

.bridge-profile-actions button:hover {
  border-color: #2d5aab;
  color: #e6e6e6;
}

.bridge-profile-new-row {
  display: flex;
  gap: 8px;
}

.bridge-profile-new-row input {
  background: #1a1a1a;
  border: 1px solid #2e2e2e;
  border-radius: 6px;
  color: #e6e6e6;
  flex: 1;
  font-size: 13px;
  padding: 6px 10px;
}

.bridge-profile-new-row input:focus {
  border-color: #2d5aab;
  outline: none;
}

.bridge-profile-new-row button {
  background: #1a1a1a;
  border: 1px solid #2e2e2e;
  border-radius: 6px;
  color: #e6e6e6;
  cursor: pointer;
  font-size: 13px;
  padding: 6px 16px;
  white-space: nowrap;
}

.bridge-profile-new-row button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.bridge-profile-new-row button:hover:not(:disabled) {
  border-color: #2d5aab;
}
```

- [ ] **Step 2: 手動檢查（沒有 CSS 測試框架，跑起 dev server 目視確認）**

Run: `npm run tauri:dev`（或若已在跑就略過），開 Settings → Claude Bridge，確認：新區塊排版沒有把面板撐出水平捲軸、「使用中」徽章顏色跟既有的綠色連線指示點（`.bridge-dot--on`）一致、按鈕 hover 顏色跟其他頁一致。

- [ ] **Step 3: Commit**

```bash
git add src/components/Settings/ClaudeBridgePage.css
git commit -m "$(cat <<'EOF'
style(bridge): style account-profile list and new-profile row

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

## Task 7: 全套驗證

**Files:** 無新增/修改，純驗證。

- [ ] **Step 1: 跑完整前端測試套件**

Run: `npm run test`
Expected: 全部 PASS，無新增的失敗或 skip

- [ ] **Step 2: 型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤（注意 `npm run build` 底層就是跑這個；不要用 `tsc --noEmit`，根目錄 `tsconfig.json` 是 solution file，`--noEmit` 那樣跑會誤判成「零錯誤」）

- [ ] **Step 3: Lint**

Run: `npm run lint`
Expected: 無錯誤

- [ ] **Step 4: 若以上任一步驟失敗，回到對應 Task 修正，不要略過**

- [ ] **Step 5: 若全部通過，不需要額外 commit**（每個 Task 已經各自 commit 過）；跟使用者回報完成狀態，附上跑過的驗證指令與結果。
