import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { listRecentProjects } from "../../lib/recentProjects";
import { splitPathTail, withLrmGuard } from "../../lib/pathUtils";
import { abbreviateHome } from "../../lib/homeDir";
import { dbListConnections, type DbConnectionInfo } from "../../ipc/db";
import type { Tab } from "../TabBar";
import { SectionTitle } from "./SectionTitle";
import { HistoryIcon, FolderIcon } from "../Icons";

interface Props {
  tabs: Tab[];
  onSelectTab: (id: string) => void;
  onOpenProject: (path: string) => void;
}

export function ResumeSection({ tabs, onSelectTab, onOpenProject }: Props) {
  const { t } = useLocale();
  // 首頁每次顯示都重讀：清單只在 cwd 變動時寫入，不需要訂閱機制。
  const projects = listRecentProjects();

  // 已連線的資料庫卡片要顯示連線名稱 + database@host。首頁必須先畫出來、
  // 資料到了再補上（不能整區等資料到齊才顯示），所以用 null 表示「還沒
  // 回來」——卡片在這個狀態下只顯示標題，不阻塞、不閃爍。查詢失敗就讓它
  // 停在 null，跟 useBridgeRunning／LaunchGrid 的 bridgeStatus() 同一種
  // 「.catch(() => {}) 吞掉」模式：資料庫卡片維持只有標題，其他卡片不受影響。
  const [connections, setConnections] = useState<DbConnectionInfo[] | null>(null);
  useEffect(() => {
    let cancelled = false;
    dbListConnections()
      .then((list) => { if (!cancelled) setConnections(list); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, []);

  // 已連線的資料庫分頁：比照終端機卡片「目錄＋摘要」兩行，重用同一組 class
  // 讓字級／顏色階層一致。找不到對應 id（還沒載入，或連線已被刪除）就回傳
  // null——不生造「未知連線」這種雜訊文字，卡片維持只有標題。
  //
  // SQLite 是檔案型資料庫，形狀跟其他型別不一樣：實測過（見
  // DatabaseConnectionsPage.tsx 的表單）檔案路徑存在 `host` 欄位、
  // `database` 是空字串——用 `db_type === "sqlite"` 分流，不是「host 是否
  // 為空」（sqlite 的 host 其實非空，是路徑本身；反而 database 才是空的）。
  // 這裡只顯示檔名，重用「最近的專案目錄」已經在用的 splitPathTail，不重寫
  // 一份路徑解析。
  function renderDbConnectionInfo(tab: Tab) {
    if (tab.type !== "database" || !tab.dbConnectionId) return null;
    const conn = connections?.find((c) => c.id === tab.dbConnectionId);
    if (!conn) return null;
    const summary = conn.db_type === "sqlite"
      ? splitPathTail(conn.host).name
      : `${conn.database}@${conn.host}`;
    return (
      <>
        <span className="home-resume-cwd">{conn.name}</span>
        <span className="home-resume-summary">{summary}</span>
      </>
    );
  }

  if (tabs.length === 0 && projects.length === 0) {
    return (
      <section className="home-section">
        <SectionTitle icon={<HistoryIcon size={17} />}>{t.home_resume_title}</SectionTitle>
        <p className="home-empty">{t.home_resume_empty}</p>
      </section>
    );
  }

  return (
    <section className="home-section">
      <SectionTitle
        icon={<HistoryIcon size={17} />}
        count={tabs.length > 0 ? t.home_resume_count(tabs.length) : undefined}
      >
        {t.home_resume_title}
      </SectionTitle>

      <div className="home-resume-grid">
        {tabs.map((tab) => (
          <button key={tab.id} className="home-resume-card" onClick={() => onSelectTab(tab.id)}>
            <span className="home-resume-title">{tab.title}</span>
            {/* 前後包 LRM：direction:rtl 會把開頭的 "/" 排到視覺尾端，實測驗證過
                （見 pathUtils.ts 的 LRM 常數說明），不是誤植的看不見字元。 */}
            {tab.cwd && <span className="home-resume-cwd">{withLrmGuard(tab.cwd)}</span>}
            {/* 這個 session 的新摘要優先；沒有就顯示上次那份。區塊標題已經
                說了是「接續上次的工作」，顯示舊摘要在這裡是成立的。 */}
            {(tab.aiSummary ?? tab.lastSessionSummary) && (
              <span className="home-resume-summary">{tab.aiSummary ?? tab.lastSessionSummary}</span>
            )}
            {/* 資料庫分頁還沒連線時，整張卡片原本會空白得看不出狀態。只做這一個
                狀態——其他分頁類型沒有對應的「未就緒」語意，連線名稱要額外 IPC。 */}
            {tab.type === "database" && !tab.dbConnectionId && (
              <span className="home-resume-chip">{t.home_resume_not_connected}</span>
            )}
            {renderDbConnectionInfo(tab)}
          </button>
        ))}
      </div>

      {projects.length > 0 && (
        <>
          <SectionTitle icon={<FolderIcon size={17} />} count={t.home_recent_count(projects.length)}>
            {t.home_recent_projects}
          </SectionTitle>
          <div className="home-recent-list">
            {projects.map((p) => {
              const { name, parent } = splitPathTail(p.path);
              return (
                <button key={p.path} className="home-recent-item" onClick={() => onOpenProject(p.path)}>
                  <span className="home-recent-chevron" aria-hidden="true">›</span>
                  <span className="home-recent-name">{name}</span>
                  {/* 前後包 LRM，理由同上（見 pathUtils.ts 的 LRM 常數說明）。 */}
                  <span className="home-recent-parent">{withLrmGuard(abbreviateHome(parent))}</span>
                </button>
              );
            })}
          </div>
        </>
      )}
    </section>
  );
}
