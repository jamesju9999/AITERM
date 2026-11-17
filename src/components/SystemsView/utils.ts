import type { Translations } from "../../lib/i18n";

export function hostOf(url: string): string {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return "";
  }
}

export function slug(name: string): string {
  const s = name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  return s || `sys-${Math.random().toString(36).slice(2, 8)}`;
}

/** 顯示用的模型名稱（haiku → Haiku）；不是常見別名就原樣顯示。 */
export function modelName(t: Translations, m: string | null | undefined): string {
  if (!m) return t.sys_model_inherit_claude;
  const known: Record<string, string> = { haiku: t.sys_model_name_haiku, sonnet: t.sys_model_name_sonnet, opus: t.sys_model_name_opus };
  return known[m] ?? m;
}
