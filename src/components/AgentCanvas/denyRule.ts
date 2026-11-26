const RULE_RE = /^Bash\(.+\)$/;

/** 前端先擋明顯不合法的禁止規則，省一趟往返；server 還是會再檢查一次。回 null 表示沒問題。 */
export function ruleProblem(rule: string): string | null {
  const t = rule.trim();
  if (!RULE_RE.test(t)) return "format";
  if (t.includes(",") || t.includes("\n")) return "chars";
  return null;
}
