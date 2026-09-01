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

export function useArtifactPanel(): ArtifactPanelState {
  const ctx = useContext(ArtifactPanelContext);
  if (!ctx) {
    throw new Error("useArtifactPanel must be used within an ArtifactPanelProvider");
  }
  return ctx;
}
