import { useLocale } from "../../contexts/LocaleContext";

const KNOWN = ["haiku", "sonnet", "opus"] as const;

interface Props {
  value: string | null | undefined;
  onChange: (m: string | null) => void;
  /** 空值時顯示的文字（系統層是「沿用 Claude Code」，任務層是「沿用系統設定」）。 */
  inheritLabel: string;
  disabled?: boolean;
  id?: string;
}

/** AI 模型三選一（外加沿用）。已存的是自訂完整名稱時也會列出來，不會被悄悄改掉。 */
export function ModelSelect({ value, onChange, inheritLabel, disabled, id }: Props) {
  const { t } = useLocale();
  const custom = value && !(KNOWN as readonly string[]).includes(value) ? value : null;
  return (
    <select id={id} aria-label={t.sys_model} value={value ?? ""} disabled={disabled} onChange={(e) => onChange(e.target.value || null)}>
      <option value="">{inheritLabel}</option>
      <option value="haiku">{t.sys_model_haiku}</option>
      <option value="sonnet">{t.sys_model_sonnet}</option>
      <option value="opus">{t.sys_model_opus}</option>
      {custom && <option value={custom}>{custom}</option>}
    </select>
  );
}
