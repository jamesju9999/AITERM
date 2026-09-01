import { useMemo } from "react";
import {
  ResponsiveContainer,
  BarChart,
  Bar,
  ComposedChart,
  Line,
  Area,
  PieChart,
  Pie,
  Cell,
  CartesianGrid,
  XAxis,
  YAxis,
  Tooltip,
  Legend,
} from "recharts";
import { CHART_PALETTE_LIGHT, CHART_PALETTE_DARK, type ThemeColors } from "../../lib/chartPalette";
import "./ArtifactChart.css";

export interface ChartSeriesSpec {
  key: string;
  label: string;
}

export interface ChartSpec {
  type: "bar" | "line" | "pie";
  title?: string;
  data: Record<string, unknown>[];
  xKey: string;
  series: ChartSeriesSpec[];
}

interface ArtifactChartProps {
  spec: ChartSpec;
}

/** 沿用 AgentStatusBar.css 既有的多主題分桶慣例：只有明確的 "light" 主題算
 *  淺色，其他（dark/nord/dracula/未設定）一律當深色處理。 */
function isDarkSurface(): boolean {
  return document.documentElement.getAttribute("data-theme") !== "light";
}

/** 單一序列且長條不多時，數值直接標在頂端。規範同時要求「不可以每個點都標」
 *  ——多序列或資料點一多就會變成噪音，那時交給座標軸、tooltip 與表格檢視。 */
const MAX_LABELLED_BARS = 12;

/** 千分位。刻度承載了沒有被直接標註的數值，讀得順比省空間重要。 */
const formatTick = (v: unknown) =>
  typeof v === "number" ? v.toLocaleString() : String(v ?? "");

/** 座標軸要 recessive：不畫軸線、不畫刻度線，只留文字。 */
function axisProps(palette: ThemeColors) {
  return {
    stroke: palette.muted,
    axisLine: false as const,
    tickLine: false as const,
    tick: { fill: palette.muted, fontSize: 11 },
  };
}

function tooltipProps(palette: ThemeColors) {
  return {
    contentStyle: {
      background: palette.surface,
      border: `1px solid ${palette.baseline}`,
      borderRadius: 8,
      color: palette.textPrimary,
      fontSize: 12,
      boxShadow: "0 6px 20px rgba(0,0,0,0.28)",
    },
    // 規範：tooltip 裡「數值是主角、序列名是配角」——讀者已經知道是哪個序列，
    // 他要的是數字。這跟圖例的階層剛好相反。
    labelStyle: { color: palette.textSecondary, marginBottom: 4, fontSize: 11 },
    itemStyle: { color: palette.textPrimary, fontWeight: 600 },
    // 預設的 hover 遮罩太重，會蓋過資料本身。
    cursor: { fill: palette.gridline, fillOpacity: 0.35 },
  };
}

export function ArtifactChart({ spec }: ArtifactChartProps) {
  const palette = useMemo(() => (isDarkSurface() ? CHART_PALETTE_DARK : CHART_PALETTE_LIGHT), []);
  const color = (i: number) => palette.categorical[i % palette.categorical.length];
  const labelBars = spec.series.length === 1 && spec.data.length <= MAX_LABELLED_BARS;

  if (spec.type === "pie") {
    const seriesKey = spec.series[0]?.key ?? "value";
    return (
      <div className="aiterm-artifact-chart">
        <ResponsiveContainer width="100%" height="100%">
          <PieChart>
            <Pie
              data={spec.data}
              dataKey={seriesKey}
              nameKey={spec.xKey}
              // 半徑留餘裕，否則外側的標籤會被容器裁掉（實機上就掉了一個，
              // 只剩一條指向空白的指引線）。
              outerRadius="65%"
              label={{ fill: palette.textSecondary, fontSize: 11 }}
              // 扇形之間用 surface 色的 2px 縫隔開，而不是描邊——白留空是
              // 分隔的機制，描邊等於加上不是資料的墨水。
              stroke={palette.surface}
              strokeWidth={2}
            >
              {spec.data.map((_, i) => (
                <Cell key={i} fill={color(i)} />
              ))}
            </Pie>
            <Tooltip {...tooltipProps(palette)} />
            {/* 圓餅圖一定要有圖例：扇形就是類別，顏色沒有對照表等於沒有意義。
                這跟長條/折線「單一序列不放圖例」的規則不同——那裡的身分是序列，
                標題已經講完了。 */}
            <Legend wrapperStyle={{ fontSize: 12, color: palette.textSecondary }} />
          </PieChart>
        </ResponsiveContainer>
      </div>
    );
  }

  const commonAxes = (
    <>
      {/* hairline 實線、只留水平線：直線對長條圖只是噪音。 */}
      <CartesianGrid stroke={palette.gridline} vertical={false} />
      <XAxis dataKey={spec.xKey} {...axisProps(palette)} />
      <YAxis {...axisProps(palette)} tickFormatter={formatTick} width={56} />
      <Tooltip {...tooltipProps(palette)} />
      {spec.series.length >= 2 && (
        <Legend wrapperStyle={{ fontSize: 12, color: palette.textSecondary }} />
      )}
    </>
  );

  if (spec.type === "line") {
    return (
      <div className="aiterm-artifact-chart">
        <ResponsiveContainer width="100%" height="100%">
          <ComposedChart data={spec.data} margin={{ top: 12, right: 16, bottom: 4, left: 0 }}>
            {commonAxes}
            {/* 線下方鋪一層 ~10% 的同色淡影。規範允許的就是這種「wash」，不是
                飽和色塊；這也是唯一不扭曲數值的加層次方式——立體陰影會讓讀者
                不確定該讀資料端的前緣還是後緣。 */}
            {spec.series.map((s, i) => (
              <Area
                key={`area-${s.key}`}
                dataKey={s.key}
                stroke="none"
                fill={color(i)}
                fillOpacity={0.1}
                // 圖例與 tooltip 交給下面的 Line，這層只是底色。
                legendType="none"
                tooltipType="none"
                isAnimationActive={false}
              />
            ))}
            {spec.series.map((s, i) => (
              <Line
                key={s.key}
                dataKey={s.key}
                name={s.label}
                stroke={color(i)}
                strokeWidth={2}
                strokeLinecap="round"
                strokeLinejoin="round"
                // 標記至少 8px（r>=4），外面帶一圈 surface 色的環，交疊時才讀得出來。
                dot={{ r: 4, fill: color(i), stroke: palette.surface, strokeWidth: 2 }}
                activeDot={{ r: 5, fill: color(i), stroke: palette.surface, strokeWidth: 2 }}
              />
            ))}
          </ComposedChart>
        </ResponsiveContainer>
      </div>
    );
  }

  return (
    <div className="aiterm-artifact-chart">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart
          data={spec.data}
          margin={{ top: 12, right: 16, bottom: 4, left: 0 }}
          barGap={2}
        >
          {commonAxes}
          {spec.series.map((s, i) => (
            <Bar
              key={s.key}
              dataKey={s.key}
              name={s.label}
              fill={color(i)}
              // 不要填滿整個欄位——留白是版面的一部分；資料端 4px 圓角、
              // 貼著基線那端維持方角。
              maxBarSize={24}
              radius={[4, 4, 0, 0]}
              // 規範：滑過的那根要有反應，讀者才知道自己指到了什麼。
              activeBar={{ fill: color(i), fillOpacity: 0.82 }}
              // 標籤穿的是文字色，不是資料色——淺色的分類色當文字會看不清楚。
              label={labelBars
                ? { position: "top", formatter: formatTick, fill: palette.textSecondary, fontSize: 11 }
                : undefined}
            />
          ))}
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}
