/**
 * 色票數值取自 `dataviz` skill 的 references/palette.md，但**順序改過**：領頭色
 * 換成 aqua，跟 AITerm 自己的品牌綠一致。
 *
 * 順序不是美感問題——它就是色盲安全機制本身（相鄰色對必須拉得開）。這個排列是
 * 實際跑 skill 的 scripts/validate_palette.js 驗出來的，深淺兩種模式的五項檢查
 * 全數 PASS。第一個候選（把 aqua 搬到最前面、其餘不動）在兩種模式都 FAIL，紅與
 * 洋紅相鄰色差不足——所以動順序之前一定要重跑驗證器，不可以憑眼睛判斷。
 *
 * hex 值本身不要手動改。
 */

export interface ThemeColors {
  categorical: string[];
  sequential: string[];
  status: { good: string; warning: string; serious: string; critical: string };
  surface: string;
  textPrimary: string;
  textSecondary: string;
  muted: string;
  gridline: string;
  baseline: string;
}

export const CHART_PALETTE_LIGHT: ThemeColors = {
  categorical: [
    "#1baf7a", // 1 aqua — 對齊 AITerm 的品牌綠
    "#2a78d6", // 2 blue
    "#eb6834", // 3 orange
    "#4a3aa7", // 4 violet
    "#eda100", // 5 yellow
    "#e87ba4", // 6 magenta
    "#008300", // 7 green
    "#e34948", // 8 red
  ],
  sequential: ["#cde2fb", "#9ec5f4", "#6da7ec", "#3987e5", "#256abf", "#184f95", "#0d366b"],
  status: { good: "#0ca30c", warning: "#fab219", serious: "#ec835a", critical: "#d03b3b" },
  surface: "#fcfcfb",
  textPrimary: "#0b0b0b",
  textSecondary: "#52514e",
  muted: "#898781",
  gridline: "#e1e0d9",
  baseline: "#c3c2b7",
};

export const CHART_PALETTE_DARK: ThemeColors = {
  categorical: [
    "#199e70", // 1 aqua — 對齊 AITerm 的品牌綠
    "#3987e5", // 2 blue
    "#d95926", // 3 orange
    "#9085e9", // 4 violet
    "#c98500", // 5 yellow
    "#d55181", // 6 magenta
    "#008300", // 7 green
    "#e66767", // 8 red
  ],
  sequential: ["#cde2fb", "#9ec5f4", "#6da7ec", "#3987e5", "#256abf", "#184f95", "#0d366b"],
  status: { good: "#0ca30c", warning: "#fab219", serious: "#ec835a", critical: "#d03b3b" },
  surface: "#1a1a19",
  textPrimary: "#ffffff",
  textSecondary: "#c3c2b7",
  muted: "#898781",
  gridline: "#2c2c2a",
  baseline: "#383835",
};
