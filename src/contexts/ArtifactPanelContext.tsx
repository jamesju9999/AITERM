import { createContext, useCallback, useContext, useState, type ReactNode } from "react";

export type ArtifactKind = "html" | "chart";

export interface Artifact {
  id: string;
  kind: ArtifactKind;
  title: string;
  /** kind="html" 時是完整 HTML 字串；kind="chart" 時是未解析的 JSON 字串
   *（由渲染面板自己 parse，見 ArtifactPanel.tsx）。 */
  content: string;
}

interface ArtifactPanelState {
  activeArtifact: Artifact | null;
  showArtifact: (artifact: Artifact) => void;
  clearArtifact: () => void;
}

const ArtifactPanelContext = createContext<ArtifactPanelState | null>(null);

export function ArtifactPanelProvider({ children }: { children: ReactNode }) {
  const [activeArtifact, setActiveArtifact] = useState<Artifact | null>(null);

  const showArtifact = useCallback((artifact: Artifact) => {
    setActiveArtifact(artifact);
  }, []);

  const clearArtifact = useCallback(() => {
    setActiveArtifact(null);
  }, []);

  return (
    <ArtifactPanelContext.Provider value={{ activeArtifact, showArtifact, clearArtifact }}>
      {children}
    </ArtifactPanelContext.Provider>
  );
}

/** 給「沒有 provider 也要能正常運作」的呼叫端用：回傳 null 而不是拋例外。
 *  MarkdownText 用它來判斷這棵樹底下能不能顯示 artifact——DesignView 與
 *  DatabaseAiChat 這個里程碑還沒掛 provider，那裡的 artifact fence 應該退回
 *  普通程式碼區塊，而不是讓整個畫面崩掉。 */
export function useOptionalArtifactPanel(): ArtifactPanelState | null {
  return useContext(ArtifactPanelContext);
}

export function useArtifactPanel(): ArtifactPanelState {
  const ctx = useContext(ArtifactPanelContext);
  if (!ctx) {
    throw new Error("useArtifactPanel must be used within an ArtifactPanelProvider");
  }
  return ctx;
}
