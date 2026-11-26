import type { ReactNode } from "react";

export function TaskColumn({
  status,
  title,
  count,
  highlighted = false,
  headerAction,
  sortControl,
  sortExtra,
  children,
}: {
  status: string;
  title: string;
  count: number;
  /** True while a card is being dragged over this column (see index.tsx's
   * mouse-event-based drag — there is no native dragover to key off). */
  highlighted?: boolean;
  /** 放在計數右邊的欄位層級動作（目前只有「已完成」欄的封存全部）。 */
  headerAction?: ReactNode;
  /** 排序選單，放在計數左邊。 */
  sortControl?: ReactNode;
  /** 排序選單旁的附加選項（目前是「不分組」）。 */
  sortExtra?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div
      className={`task-column${highlighted ? " task-column--drop-target" : ""}`}
      data-testid={`column-${status}`}
      // 驅動欄位頂端那條狀態色線（見 index.css）。刻意不重用上面的
      // data-testid：testid 是給測試用的鉤子，拿它當樣式選擇器會讓兩者
      // 綁死，改測試就可能不小心改掉外觀。比照卡片的 data-task-status。
      data-column-status={status}
    >
      <div className="task-column-head">
        <span className="task-column-title">{title}</span>
        {/* 排序等控制項自成一組：欄位夠寬時跟標題同一列，太窄時整組換到第二列（見 index.css 的 @container）。 */}
        <div className="task-column-tools">
          {sortControl}
          {sortExtra}
        </div>
        <div className="task-column-head-right">
          <span className="task-column-count">{count}</span>
          {headerAction}
        </div>
      </div>
      <div className="task-column-body">{children}</div>
    </div>
  );
}
