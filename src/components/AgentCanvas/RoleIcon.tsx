import type { ReactNode } from "react";

import type { AgentKind } from "../../ipc/agents";
import { KindIcon } from "./KindIcon";

/** 角色圖示的名稱。跟 KindIcon 同一套線條風格（24 格、線寬 2、顏色跟著文字）。 */
export type RoleIconName =
  | "spec"
  | "migrate"
  | "test"
  | "bug"
  | "palette"
  | "wrench"
  | "gear"
  | "doc"
  | "chart"
  | "landmark"
  | "check"
  | "layers"
  | "shield"
  | "star";

const PATHS: Record<RoleIconName, ReactNode> = {
  spec: (
    <>
      <path d="M4 20V4l16 16Z" />
      <path d="M8 16v-4l4 4Z" />
    </>
  ),
  migrate: (
    <>
      <path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
      <path d="M21 3v5h-5" />
      <path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
      <path d="M3 21v-5h5" />
    </>
  ),
  test: (
    <>
      <path d="M10 2v7.5L4.5 19a2 2 0 0 0 1.7 3h11.6a2 2 0 0 0 1.7-3L14 9.5V2" />
      <path d="M8.5 2h7" />
      <path d="M7 16h10" />
    </>
  ),
  bug: (
    <>
      <rect x="8" y="6" width="8" height="14" rx="4" />
      <path d="M9.5 3 11 6" />
      <path d="M14.5 3 13 6" />
      <path d="M4 10h4M16 10h4M4 15h4M16 15h4" />
      <path d="M12 11v9" />
    </>
  ),
  palette: (
    <>
      <path d="M12 22a10 10 0 1 1 10-10c0 2.2-1.8 3-3.5 3H16a2 2 0 0 0-1.5 3.3A2 2 0 0 1 12 22Z" />
      <circle cx="7.5" cy="11" r="1" fill="currentColor" />
      <circle cx="10" cy="7" r="1" fill="currentColor" />
      <circle cx="15" cy="7" r="1" fill="currentColor" />
    </>
  ),
  wrench: (
    <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94Z" />
  ),
  gear: (
    <>
      <circle cx="12" cy="12" r="3" />
      <circle cx="12" cy="12" r="7" />
      <path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9 7 7M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1" />
    </>
  ),
  doc: (
    <>
      <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8Z" />
      <path d="M14 3v5h5" />
      <path d="M9 13h6M9 17h6" />
    </>
  ),
  chart: (
    <>
      <path d="M3 3v18h18" />
      <path d="M8 17v-5M13 17V8M18 17v-3" />
    </>
  ),
  landmark: (
    <>
      <path d="M3 21h18" />
      <path d="M5 21V11M9.5 21V11M14.5 21V11M19 21V11" />
      <path d="M2 10 12 3l10 7Z" />
    </>
  ),
  check: (
    <>
      <circle cx="12" cy="12" r="9" />
      <polyline points="8 12 11 15 16 9" />
    </>
  ),
  layers: (
    <>
      <path d="M12 3 2 8l10 5 10-5Z" />
      <path d="m2 13 10 5 10-5" />
      <path d="m2 17.5 10 5 10-5" />
    </>
  ),
  shield: (
    <>
      <path d="M12 3 4 6v6c0 5 3.5 8 8 9 4.5-1 8-4 8-9V6Z" />
      <polyline points="9 12 11 14 15 10" />
    </>
  ),
  star: <path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2L12 17.3 6.4 20.2l1.1-6.2L3 9.6l6.2-.9Z" />,
};

/** 角色圖示：名稱是種類時直接用 KindIcon（同一個圖不畫兩份）。 */
export function RoleIcon({ name, size = 15 }: { name: RoleIconName | AgentKind; size?: number }) {
  if (name === "coder" || name === "reviewer" || name === "orchestrator" || name === "verifier") {
    return <KindIcon kind={name} size={size} />;
  }
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      data-role-icon={name}
    >
      {PATHS[name]}
    </svg>
  );
}
