import { formatAiError, type AiError } from "../../ipc/ai";

/** Tauri 的錯誤是物件而不是 Error——不能 String(e)，會變成 [object Object]。 */
export function describeError(e: unknown): string {
  if (e && typeof e === "object") {
    if ("kind" in e) return formatAiError(e as AiError);
    if ("message" in e) return String((e as { message: unknown }).message);
  }
  return typeof e === "string" ? e : "unknown";
}
