interface ArtifactHtmlFrameProps {
  html: string;
  title: string;
}

/**
 * 用 sandbox iframe 隔離渲染 AI 生成的任意 HTML。允許跑 JS（allow-scripts），
 * 但絕對不給 allow-same-origin——這樣瀏覽器會把這個 iframe 當成獨立的不透明
 * 來源，裡面的 JS 完全碰不到主視窗的 DOM/localStorage，更不可能碰到 Tauri
 * 的 IPC bridge。這個組合是刻意的，不是遺漏，見
 * docs/superpowers/specs/2026-09-01-artifact-panel-design.md 的「安全性」一節。
 */
export function ArtifactHtmlFrame({ html, title }: ArtifactHtmlFrameProps) {
  return (
    <iframe
      className="aiterm-artifact-html-frame"
      title={title}
      srcDoc={html}
      sandbox="allow-scripts"
    />
  );
}
