import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

interface Props {
  elevated: boolean;
}

/** 分頁目前是否處於提權模式的徽章，仿 ShellWarningBadge 的樣式與掛載模式。 */
export function ElevationBadge({ elevated }: Props) {
  const { t } = useLocale();
  if (!elevated) return null;
  return (
    <span className="aiterm-elevation-badge" title={t.elevation_badge_label}>
      ⚡ {t.elevation_badge_label}
    </span>
  );
}
