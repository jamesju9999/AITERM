import { useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";

interface Props {
  /** 輸入框的初始內容。編輯既有條目時由呼叫端帶入那一筆的別名；只在這一列
   *  出現的當下讀一次，之後由使用者掌控。 */
  initialName?: string;
  /** 使用者按「儲存」。`name` 是輸入框的原始內容，**可能是空字串**——要不要
   *  退回預設別名由呼叫端決定。 */
  onSave: (name: string) => void;
  /** 使用者按「不用」。連線本身已經成立，這只代表不存。 */
  onSkip: () => void;
}

/**
 * 連線成功後「要把這台存進地址簿嗎？」那一列。
 *
 * **刻意只負責收集別名、回報按了哪個鈕。** 決定這次是「新增」還是「更新」
 * 的 `confirmSave` 留在 `ConnectDialog`：它依賴 `editingId`、錯誤訊息與清單
 * 重抓，而且那段驗證是承重的——`editingId` 曾經從四個不同的出口殘留下來，
 * 把資料寫進錯的條目，最後才改成在使用當下重新驗證。把它搬進來只會把耦合
 * 換個地方藏起來，不會消除它。
 *
 * 別名的輸入狀態放在這裡：它的生命週期剛好就是這一列的生命週期——這一列
 * 收起來，輸入的內容就該跟著消失；存檔驗證失敗而這一列還留著時，內容也該
 * 留著讓使用者直接重按。
 */
export function SaveHostPrompt({ initialName = "", onSave, onSkip }: Props) {
  const { t } = useLocale();
  const [name, setName] = useState(initialName);
  return (
    <div className="aiterm-connect__save">
      <div>{t.connect_save_prompt}</div>
      <label className="aiterm-connect__label" htmlFor="aiterm-connect-savename">
        {t.connect_save_name_label}
      </label>
      <input
        id="aiterm-connect-savename"
        className="aiterm-connect__text"
        type="text"
        value={name}
        onChange={(e) => setName(e.target.value)}
      />
      <div className="aiterm-connect__actions">
        <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onSkip}>
          {t.connect_save_skip}
        </button>
        <button
          className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
          onClick={() => onSave(name)}
        >
          {t.connect_save_confirm}
        </button>
      </div>
    </div>
  );
}
