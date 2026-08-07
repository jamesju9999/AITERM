# Auto Update Badge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On app startup, automatically check GitHub for a newer version and show an orange dot badge on the ⚙ Settings button when one is available; pre-populate the About page with the result so the user doesn't need to manually press "檢查更新".

**Architecture:** Update check runs once in `AppRoutes` (App.tsx) via `useEffect` on mount. The result (`hasUpdate: boolean`, `latestVersion: string`) is prop-drilled two levels: `AppRoutes → TerminalApp → TabBar` for the badge, and `AppRoutes → SettingsView → AboutPage` for pre-filling status. No new Context needed.

**Tech Stack:** React 19, TypeScript, GitHub Tags REST API (already used in AboutPage), CSS absolute positioning for badge.

---

## File Map

| File | Change |
|------|--------|
| `src/App.tsx` | Add update-check `useEffect` + `updateInfo` state; pass props to `TerminalApp` and `SettingsView` |
| `src/components/TerminalApp.tsx` | Accept `hasUpdate?: boolean` prop; forward to `TabBar` |
| `src/components/TabBar/index.tsx` | Accept `hasUpdate?: boolean` prop; render `.update-badge` span on ⚙ button in both collapsed and expanded modes |
| `src/components/TabBar/index.css` | Add `.update-badge` style (orange absolute dot) |
| `src/components/Settings/SettingsView.tsx` | Accept `updateInfo?: UpdateInfo` prop; pass to `<AboutPage>` |
| `src/components/Settings/AboutPage.tsx` | Accept `initialStatus?: UpdateStatus` and `latestVersion?: string` props; seed state on mount |

---

## Task 1: Add update-check logic to App.tsx

**Files:**
- Modify: `src/App.tsx`

- [ ] **Step 1: Add `UpdateInfo` type and state to `AppRoutes`**

  Replace the top of `AppRoutes` function. Add the type above the function definition and `updateInfo` state + check logic inside it:

  ```tsx
  // src/App.tsx — add type above AppRoutes
  interface UpdateInfo {
    hasUpdate: boolean;
    latestVersion: string;
  }

  const TAGS_API = "https://api.github.com/repos/jamesju9999/AITERM/tags";

  function AppRoutes() {
    const navigate = useNavigate();
    const location = useLocation();
    const [ready, setReady] = useState(false);
    const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);

    // Auto-check for updates once on mount
    useEffect(() => {
      getVersion()
        .then(async (current) => {
          try {
            const res = await fetch(TAGS_API);
            if (!res.ok) return;
            const tags = await res.json() as { name: string }[];
            if (tags.length === 0) return;
            const latest = tags[0].name.replace(/^v/, "");
            const cur = current.replace(/^v/, "");
            if (latest !== cur) {
              setUpdateInfo({ hasUpdate: true, latestVersion: latest });
            }
          } catch {
            // silently ignore — update check is best-effort
          }
        })
        .catch(() => {});
    }, []);
    // ... rest of existing code unchanged
  ```

  Also add the missing import at the top of the file:

  ```tsx
  import { getVersion } from "@tauri-apps/api/app";
  ```

- [ ] **Step 2: Pass `updateInfo` as props to `TerminalApp` and `SettingsView`**

  In the JSX returned by `AppRoutes`, update the two component usages:

  ```tsx
  // TerminalApp — add hasUpdate prop
  <TerminalApp hasUpdate={updateInfo?.hasUpdate ?? false} />

  // SettingsView — add updateInfo prop
  <SettingsView updateInfo={updateInfo ?? undefined} />
  ```

- [ ] **Step 3: Type-check**

  ```bash
  cd /Users/jamesju/Documents/GitHub/AITERM
  npx tsc --noEmit 2>&1 | head -30
  ```

  Expected: errors only about `TerminalApp` and `SettingsView` not yet accepting the new props (will be fixed in Tasks 2 and 4). No other errors.

---

## Task 2: Thread `hasUpdate` through `TerminalApp` to `TabBar`

**Files:**
- Modify: `src/components/TerminalApp.tsx`

- [ ] **Step 1: Add `hasUpdate` prop to `TerminalApp`**

  Change the component signature from:

  ```tsx
  export function TerminalApp() {
  ```

  to:

  ```tsx
  interface TerminalAppProps {
    hasUpdate?: boolean;
  }

  export function TerminalApp({ hasUpdate = false }: TerminalAppProps) {
  ```

- [ ] **Step 2: Pass `hasUpdate` to `<TabBar>`**

  Find the `<TabBar` usage (around line 253) and add the prop:

  ```tsx
  <TabBar
    tabs={tabs}
    activeId={activeId}
    onSelect={setActiveId}
    onClose={handleCloseTab}
    onAdd={handleAddTab}
    onRename={handleRename}
    isSidebarOpen={isSidebarOpen}
    onToggle={toggleSidebar}
    width={sidebarWidth}
    pickerOpen={pickerOpen}
    onPickerSelect={handlePickerSelect}
    onPickerClose={() => setPickerOpen(false)}
    hasUpdate={hasUpdate}
  />
  ```

- [ ] **Step 3: Type-check**

  ```bash
  npx tsc --noEmit 2>&1 | head -20
  ```

  Expected: error about `TabBar` not accepting `hasUpdate` (fixed in Task 3). No new unrelated errors.

---

## Task 3: Add badge to `TabBar`

**Files:**
- Modify: `src/components/TabBar/index.tsx`
- Modify: `src/components/TabBar/index.css`

- [ ] **Step 1: Add `hasUpdate` to `TabBarProps` interface**

  In `src/components/TabBar/index.tsx`, add `hasUpdate?: boolean` to the interface:

  ```tsx
  export interface TabBarProps {
    tabs: Tab[];
    activeId: string;
    onSelect: (id: string) => void;
    onClose: (id: string) => void;
    onAdd: () => void;
    onRename?: (id: string, title: string) => void;
    isSidebarOpen: boolean;
    onToggle: () => void;
    width: number;
    pickerOpen?: boolean;
    onPickerSelect?: (type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs") => void;
    onPickerClose?: () => void;
    hasUpdate?: boolean;
  }
  ```

- [ ] **Step 2: Destructure `hasUpdate` in the function signature**

  Change:

  ```tsx
  export function TabBar({ tabs, activeId, onSelect, onClose, onAdd, onRename, isSidebarOpen, onToggle, width, pickerOpen, onPickerSelect, onPickerClose }: TabBarProps) {
  ```

  to:

  ```tsx
  export function TabBar({ tabs, activeId, onSelect, onClose, onAdd, onRename, isSidebarOpen, onToggle, width, pickerOpen, onPickerSelect, onPickerClose, hasUpdate = false }: TabBarProps) {
  ```

- [ ] **Step 3: Add badge to the collapsed ⚙ button**

  Replace the collapsed settings button:

  ```tsx
  // BEFORE:
  <button 
    className="aiterm-sidebar-toggle" 
    onClick={() => navigate("/settings")} 
    title={`${t.settings} (Ctrl+,)`}
    style={{ background: 'transparent', border: 'none', color: '#888', cursor: 'pointer', fontSize: '18px' }}
  >
    ⚙
  </button>

  // AFTER:
  <button 
    className="aiterm-sidebar-toggle" 
    onClick={() => navigate("/settings")} 
    title={`${t.settings} (Ctrl+,)`}
    style={{ background: 'transparent', border: 'none', color: '#888', cursor: 'pointer', fontSize: '18px', position: 'relative' }}
  >
    ⚙
    {hasUpdate && <span className="update-badge" aria-label="Update available" />}
  </button>
  ```

- [ ] **Step 4: Add badge to the expanded ⚙ footer item**

  Replace the footer settings div:

  ```tsx
  // BEFORE:
  <div
    className="aiterm-tab"
    style={{ padding: "0 8px" }}
    onClick={() => navigate("/settings")}
    title={`${t.settings} (Ctrl+,)`}
  >
    <span style={{ marginRight: "8px", fontSize: "16px" }}>⚙</span>
    <span className="aiterm-tab-title">{t.settings}</span>
  </div>

  // AFTER:
  <div
    className="aiterm-tab"
    style={{ padding: "0 8px" }}
    onClick={() => navigate("/settings")}
    title={`${t.settings} (Ctrl+,)`}
  >
    <span style={{ marginRight: "8px", fontSize: "16px", position: "relative", display: "inline-block" }}>
      ⚙
      {hasUpdate && <span className="update-badge" aria-label="Update available" />}
    </span>
    <span className="aiterm-tab-title">{t.settings}</span>
  </div>
  ```

- [ ] **Step 5: Add `.update-badge` CSS**

  Append to `src/components/TabBar/index.css`:

  ```css
  /* Update available badge — orange dot on the ⚙ icon */
  .update-badge {
    position: absolute;
    top: -3px;
    right: -4px;
    width: 8px;
    height: 8px;
    background: #f97316;
    border-radius: 50%;
    border: 1.5px solid #151515;
    pointer-events: none;
  }
  ```

- [ ] **Step 6: Type-check**

  ```bash
  npx tsc --noEmit 2>&1 | head -20
  ```

  Expected: no errors related to `TabBar` or `TerminalApp`. Possibly still an error about `SettingsView` (fixed in Task 4).

---

## Task 4: Thread `updateInfo` through `SettingsView` to `AboutPage`

**Files:**
- Modify: `src/components/Settings/SettingsView.tsx`
- Modify: `src/components/Settings/AboutPage.tsx`

- [ ] **Step 1: Add `UpdateInfo` type and prop to `SettingsView`**

  In `src/components/Settings/SettingsView.tsx`, add the type and prop:

  ```tsx
  // Add at the top of the file (after imports)
  interface UpdateInfo {
    hasUpdate: boolean;
    latestVersion: string;
  }

  // Change function signature from:
  export function SettingsView() {

  // to:
  export function SettingsView({ updateInfo }: { updateInfo?: UpdateInfo }) {
  ```

- [ ] **Step 2: Pass `updateInfo` to `<AboutPage>`**

  Change `{tab === "about" && <AboutPage />}` to:

  ```tsx
  {tab === "about" && (
    <AboutPage
      initialLatestVersion={updateInfo?.latestVersion}
    />
  )}
  ```

- [ ] **Step 3: Update `AboutPage` to accept and use the prop**

  In `src/components/Settings/AboutPage.tsx`, add prop and seed state on mount.

  Change the function signature from:

  ```tsx
  export function AboutPage() {
  ```

  to:

  ```tsx
  interface AboutPageProps {
    initialLatestVersion?: string;
  }

  export function AboutPage({ initialLatestVersion }: AboutPageProps) {
  ```

  Then seed `updateStatus` and `latestVersion` state from the prop on mount. Add a new `useEffect` right after the existing `getVersion` effect:

  ```tsx
  // Seed update state from parent if auto-check already ran
  useEffect(() => {
    if (!initialLatestVersion) return;
    setLatestVersion(initialLatestVersion);
    setUpdateStatus("available");
  }, [initialLatestVersion]);
  ```

- [ ] **Step 4: Type-check — should be clean**

  ```bash
  npx tsc --noEmit 2>&1 | head -20
  ```

  Expected: no errors.

---

## Task 5: Smoke-test and commit

- [ ] **Step 1: Run frontend tests**

  ```bash
  cd /Users/jamesju/Documents/GitHub/AITERM
  npm run test 2>&1 | tail -20
  ```

  Expected: all tests pass (no existing tests cover TabBar/About, so no regressions expected).

- [ ] **Step 2: Run lint**

  ```bash
  npm run lint 2>&1 | head -30
  ```

  Expected: no new errors.

- [ ] **Step 3: Commit**

  ```bash
  git add src/App.tsx \
    src/components/TerminalApp.tsx \
    src/components/TabBar/index.tsx \
    src/components/TabBar/index.css \
    src/components/Settings/SettingsView.tsx \
    src/components/Settings/AboutPage.tsx

  git commit -m "$(cat <<'EOF'
  feat(update): auto-check on startup + badge on settings icon

  - App.tsx: check GitHub tags API once on mount, hold updateInfo state
  - TabBar: show orange dot badge on ⚙ when hasUpdate=true (both collapsed/expanded)
  - AboutPage: pre-fill "available" status when parent already fetched result

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
  EOF
  )"
  ```
