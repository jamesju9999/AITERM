import type { AgentKind } from "../../ipc/agents";

/** 使用者自己存的角色。跨專案共用，存在這台機器的 localStorage。 */
export interface CustomRole {
  id: string;
  kind: AgentKind;
  name: string;
  prefix: string;
}

const KEY = "aiterm.agentCanvas.customRoles";

export function loadCustomRoles(): CustomRole[] {
  try {
    const raw = localStorage.getItem(KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (r): r is CustomRole =>
        !!r && typeof r.id === "string" && typeof r.name === "string" && typeof r.prefix === "string" && typeof r.kind === "string",
    );
  } catch {
    return [];
  }
}

function persist(roles: CustomRole[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(roles));
  } catch {
    // 私密視窗等情況存不了：這次對話框裡照樣可以用，只是下次不會出現。
  }
}

/** 同種類、同名稱就覆寫（等於「編輯」），否則新增。回傳更新後的清單。 */
export function saveCustomRole(role: Omit<CustomRole, "id">): CustomRole[] {
  const all = loadCustomRoles();
  const name = role.name.trim();
  const existing = all.find((r) => r.kind === role.kind && r.name === name);
  const next = existing
    ? all.map((r) => (r === existing ? { ...r, prefix: role.prefix } : r))
    : [...all, { ...role, name, id: crypto.randomUUID() }];
  persist(next);
  return next;
}

export function deleteCustomRole(id: string): CustomRole[] {
  const next = loadCustomRoles().filter((r) => r.id !== id);
  persist(next);
  return next;
}
