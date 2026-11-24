// 連到問題收集 server 的輸入檢查（位址與邀請碼），與後端的規則一致。

/** 位址要能解析成 host[:port]；錯誤當場提示，不送出。 */
export function validServerAddr(input: string): boolean {
  const s = input.trim().replace(/^https:\/\//, "").replace(/\/$/, "");
  if (!s || /[\s/]/.test(s)) return false;
  const i = s.lastIndexOf(":");
  if (i === -1) return true;
  const port = s.slice(i + 1);
  return i > 0 && /^\d{1,5}$/.test(port) && Number(port) <= 65535;
}

/** 邀請碼：6 字元編號 + 16 字元祕密，忽略大小寫、空白與 `-`。 */
export function validInviteCode(input: string): boolean {
  const clean = input.replace(/[\s-]/g, "").toUpperCase();
  return /^[ABCDEFGHJKMNPQRSTUVWXYZ23456789]{22}$/.test(clean);
}
