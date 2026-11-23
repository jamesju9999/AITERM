/** 白名單接受電子郵件地址、Telegram 聊天室 `tg:<chat_id>`，或 LINE 使用者 `line:<userId>`（與後端 `valid_address` 一致）。 */
export function validAddress(a: string): boolean {
  const v = a.trim().toLowerCase();
  if (v.startsWith("tg:")) return /^tg:-?\d+$/.test(v);
  if (v.startsWith("line:")) return /^line:u[0-9a-f]{32}$/.test(v);
  return /^[^@\s<>]+@[^@\s<>]+\.[^@\s<>]+$/.test(v);
}
