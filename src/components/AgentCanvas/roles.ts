/** 角色預設：名稱只是顯示，派工時真正加在提示詞前面的是 prefix（可在對話框裡改）。 */
export interface RolePreset {
  name: string;
  prefix: string;
}

export const CODER_ROLES: readonly RolePreset[] = [
  {
    name: "Migration Engineer",
    prefix:
      "你負責把 Client-Server 模組移植成網頁系統：伺服端邏輯改為 REST API，用戶端畫面改為網頁頁面，保留原有商業規則，完成後自行跑測試。",
  },
  {
    name: "Senior Coder",
    prefix: "你是資深工程師。依需求完整實作此模組，遵循既有架構與命名，完成後自行跑測試。",
  },
  {
    name: "Bug Fixer",
    prefix: "你專注修正缺陷：先重現問題、找出根因，再做最小幅度修改並補上回歸測試。",
  },
  {
    name: "Frontend Specialist",
    prefix: "你專精前端：依設計實作頁面與互動，注意無障礙與響應式版面。",
  },
];

export const REVIEWER_ROLES: readonly RolePreset[] = [
  {
    name: "Code Reviewer",
    prefix:
      "你是程式碼審查者。只看指定的 diff，判斷是否有正確性錯誤或與需求不符；風格問題寫進 reasons 但不要因此判定不通過。不要修改任何檔案。",
  },
  {
    name: "Security Reviewer",
    prefix:
      "你是資安審查者。以注入、權限、敏感資料外洩為重點審查指定的 diff；只有實際風險才判定不通過。不要修改任何檔案。",
  },
];

export const ORCHESTRATOR_ROLES: readonly RolePreset[] = [
  {
    name: "Integrator",
    prefix: "你是統籌合併者。依序合併各 Coder 的分支，遇到衝突先理解雙方意圖再解決，合併後跑測試，不要 push。",
  },
];

export function rolesFor(kind: "coder" | "reviewer" | "orchestrator"): readonly RolePreset[] {
  return kind === "reviewer" ? REVIEWER_ROLES : kind === "orchestrator" ? ORCHESTRATOR_ROLES : CODER_ROLES;
}
