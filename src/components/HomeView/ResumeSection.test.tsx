import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { ResumeSection } from "./ResumeSection";
import { RECENT_PROJECTS_KEY } from "../../lib/recentProjects";
import { LRM } from "../../lib/pathUtils";
import type { Tab } from "../TabBar";

// ResumeSection 掛載就會查資料庫連線清單（用來把已連線的資料庫卡片補上
// 連線名稱／database@host）。不 mock 的話會真的打 Tauri IPC，在 jsdom 下
// 變成一個沒人管的 rejection，測試結果會取決於它何時被拒絕——跟
// index.test.tsx 對 usageSummary 的處理是同一個理由。
const dbListConnectionsMock = vi.fn();
vi.mock("../../ipc/db", () => ({
  dbListConnections: (...args: unknown[]) => dbListConnectionsMock(...args),
}));

function renderResume(tabs: Tab[], onSelectTab = vi.fn(), onOpenProject = vi.fn()) {
  const { container } = render(
    <LocaleProvider>
      <ResumeSection tabs={tabs} onSelectTab={onSelectTab} onOpenProject={onOpenProject} />
    </LocaleProvider>,
  );
  return { container, onSelectTab, onOpenProject };
}

beforeEach(() => {
  localStorage.clear();
  dbListConnectionsMock.mockReset();
  // 預設回傳空清單：既有測試大多不關心資料庫連線資訊，給一個確定會 resolve
  // 的值，避免每個既有測試都要多等一輪 microtask。
  dbListConnectionsMock.mockResolvedValue([]);
});

describe("ResumeSection", () => {
  it("分頁卡片顯示工作目錄與 AI 摘要", () => {
    const { container } = renderResume([
      { id: "t1", title: "Tab 1", type: "terminal", cwd: "/repo/aiterm", aiSummary: "跑了建置" },
    ]);
    // getByText 是完整比對，cwd 前後多了不可見的 LRM，改用 textContent 比對。
    expect(container.querySelector(".home-resume-cwd")?.textContent).toBe(`${LRM}/repo/aiterm${LRM}`);
    expect(screen.getByText("跑了建置")).toBeInTheDocument();
  });

  // direction:rtl 加 text-overflow:ellipsis 會把路徑開頭的中性符號（"/"）排到
  // 視覺尾端，已用無頭瀏覽器截圖實測重現過；修法是在文字前後包不可見的
  // LRM（U+200E）。這裡測的是「字串本身沒被改壞」——把 LRM 拿掉之後，內容
  // 仍是完整、未經改動的路徑。jsdom 不做 bidi 版面配置，所以測不到、也不宣
  // 稱測到「視覺上是不是真的從開頭省略」，那部分只有使用者眼睛看得到。
  it("cwd 拿掉 LRM 之後仍是完整路徑（CSS 省略、bidi 修正都不影響 DOM 文字）", () => {
    const longPath = "/Users/jamesju/Documents/GitHub/AITERM/src/components/HomeView";
    const { container } = renderResume([
      { id: "t1", title: "Tab 1", type: "terminal", cwd: longPath },
    ]);
    const cwdEl = container.querySelector(".home-resume-cwd");
    const lrmPattern = new RegExp(LRM, "g");
    expect(cwdEl?.textContent?.replace(lrmPattern, "")).toBe(longPath);
  });

  // cwd 只對終端機分頁有意義，aiSummary 只有跑過指令的分頁才有。
  // 沒有的欄位就不要留空位。用終端機分頁而非資料庫分頁，避免跟「尚未連線」
  // 標籤的測試混在一起。
  it("沒有 cwd 或摘要的分頁只顯示標題", () => {
    const { container } = renderResume([{ id: "t1", title: "終端機", type: "terminal" }]);
    expect(screen.getByText("終端機")).toBeInTheDocument();
    expect(container.querySelector(".home-resume-cwd")).toBeNull();
    expect(container.querySelector(".home-resume-summary")).toBeNull();
    expect(container.querySelector(".home-resume-chip")).toBeNull();
  });

  // 左邊框色靠 CSS 自訂屬性 --card-color 傳進去（跟 LaunchGrid.tsx 同一套
  // 機制）。分兩筆分別測一般終端機與 Claude Code，才擋得住「查色函式退化
  // 成只用 type find」這種讓 Claude Code 分頁拿到綠色的錯誤。
  it("分頁卡片帶有分頁類型專屬色的 --card-color", () => {
    const { container } = renderResume([
      { id: "t1", title: "終端機", type: "terminal" },
      { id: "t2", title: "Claude Code", type: "terminal", claudeBridge: "explicit" },
    ]);
    const cards = container.querySelectorAll(".home-resume-card");
    expect((cards[0] as HTMLElement).style.getPropertyValue("--card-color")).toBe("#4ade80");
    expect((cards[1] as HTMLElement).style.getPropertyValue("--card-color")).toBe("#d97757");
  });

  it("點分頁卡片會切到該分頁", () => {
    const { onSelectTab } = renderResume([{ id: "t7", title: "Tab 7", type: "terminal" }]);
    fireEvent.click(screen.getByText("Tab 7"));
    expect(onSelectTab).toHaveBeenCalledWith("t7");
  });

  // 沒有連線的資料庫分頁原本整張卡片空白得看不出狀態，現在要顯示標籤。
  it("沒有連線的資料庫分頁顯示「尚未連線」", () => {
    renderResume([{ id: "t1", title: "資料庫", type: "database" }]);
    expect(screen.getByText("尚未連線")).toBeInTheDocument();
  });

  // 兩條都要，否則「一律顯示」也會通過只測「沒連線時顯示」的那一條。
  it("已連線的資料庫分頁不顯示「尚未連線」", () => {
    renderResume([{ id: "t1", title: "資料庫", type: "database", dbConnectionId: "conn-1" }]);
    expect(screen.queryByText("尚未連線")).not.toBeInTheDocument();
  });

  it("已連線的資料庫分頁顯示連線名稱與 database@host", async () => {
    dbListConnectionsMock.mockResolvedValue([
      {
        id: "conn-1", name: "PROD_PG", db_type: "postgresql",
        host: "10.0.1.5", port: 5432, database: "postgres",
        username: "admin", is_connected: true,
      },
    ]);
    renderResume([{ id: "t1", title: "資料庫", type: "database", dbConnectionId: "conn-1" }]);
    // 首頁必須先畫出來、資料到了再補上，所以用 findByText 等非同步的
    // dbListConnections() 回來，而不是 getByText 立即斷言。
    expect(await screen.findByText("PROD_PG")).toBeInTheDocument();
    expect(screen.getByText("postgres@10.0.1.5")).toBeInTheDocument();
  });

  // SQLite 是檔案型資料庫：host 存的是檔案路徑，database 是空字串（實測過
  // DatabaseConnectionsPage.tsx 的表單）。用 db_type 分流、不是「host 是否
  // 為空」——sqlite 的 host 反而非空。這裡只該顯示檔名，不含孤零零的 "@"。
  it("SQLite 連線只顯示檔名，不含 @", async () => {
    dbListConnectionsMock.mockResolvedValue([
      {
        id: "conn-2", name: "SQL-Lite-Chinook", db_type: "sqlite",
        host: "/Users/jamesju/Library/DBeaverData/workspace6/.metadata/sample-database-sqlite-1/Chinook.db",
        port: 0, database: "", username: "", is_connected: true,
      },
    ]);
    renderResume([{ id: "t1", title: "資料庫", type: "database", dbConnectionId: "conn-2" }]);
    expect(await screen.findByText("Chinook.db")).toBeInTheDocument();
    expect(screen.queryByText(/^@/)).not.toBeInTheDocument();
    expect(screen.queryByText(/DBeaverData/)).not.toBeInTheDocument();
  });

  // jsdom 不做版面配置，測不到「有沒有真的視覺爆框」，也不載入 index.css
  // （這個測試檔繞過 index.tsx 直接測 ResumeSection，CSS 從沒被載入過，
  // getComputedStyle 測不出真實樣式）。這裡只鎖「內容本身完整、正確」——
  // 卡片會不會撐破、省略號有沒有出現在對的位置，都只有使用者眼睛看得到。
  it("極長的連線名稱／資料庫名內容仍完整（不斷言視覺樣式，jsdom 測不到）", async () => {
    const longName = "A_Very_Long_Connection_Name_That_Someone_Might_Actually_Type_In_The_Form";
    const longDb = "a_very_long_database_name_here";
    const longHost = "some.really.long.hostname.example.internal";
    dbListConnectionsMock.mockResolvedValue([
      {
        id: "conn-3", name: longName, db_type: "postgresql",
        host: longHost, port: 5432, database: longDb,
        username: "admin", is_connected: true,
      },
    ]);
    const { container } = renderResume([
      { id: "t1", title: "資料庫", type: "database", dbConnectionId: "conn-3" },
    ]);
    await screen.findByText(longName);
    // conn.name 用 .home-resume-cwd（跟終端機卡片的 cwd 同一個位置／樣式），
    // 但它不是路徑，沒有套 withLrmGuard；database@host 用 .home-resume-summary。
    expect(container.querySelector(".home-resume-cwd")?.textContent).toBe(longName);
    expect(container.querySelector(".home-resume-summary")?.textContent).toBe(`${longDb}@${longHost}`);
  });

  // 連線可能已被刪除但分頁還在——這種情況不能顯示「未知連線」之類的雜訊。
  it("找不到對應 id 時只顯示標題，不顯示任何替代文字", async () => {
    const resolved = Promise.resolve([]);
    dbListConnectionsMock.mockReturnValue(resolved);
    const { container } = renderResume([
      { id: "t1", title: "資料庫", type: "database", dbConnectionId: "deleted-conn" },
    ]);
    // 等 dbListConnections() 的 .then() 真的跑完，state 更新反映到畫面上，
    // 而不是靠計時器賭一個時間點。
    await act(async () => { await resolved; });
    expect(screen.getByText("資料庫")).toBeInTheDocument();
    expect(container.querySelector(".home-resume-cwd")).toBeNull();
    expect(container.querySelector(".home-resume-summary")).toBeNull();
  });

  // 查詢失敗不能讓其他卡片陪葬——這條要真的斷言「別的卡片還在」，不是只測
  // 資料庫卡片本身沒爆炸。
  it("查詢失敗時資料庫分頁只顯示標題，且其他卡片照常渲染", async () => {
    const rejected = Promise.reject(new Error("boom"));
    dbListConnectionsMock.mockReturnValue(rejected);
    renderResume([
      { id: "t1", title: "終端機", type: "terminal", cwd: "/repo/aiterm" },
      { id: "t2", title: "資料庫", type: "database", dbConnectionId: "conn-1" },
    ]);
    await act(async () => { await rejected.catch(() => {}); });

    const dbCard = screen.getByText("資料庫").closest(".home-resume-card")!;
    expect(dbCard.querySelector(".home-resume-cwd")).toBeNull();
    expect(dbCard.querySelector(".home-resume-summary")).toBeNull();

    // 其他卡片（終端機）不受影響，照常顯示它的 cwd。
    expect(screen.getByText("終端機")).toBeInTheDocument();
    const terminalCard = screen.getByText("終端機").closest(".home-resume-card")!;
    expect(terminalCard.querySelector(".home-resume-cwd")?.textContent).toBe(`${LRM}/repo/aiterm${LRM}`);
  });

  it("列出最近的專案目錄，資料夾名在前", () => {
    localStorage.setItem(
      RECENT_PROJECTS_KEY,
      JSON.stringify([{ path: "/repo/aiterm", lastUsedAt: 1 }]),
    );
    renderResume([]);
    expect(screen.getByText("aiterm")).toBeInTheDocument();
  });

  // 規格給的具體例子：長路徑要顯示資料夾名，不是完整路徑。
  it("最近專案顯示資料夾名而非完整路徑", () => {
    localStorage.setItem(
      RECENT_PROJECTS_KEY,
      JSON.stringify([{ path: "/a/b/03_FCS_Nuntio", lastUsedAt: 1 }]),
    );
    renderResume([]);
    expect(screen.getByText("03_FCS_Nuntio")).toBeInTheDocument();
    expect(screen.queryByText("/a/b/03_FCS_Nuntio")).not.toBeInTheDocument();
  });

  // 父路徑欄位跟 cwd 一樣套了 direction:rtl，一樣要包 LRM。這裡同樣只鎖
  // 「拿掉 LRM 後內容沒被改壞」，不宣稱測到視覺順序。
  it("最近專案的父路徑拿掉 LRM 之後仍是完整父路徑", () => {
    localStorage.setItem(
      RECENT_PROJECTS_KEY,
      JSON.stringify([{ path: "/a/b/03_FCS_Nuntio", lastUsedAt: 1 }]),
    );
    const { container } = renderResume([]);
    const parentEl = container.querySelector(".home-recent-parent");
    const lrmPattern = new RegExp(LRM, "g");
    // abbreviateHome 在測試環境（無 Tauri IPC）拿不到家目錄，會原樣返回，
    // 這裡斷言的正是「原樣返回的內容」，跟 F 項的非同步決策一致。
    expect(parentEl?.textContent?.replace(lrmPattern, "")).toBe("/a/b");
  });

  it("點最近專案會用該路徑呼叫 onOpenProject", () => {
    localStorage.setItem(
      RECENT_PROJECTS_KEY,
      JSON.stringify([{ path: "/repo/aiterm", lastUsedAt: 1 }]),
    );
    const { onOpenProject } = renderResume([]);
    // 內容現在拆成資料夾名／父路徑兩個 span，用可及名稱（子節點文字合併）找按鈕。
    fireEvent.click(screen.getByRole("button", { name: /aiterm/ }));
    expect(onOpenProject).toHaveBeenCalledWith("/repo/aiterm");
  });

  // 多筆才分得出「列出全部」與「只列第一筆」。
  it("多個最近專案全部列出", () => {
    localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify([
      { path: "/a", lastUsedAt: 2 },
      { path: "/b", lastUsedAt: 1 },
    ]));
    const { container } = renderResume([]);
    expect(container.querySelectorAll(".home-recent-item")).toHaveLength(2);
  });

  it("最近專案標題顯示筆數", () => {
    localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify([
      { path: "/a", lastUsedAt: 2 },
      { path: "/b", lastUsedAt: 1 },
    ]));
    renderResume([]);
    expect(screen.getByText("2 個")).toBeInTheDocument();
  });

  it("沒有分頁也沒有最近專案時顯示空狀態", () => {
    renderResume([]);
    expect(screen.getByText("還沒有可以接續的工作")).toBeInTheDocument();
  });
});
