# AITERM 設計與規格 (Design) 分頁 - SDD 驅動

## 1. 意圖與目標 (Intent & Goals)

在 AITERM 中新增一個專屬的「設計/規格 (Design)」分頁，提供使用者一個與 AI 進行 **Spec-Driven Development (SDD, 規格驅動開發)** 討論的沉浸式環境。

目標是讓使用者能將模糊的需求想法，透過系統化的 AI 引導，逐步轉化為：
1.  **嚴謹且無歧義的規格書 (Specification)**
2.  **可追溯的系統設計文件 (System Design)**
3.  **可執行的實作任務清單 (Task Plan)**

所有產出文件**必須支援繁體中文**，且最終能儲存到專案的指定路徑中，為後續的 AI 自動化實作 (如 `writing-plans`, `executing-plans`) 提供高品質的前置作業包。

## 2. 核心功能與互動模式 (Core Features & Interaction)

### 2.1 雙模式切換 (Dual Interaction Modes)
使用者可以在介面頂部自由切換兩種互動模式，以適應不同的工作風格與專案大小：
*   **對話驅動 (Chat-Centric)**：左側為自由對話視窗，右側為即時更新的 Markdown 預覽面板。適合快速迭代與熟悉流程的進階使用者。
*   **精靈引導 (Wizard-Driven)**：明確的階段性進度條 (探索 -> 規格核准 -> 系統設計 -> 任務規劃)。AI 主導流程，強制確保前一階段完成並「核准 (Approve)」後，才能進入下一階段。

### 2.2 SDD 嚴格階段控管 (Strict SDD Phasing)
落實 SDD 精神，將流程嚴格劃分為三個階段，且產出物具備強關聯性：
1.  **規格定義 (Spec Definition)**：
    *   **AI 角色**：產品經理 (Product Manager)。
    *   **任務**：釐清意圖、邊界、驗收標準。
    *   **產出**：Markdown 規格書 (`XXX-design.md`)。
    *   **限制**：必須由使用者明確點擊「核准並凍結 (Approve & Freeze)」後，才能進行後續設計。
2.  **系統設計 (System Design)**：
    *   **AI 角色**：軟體架構師 (Software Architect)。
    *   **任務**：基於已凍結的規格，進行架構、API、資料庫設計。
    *   **產出**：架構文件 (`XXX-architecture.md`)。
    *   **限制**：設計中的每一個決策，都必須能追溯到規格書中的要求。不允許超前部署 (Speculative Features)。
3.  **任務規劃 (Task Planning)**：
    *   **AI 角色**：技術主管 (Tech Lead)。
    *   **任務**：將架構拆解為可執行的 Tasks。
    *   **產出**：實作計畫 (`XXX-plan.md` 或 Checklists)。
    *   **限制**：每個 Task 必須附帶驗證條件。

### 2.3 右側預覽面板 (Preview Panel)
*   提供多個頁籤 (Tabs)：`規格 (Spec)`, `架構 (Architecture)`, `計畫 (Plan)`。
*   顯示當前文件的狀態 (如：草稿中、審閱中、已核准)。
*   AI 透過專屬的 Tool Calls (`update_spec_draft`, `update_sdd_draft`, `update_plan_draft`) 在背景持續更新這些草稿。

## 3. 技術架構與資料流 (Architecture & Data Flow)

### 3.1 狀態持久化與 Context 壓縮 (Session & Context Management)
為解決地端模型 (Local Models) Context Window 較小的問題，並支援跨工作階段接續討論，採用 **後端 SQLite 持久化 + 智能摘要機制**：
*   **資料表**：
    *   `design_sessions`: 儲存對話主題、當前草稿 (Spec/SDD/Plan)、`context_summary` (壓縮後的脈絡摘要)。
    *   `design_messages`: 儲存完整的對話歷史。
*   **壓縮機制 (Compression)**：當對話 Token 接近模型上限的 80% 時，在背景觸發 AI 摘要任務，將歷史對話壓縮成 500 字以內的 `context_summary`，並以此取代舊的對話歷史餵給 AI，確保不遺失早期決策脈絡。

### 3.2 專案脈絡感知 (Context-Awareness)
*   **檔案樹讀取**：對話開始前或使用者主動載入時，前端呼叫 Tauri Command 獲取當前工作目錄的檔案結構，讓 AI 了解專案現況。
*   **網址解析**：若使用者提供網址，系統提示是否允許抓取。允許則透過 Tauri 後端抓取網頁內容，壓縮後加入 Context 供 AI 參考。

### 3.3 檔案儲存機制 (File Storage)
採用 **混合模式 (Hybrid)**：
*   在流程最後，點擊「完成並存檔 (Finalize & Save)」時，系統預設提供推薦路徑 (如 `docs/superpowers/specs/`)。
*   使用者可點擊修改路徑與檔名，確認後由 Tauri Command 將右側草稿寫入實體檔案。

## 4. 實作考量與邊界 (Implementation Details)

*   **多語系支援**：所有 System Prompts 必須強制要求 AI **以繁體中文 (zh-TW)** 輸出對話與規格文件內容。
*   **Tauri IPC 擴充**：需要新增處理 Session 管理 (`design_start_session`, `design_save_spec` 等) 與網址抓取 (`fetch_url_content`) 的 Rust Commands。
*   **UI 組件**：需開發新的 `DesignTab.tsx`, `DesignChat.tsx`, `SpecPreview.tsx` 等組件，並復用現有的 `Markdown` 渲染組件。

---
*Generated via Brainstorming Skill.*