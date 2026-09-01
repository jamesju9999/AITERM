import { useMemo } from "react";
import {
  ResponsiveContainer,
  BarChart,
  Bar,
  LineChart,
  Line,
  PieChart,
  Pie,
  Cell,
  CartesianGrid,
  XAxis,
  YAxis,
  Tooltip,
  Legend,
} from "recharts";
import { CHART_PALETTE_LIGHT, CHART_PALETTE_DARK } from "../../lib/chartPalette";

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

export function ArtifactChart({ spec }: ArtifactChartProps) {
  const palette = useMemo(() => (isDarkSurface() ? CHART_PALETTE_DARK : CHART_PALETTE_LIGHT), []);

  if (spec.type === "pie") {
    const seriesKey = spec.series[0]?.key ?? "value";
    return (
      <ResponsiveContainer width="100%" height={320}>
        <PieChart>
          <Pie data={spec.data} dataKey={seriesKey} nameKey={spec.xKey} label>
            {spec.data.map((_, i) => (
              <Cell key={i} fill={palette.categorical[i % palette.categorical.length]} />
            ))}
          </Pie>
          <Tooltip />
          {spec.series.length >= 2 && <Legend />}
        </PieChart>
      </ResponsiveContainer>
    );
  }

  const commonAxes = (
    <>
      <CartesianGrid stroke={palette.gridline} strokeDasharray="3 3" />
      <XAxis dataKey={spec.xKey} stroke={palette.muted} />
      <YAxis stroke={palette.muted} />
      <Tooltip
        contentStyle={{
          background: palette.surface,
          border: `1px solid ${palette.baseline}`,
          color: palette.textPrimary,
        }}
      />
      {spec.series.length >= 2 && <Legend />}
    </>
  );

  if (spec.type === "line") {
    return (
      <ResponsiveContainer width="100%" height={320}>
        <LineChart data={spec.data}>
          {commonAxes}
          {spec.series.map((s, i) => (
            <Line
              key={s.key}
              dataKey={s.key}
              name={s.label}
              stroke={palette.categorical[i % palette.categorical.length]}
            />
          ))}
        </LineChart>
      </ResponsiveContainer>
    );
  }

  return (
    <ResponsiveContainer width="100%" height={320}>
      <BarChart data={spec.data}>
        {commonAxes}
        {spec.series.map((s, i) => (
          <Bar
            key={s.key}
            dataKey={s.key}
            name={s.label}
            fill={palette.categorical[i % palette.categorical.length]}
          />
        ))}
      </BarChart>
    </ResponsiveContainer>
  );
}
