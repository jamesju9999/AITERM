import type { MermaidConfig } from "mermaid";

/**
 * Mermaid 全域設定 —— 深色 editorial 皮，色系取自 cathrynlavery/diagram-design
 * 的 dark 變體（slate paper、white-smoke ink、atomic-tangerine accent）。
 * 見專案根目錄 `NOTICE.md`。
 *
 * 目的：即使模型退回用 Mermaid（弱模型很常這樣），畫出來的圖也長得像
 * diagram-design，而不是 Mermaid 預設的淡紫風。
 *
 * 設計取向（避免 wireframe 感）：
 *  - 邊框是耳語級低對比（ink @ 12%），靠 node/底色的微差分層，不用純白硬線。
 *  - 箭頭壓暗到 `soft`，讓視線先落在節點而不是連線。
 *  - 字重分層：節點名 500、edge label 400 且用 muted 色 + 縮小，subgraph 標題
 *    當 eyebrow（muted、加字距）。
 *  - 不碰連接線路由：dagre 算的是曲線控制點，硬改 `step` 會讓線互疊、箭頭歪，
 *    維持預設 `basis`。
 *
 * `MermaidBlock.tsx`（渲染）與 `mermaidSanitize.test.ts`（測試用 parse）共用這份，
 * 避免兩處各自漂移。
 */

// diagram-design dark tokens
const PAPER = "#262a37"; // page / diagram background
const NODE = "#313746"; // node fill — one step up from paper
const NODE_2 = "#3b4252"; // secondary fill (alt nodes, notes, activation)
const BORDER = "rgba(240,242,246,0.13)"; // whisper hairline
const INK = "#eceef2"; // primary text
const MUTED = "#9aa3b2"; // secondary text, edge labels, subgraph titles
const ARROW = "#7b8394"; // edge / signal stroke — dimmer than the text
const ACCENT = "#f0895a"; // atomic-tangerine (dark variant) — focal only

export const MERMAID_INIT: MermaidConfig = {
  startOnLoad: false,
  securityLevel: "loose", // needed for themeCSS + some interactive styling
  theme: "base",
  fontFamily:
    "'Geist', 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
  flowchart: { htmlLabels: true, padding: 14, nodeSpacing: 44, rankSpacing: 52 },
  themeVariables: {
    darkMode: true,
    background: PAPER,
    mainBkg: NODE,
    primaryColor: NODE,
    primaryBorderColor: BORDER,
    primaryTextColor: INK,
    secondaryColor: NODE_2,
    secondaryBorderColor: BORDER,
    secondaryTextColor: INK,
    tertiaryColor: PAPER,
    tertiaryBorderColor: BORDER,
    tertiaryTextColor: INK,
    lineColor: ARROW,
    textColor: INK,
    titleColor: INK,
    nodeBorder: BORDER,
    nodeTextColor: INK,
    edgeLabelBackground: PAPER, // mask behind edge labels = page colour, no chip
    // subgraph containers — barely a lift, faint frame, muted title
    clusterBkg: "#2b303d",
    clusterBorder: "rgba(240,242,246,0.09)",
    // focal accent (Mermaid uses these for the note/highlight roles)
    accent: ACCENT,
    // sequence
    actorBkg: NODE,
    actorBorder: BORDER,
    actorTextColor: INK,
    actorLineColor: ARROW,
    signalColor: ARROW,
    signalTextColor: INK,
    labelBoxBkgColor: NODE_2,
    labelBoxBorderColor: BORDER,
    labelTextColor: INK,
    loopTextColor: MUTED,
    noteBkgColor: NODE_2,
    noteBorderColor: BORDER,
    noteTextColor: INK,
    activationBkgColor: NODE_2,
    activationBorderColor: ARROW,
    // state / class / er reuse primary + line vars above
  },
  themeCSS: `
    .node rect, .node circle, .node polygon, .node path, .node .label-container {
      filter: none;
    }
    .nodeLabel, .nodeLabel p { font-weight: 500; letter-spacing: 0.005em; }
    .edgeLabel, .edgeLabel p {
      background: ${PAPER};
      color: ${MUTED};
      font-size: 12.5px;
      font-weight: 400;
    }
    .edgeLabel rect { fill: ${PAPER}; }
    .cluster-label, .cluster-label p {
      color: ${MUTED};
      font-weight: 600;
      font-size: 12px;
      letter-spacing: 0.06em;
      text-transform: uppercase;
    }
    .edgePath .arrowMarkerPath, marker path { fill: ${ARROW}; stroke: none; }

    /* Functional colour — everything else stays monochrome.
       Decision points get a cooler, deeper wash so branches read at a glance. */
    .node polygon {
      fill: #2b3547 !important;
      stroke: rgba(240,242,246,0.22) !important;
    }
    /* Circular start / end markers — the one spot the accent earns its keep.
       (Only (( )) circle nodes; ([ ]) stadium renders like a rounded rect and
       stays monochrome.) */
    .node circle {
      fill: rgba(240,137,90,0.13) !important;
      stroke: rgba(240,137,90,0.55) !important;
    }
  `,
};
