/** 白名單接受電子郵件地址，或 Telegram 聊天室 `tg:<chat_id>`（與後端 `valid_address` 一致）。 */
export function validAddress(a: string): boolean {
  const v = a.trim().toLowerCase();
  if (v.startsWith("tg:")) return /^tg:-?\d+$/.test(v);
  return /^[^@\s<>]+@[^@\s<>]+\.[^@\s<>]+$/.test(v);
}
