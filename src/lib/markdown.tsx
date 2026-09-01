import type { ReactNode } from "react";
import ReactMarkdown from "react-markdown";
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

// ── Block markdown renderer ────────────────────────────────────────────────

export function MarkdownText({ text, streaming }: { text: string; streaming?: boolean }): ReactNode {
  return (
    <div className="markdown-body">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex]}
        components={{
          code({ node, className, children, ...props }) {
            const match = /language-([\w-]+)/.exec(className || "");

            if (match && match[1].toLowerCase() === "mermaid") {
              return <MermaidBlock chart={String(children)} />;
            }
            if (!streaming && match && match[1].toLowerCase() === "artifact-html") {
              return <ArtifactBlockCard kind="html" content={String(children)} />;
            }
            if (!streaming && match && match[1].toLowerCase() === "artifact-chart") {
              return <ArtifactBlockCard kind="chart" content={String(children)} />;
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
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}