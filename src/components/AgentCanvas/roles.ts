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
