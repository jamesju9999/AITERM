import { useEffect } from "react";
import { useLocale } from "../contexts/LocaleContext";
import { setTrayLabels } from "../ipc/resident";

/**
 * 托盤（選單列）選單是 Rust 端建的，它不知道使用者選了哪個語系。
 * 掛載與語系改變時，把目前語系的文字送過去。
 * 失敗一律靜默：托盤沒建成（見 resident::hide_for_close）或非桌面環境時，標籤本來就沒有意義。
 */
export function useTrayLabels() {
  const { t } = useLocale();
  useEffect(() => {
    void setTrayLabels(t.tray_show, t.tray_new_tab, t.tray_quit).catch(() => {});
  }, [t.tray_show, t.tray_new_tab, t.tray_quit]);
}
