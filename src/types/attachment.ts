import { nanoid } from "nanoid";
import type { ContentPart } from "../ipc/ai";

export type AttachmentKind = "image" | "text" | "binary";

export interface Attachment {
  id: string;
  kind: AttachmentKind;
  name: string;
  mimeType: string;
  /** image → base64 data URI; text → raw content; binary → absolute path */
  data: string;
  /** Same as data for images — used as <img src>. */
  previewUrl?: string;
}

/** Classify a File by MIME type and extension. */
export function classifyFile(file: File): AttachmentKind {
  if (file.type.startsWith("image/")) return "image";
  const textExtensions = /\.(txt|md|json|csv|ts|tsx|js|jsx|py|rs|go|java|c|cpp|h|css|html|xml|yaml|yml|toml|sh|bash|zsh)$/i;
  if (file.type.startsWith("text/") || textExtensions.test(file.name)) return "text";
  return "binary";
}

/** Read a file into an Attachment. Returns a Promise. */
export async function readFileAsAttachment(file: File): Promise<Attachment> {
  const kind = classifyFile(file);
  const id = nanoid();
  // Tauri 2 injects .path on File objects in the webview
  const filePath = (file as File & { path?: string }).path ?? file.name;

  if (kind === "image") {
    const data = await readAsDataUrl(file);
    return { id, kind, name: file.name, mimeType: file.type, data, previewUrl: data };
  }
  if (kind === "text") {
    const data = await readAsText(file);
    return { id, kind, name: file.name, mimeType: file.type || "text/plain", data };
  }
  // binary: just store the path
  return { id, kind, name: file.name, mimeType: file.type || "application/octet-stream", data: filePath };
}

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

function readAsText(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsText(file);
  });
}

/** Build a ContentPart[] from user text + attachments. */
export function buildContentParts(text: string, attachments: Attachment[]): ContentPart[] {
  const parts: ContentPart[] = [];
  if (text.trim()) {
    parts.push({ type: "text", text });
  }
  for (const att of attachments) {
    if (att.kind === "image") {
      parts.push({ type: "image_url", image_url: { url: att.data } });
    } else if (att.kind === "text") {
      parts.push({ type: "text", text: `[${att.name}]\n${att.data}` });
    } else {
      parts.push({ type: "text", text: att.data });
    }
  }
  return parts;
}

/** Extract displayable text from a content value (for session title, etc.). */
export function contentToDisplayString(content: string | ContentPart[]): string {
  if (typeof content === "string") return content;
  return content
    .filter((p): p is Extract<ContentPart, { type: "text" }> => p.type === "text")
    .map((p) => p.text)
    .join(" ");
}
