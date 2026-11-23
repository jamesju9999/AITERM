/**
 * 白名單與項目的寄件者代號存的是小寫（比對不分大小寫），但 LINE 的 userId 開頭是大寫 `U`，
 * 畫面上要顯示成使用者在 LINE 後台看到的樣子。比對與回覆都不受影響（回覆時後端會還原大寫 U）。
 */
export function displayAddress(a: string): string {
  const m = /^line:([ucr])([0-9a-f]{32})$/i.exec(a);
  return m ? `line:${m[1].toUpperCase()}${m[2]}` : a;
}

/** 白名單接受電子郵件地址、Telegram 聊天室 `tg:<chat_id>`，或 LINE 使用者／群組／聊天室（`line:<編號>`，或直接貼 U／C／R 開頭的編號；與後端 `valid_address` 一致）。 */
export function validAddress(a: string): boolean {
  const v = a.trim().toLowerCase();
  if (v.startsWith("tg:")) return /^tg:-?\d+$/.test(v);
  if (v.startsWith("line:")) return /^line:[ucr][0-9a-f]{32}$/.test(v);
  if (/^[ucr][0-9a-f]{32}$/.test(v)) return true;
  return /^[^@\s<>]+@[^@\s<>]+\.[^@\s<>]+$/.test(v);
}
