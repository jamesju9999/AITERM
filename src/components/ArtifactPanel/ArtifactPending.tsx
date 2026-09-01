import type { ArtifactKind } from "../../contexts/ArtifactPanelContext";
import { FileTextIcon, ChartIcon } from "../Icons";
import "./ArtifactPending.css";

interface ArtifactPendingProps {
  kind: ArtifactKind;
}

/**
 * 串流中、artifact 區塊還沒寫完時，聊天泡泡裡先擺這張卡。
 *
 * 沒有它的話，一份 HTML 報告動輒好幾千個 token，期間畫面上只有模型那句「我已完成
 * 整理」然後長時間沒動靜——使用者會以為當掉了（實測回報過）。原本這段時間會把
 * 半成品的原始 HTML 直接倒進泡泡裡，那同樣沒有比較好懂。
 */
export function ArtifactPending({ kind }: ArtifactPendingProps) {
  return (
    <div className="aiterm-artifact-pending" aria-busy="true">
      {kind === "html" ? <FileTextIcon size={14} /> : <ChartIcon size={14} />}
      <span>{kind === "html" ? "文件產生中" : "圖表產生中"}</span>
      <span className="aiterm-artifact-pending__dots" aria-hidden="true">
        <i />
        <i />
        <i />
      </span>
    </div>
  );
}
