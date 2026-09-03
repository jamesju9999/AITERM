import type { MermaidConfig } from "mermaid";

/**
 * Mermaid 全域設定 —— 深色 editorial 皮，色票對齊 cathrynlavery/diagram-design
 * 的 dark 變體（paper `#2d3142` / ink `#f5f5f5` / muted `#bfc0c0` /
 * accent `#f08a59`）。見專案根目錄 `NOTICE.md`。
 *
 * 目的：即使模型退回用 Mermaid（弱模型很常這樣），畫出來的圖也長得像
 * diagram-design，而不是 Mermaid 預設的淡紫風。這層皮只動配色、字體、edge label
 * 遮罩與陰影——不碰連接線路由：Mermaid 的 dagre 算的是曲線控制點，硬改成 `step`
 * 正交會讓線互相疊、箭頭 marker 歪掉，弊大於利，維持預設的 `basis` 曲線。
 *
 * `MermaidBlock.tsx`（渲染）與 `mermaidSanitize.test.ts`（測試用 parse）共用這份，
 * 避免兩處各自漂移。
 */
export const MERMAID_INIT: MermaidConfig = {
  startOnLoad: false,
  securityLevel: "loose", // needed for themeCSS + some interactive styling
  theme: "base",
  fontFamily:
    "'Geist', 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
  flowchart: { htmlLabels: true, padding: 12 },
  themeVariables: {
    darkMode: true,
    background: "#2d3142", // paper
    mainBkg: "#393e53", // paper-2 — node fill
    primaryColor: "#393e53",
    primaryBorderColor: "#f5f5f5", // ink
    primaryTextColor: "#f5f5f5",
    secondaryColor: "#4a4f66",
    tertiaryColor: "#2d3142",
    lineColor: "#bfc0c0", // muted — default arrow stroke
    textColor: "#f5f5f5",
    titleColor: "#f5f5f5",
    nodeBorder: "#f5f5f5",
    clusterBkg: "#2b2f3f", // subgraph container — a touch below paper
    clusterBorder: "rgba(245,245,245,0.20)",
    edgeLabelBackground: "#2d3142", // mask behind edge labels = paper, no mismatched chip
    // sequence
    actorBkg: "#393e53",
    actorBorder: "#f5f5f5",
    actorTextColor: "#f5f5f5",
    signalColor: "#bfc0c0",
    signalTextColor: "#f5f5f5",
    labelBoxBkgColor: "#4a4f66",
    labelBoxBorderColor: "rgba(245,245,245,0.30)",
    labelTextColor: "#f5f5f5",
    loopTextColor: "#f5f5f5",
    noteBkgColor: "#4a4f66",
    noteBorderColor: "rgba(245,245,245,0.30)",
    noteTextColor: "#f5f5f5",
    activationBkgColor: "#4a4f66",
    activationBorderColor: "#bfc0c0",
  },
  themeCSS: `
    .node rect, .node circle, .node polygon, .node path { filter: none; }
    .nodeLabel, .edgeLabel, .cluster-label { font-weight: 600; }
    .edgeLabel .label, .edgeLabel foreignObject div { color: #f5f5f5; }
  `,
};
