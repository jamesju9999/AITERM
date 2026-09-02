import type { ChartSpec } from "./ArtifactChart";
import "./ArtifactChartTable.css";

/**
 * 一張圖的表格孿生。
 *
 * 規範說得很直接：tooltip 只能「加強」不能「把關」——每個數值都必須有不用 hover
 * 也拿得到的途徑。圖上只標了選擇性的幾個值，其餘就靠這裡；順帶讓讀螢幕的人、
 * 以及想複製數字的人也拿得到。
 */
export function ArtifactChartTable({ specs }: { specs: ChartSpec[] }) {
  return (
    <div className="aiterm-artifact-table">
      {specs.map((spec, i) => (
        <div key={i}>
          {spec.title && <div className="aiterm-artifact-table__title">{spec.title}</div>}
          <table>
            <thead>
              <tr>
                <th>{spec.xKey}</th>
                {spec.series.map((s) => (
                  <th key={s.key} className="aiterm-artifact-table__num">{s.label}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {spec.data.map((row, r) => (
                <tr key={r}>
                  <td>{String(row[spec.xKey] ?? "")}</td>
                  {spec.series.map((s) => {
                    const v = row[s.key];
                    return (
                      <td key={s.key} className="aiterm-artifact-table__num">
                        {typeof v === "number" ? v.toLocaleString() : String(v ?? "")}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ))}
    </div>
  );
}
