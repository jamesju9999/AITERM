import type { TaskRow } from "../../ipc/tasks";
import type { Translations } from "../../lib/i18n";

/**
 * 自動產生的卡片（審查／修正／拆解／合併）在後端建立時標題是中文——後端不
 * 知道介面語系。顯示時依卡片種類把標題拆開、用目前語系重組；格式對不上
 * （例如使用者自己改過標題）就原樣顯示。格式來源：tasks/workflow.rs。
 */
export function localizedCardTitle(card: Pick<TaskRow, "title" | "card_kind">, t: Translations): string {
  const title = card.title;
  switch (card.card_kind) {
    case "review": {
      const m = /^審查(規格)?：(.+?)(（修正後）)?$/.exec(title);
      return m ? t.ac_title_review(m[2], !!m[1], !!m[3]) : title;
    }
    case "fix": {
      const m = /^修正：(.+)（第 (\d+) 輪）$/.exec(title);
      return m ? t.ac_title_fix(m[1], Number(m[2])) : title;
    }
    case "plan": {
      const m = /^拆解規格：(.+)$/.exec(title);
      return m ? t.ac_title_plan(m[1]) : title;
    }
    case "merge": {
      const m = /^合併 (\d+) 個(?:工程師| ?Coder ?)分支$/.exec(title);
      return m ? t.ac_title_merge(Number(m[1])) : title;
    }
    default:
      return title;
  }
}
