import { useMemo, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
// Import katex css for math equations
import "katex/dist/katex.min.css";

/**
 * If `raw` looks like a JSON object with a common response field, extract it.
 * Handles:
 *   - Direct JSON: { "response": "..." }
 *   - Fenced JSON: ```json\n{ "response": "..." }\n```
 * Also handles partial/streaming JSON gracefully.
 */
export function extractResponseText(raw: string): string {
  let trimmed = raw.trim();

  // Strip markdown code fences: ```json\n...\n``` or ```\n...\n```
  if (trimmed.startsWith("```")) {
    const newlinePos = trimmed.indexOf("\n");
    if (newlinePos !== -1) {
      const inner = trimmed.slice(newlinePos + 1);
      const closePos = inner.lastIndexOf("```");
      trimmed = closePos !== -1 ? inner.slice(0, closePos).trim() : inner.trim();
    }
  }

  if (!trimmed.startsWith("{")) return raw;

  // Full parse first
  try {
    const parsed = JSON.parse(trimmed);
    if (parsed && typeof parsed === "object") {
      // Generic single-string response fields
      for (const key of ["response", "text", "content", "answer", "output", "thought"]) {
        if (typeof parsed[key] === "string") return parsed[key];
      }

      // { "message": "...", "commands": [...] } — Gemma / some local models
      if (typeof parsed["message"] === "string") {
        const parts: string[] = [parsed["message"]];
        if (Array.isArray(parsed["commands"])) {
          for (const s of parsed["commands"]) {
            if (typeof s === "string" && s.trim()) {
              parts.push(`<cmd>${s.trim()}</cmd>`);
            }
          }
        }
        return parts.join("\n\n");
      }

      // AI-command / chat JSON with "explanation" + optional "suggestions" or "command".
      // Some models (e.g. Gemma, Llama) output this format even for chat requests.
      if (typeof parsed["explanation"] === "string") {
        const parts: string[] = [parsed["explanation"]];
        // "suggestions": array of command strings
        if (Array.isArray(parsed["suggestions"])) {
          for (const s of parsed["suggestions"]) {
            if (typeof s === "string" && s.trim()) {
              parts.push(`<cmd>${s.trim()}</cmd>`);
            }
          }
        }
        // "command": single command string (the /ai query schema)
        if (typeof parsed["command"] === "string" && parsed["command"].trim() && parsed["command"].trim() !== "DONE") {
          parts.push(`<cmd>${parsed["command"].trim()}</cmd>`);
        }
        return parts.join("\n\n");
      }
    }
  } catch {
    // Partial JSON (mid-stream) — regex fallback for known single-string fields
    const m = trimmed.match(
      /"(?:response|text|content|answer|output|thought)"\s*:\s*"([\s\S]*?)(?:(?<!\\)"\s*[,}]|$)/ 
    );
    if (m) {
      try {
        return JSON.parse(`"${m[1]}"`);
      } catch {
        return m[1];
      }
    }
  }

  return raw;
}

/**
 * Convert literal \n / \t escape sequences to real characters.
 * Models sometimes emit these as plain text rather than actual control chars.
 */
export function unescapeNewlines(text: string): string {
  return text.replace(/\\n/g, "\n").replace(/\\t/g, "\t");
}

import { MermaidBlock } from "../components/MermaidBlock";
import { ArtifactBlockCard } from "../components/ArtifactPanel/ArtifactBlockCard";
import { ArtifactPending } from "../components/ArtifactPanel/ArtifactPending";
import { useOptionalArtifactPanel } from "../contexts/ArtifactPanelContext";

// ── Block markdown renderer ────────────────────────────────────────────────

export function MarkdownText({ text, streaming }: { text: string; streaming?: boolean }): ReactNode {
  // 沒有 ArtifactPanelProvider 的樹（目前是 DesignView 的 SpecPreview，它刻意
  // 不接——自己已經有右側預覽面板）不能渲染 artifact 卡片：那個元件會呼叫
  // useArtifactPanel()，在沒有 provider 時拋例外、把整個畫面帶走。退回普通程式碼
  // 區塊，維持這個功能之前的行為。
  const canShowArtifacts = useOptionalArtifactPanel() !== null;
  // components 一定要 memo 住：react-markdown 把這個物件裡的每個函式當成該
  // 節點的 React element type。如果每次 render 都給一個新的物件字面量（原本
  // 的寫法），任何讓 MarkdownText 重新 render 的外部狀態變化（例如 artifact
  // context 更新）都會讓 react-markdown 以為 code 節點的元件型別變了，導致
  // 該節點被整個卸載再重新掛載——ArtifactBlockCard 的掛載 effect 因此重跑，
  // 又觸發一次同樣的狀態變化，形成無窮迴圈（實測會讓 vitest 卡死，不是理論）。
  const components: Components = useMemo(() => ({
    code({ node, className, children, ...props }) {
      const match = /language-([\w-]+)/.exec(className || "");

      if (match && match[1].toLowerCase() === "mermaid") {
        return <MermaidBlock chart={String(children)} />;
      }
      const artifactKind = match && canShowArtifacts
        ? ({ "artifact-html": "html", "artifact-chart": "chart" } as const)[
            match[1].toLowerCase() as "artifact-html" | "artifact-chart"
          ]
        : undefined;
      if (artifactKind) {
        // 串流中先擺「產生中」的卡：內容還沒收完，登記進面板只會顯示半成品，
        // 而退回原始碼區塊等於把好幾千個 token 的 HTML 倒進聊天泡泡。
        return streaming
          ? <ArtifactPending kind={artifactKind} />
          : <ArtifactBlockCard kind={artifactKind} content={String(children)} />;
      }

      const isInline = !match && !String(children).includes("\n");
      return isInline ? (
        <code className="md-code" {...props}>
          {children}
        </code>
      ) : (
        <pre className="md-pre">
          <code className={className} {...props}>
            {children}
          </code>
        </pre>
      );
    },
    p({ children }) {
      return <p className="md-p">{children}</p>;
    },
    h1({ children }) { return <h1 className="md-h1">{children}</h1>; },
    h2({ children }) { return <h2 className="md-h2">{children}</h2>; },
    h3({ children }) { return <h3 className="md-h3">{children}</h3>; },
    ul({ children }) { return <ul className="md-list">{children}</ul>; },
    ol({ children }) { return <ol className="md-list">{children}</ol>; },
    a({ children, href }) {
      return <a href={href} target="_blank" rel="noopener noreferrer">{children}</a>;
    }
  }), [streaming, canShowArtifacts]);

  return (
    <div className="markdown-body">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex]}
        components={components}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}