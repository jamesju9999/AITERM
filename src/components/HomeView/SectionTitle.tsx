import type { ReactNode } from "react";

interface Props {
  icon: ReactNode;
  children: ReactNode;
  /** 右側的次要計數，例如「2 個分頁」。沒有就不顯示。 */
  count?: string;
}

/** 首頁各區塊共用的標題列：圖示 + 標題 + 靠右的次要計數。
 *  `ResumeSection` 的主標題與「最近的專案目錄」子標題也共用這個元件，
 *  讓兩層標題視覺一致。 */
export function SectionTitle({ icon, children, count }: Props) {
  return (
    <h2 className="home-section-title">
      <span className="home-section-icon">{icon}</span>
      {children}
      {count && <span className="home-section-count">{count}</span>}
    </h2>
  );
}
