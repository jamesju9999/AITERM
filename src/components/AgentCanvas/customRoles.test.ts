import { beforeEach, describe, expect, it } from "vitest";

import { deleteCustomRole, loadCustomRoles, saveCustomRole } from "./customRoles";

beforeEach(() => localStorage.clear());

describe("customRoles", () => {
  it("新增、同名覆寫（編輯）、刪除", () => {
    saveCustomRole({ kind: "coder", name: "SAP 顧問", prefix: "v1" });
    saveCustomRole({ kind: "coder", name: "SAP 顧問 ", prefix: "v2" });
    let all = loadCustomRoles();
    expect(all).toHaveLength(1);
    expect(all[0].prefix).toBe("v2");

    saveCustomRole({ kind: "reviewer", name: "SAP 顧問", prefix: "r" });
    expect(loadCustomRoles()).toHaveLength(2);

    all = deleteCustomRole(all[0].id);
    expect(all.map((r) => r.kind)).toEqual(["reviewer"]);
  });

  it("壞掉的資料不會讓對話框炸掉", () => {
    localStorage.setItem("aiterm.agentCanvas.customRoles", "{not json");
    expect(loadCustomRoles()).toEqual([]);
    localStorage.setItem("aiterm.agentCanvas.customRoles", JSON.stringify([{ id: 1 }, { id: "a", kind: "coder", name: "n", prefix: "p" }]));
    expect(loadCustomRoles()).toHaveLength(1);
  });
});
