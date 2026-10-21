/** 工具類別圖示：同一套 14px 線條圖示，取代各系統大小不一的 emoji。 */
import type { ReactElement } from "react";

const PATHS: Record<string, ReactElement> = {
  file: (<><path d="M4 2h5l3 3v9H4z" /><path d="M9 2v3h3" /></>),
  shell: (<><path d="M2.5 4l4 4-4 4" /><path d="M8 12h5.5" /></>),
  web: (<><circle cx="8" cy="8" r="6" /><path d="M2 8h12M8 2c2 2 2 10 0 12M8 2c-2 2-2 10 0 12" /></>),
  database: (<><ellipse cx="8" cy="4" rx="5" ry="2" /><path d="M3 4v8c0 1.1 2.2 2 5 2s5-.9 5-2V4M3 8c0 1.1 2.2 2 5 2s5-.9 5-2" /></>),
  mcp: (<><path d="M6 2v3M10 2v3M4 5h8v3a4 4 0 0 1-8 0zM8 12v2" /></>),
  other: (<><circle cx="8" cy="8" r="2" /><path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4" /></>),
};

export function CategoryIcon({ category }: { category: string }) {
  return (
    <svg className="aiterm-cact__icon" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {PATHS[category] ?? PATHS.other}
    </svg>
  );
}
