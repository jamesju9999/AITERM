import type { AgentKind } from "../../ipc/agents";

/** 四種角色同一套線條圖示（同大小、同線寬），顏色跟著卡片的角色色。 */
export function KindIcon({ kind, size = 15 }: { kind: AgentKind; size?: number }) {
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
      data-kind-icon={kind}
    >
      {kind === "coder" && (
        <>
          <polyline points="16 18 22 12 16 6" />
          <polyline points="8 6 2 12 8 18" />
        </>
      )}
      {kind === "reviewer" && (
        <>
          <circle cx="11" cy="11" r="7" />
          <line x1="21" y1="21" x2="16" y2="16" />
          <polyline points="8 11 10.5 13.5 14 9" />
        </>
      )}
      {kind === "orchestrator" && (
        <>
          <circle cx="6" cy="5" r="2.5" />
          <circle cx="18" cy="12" r="2.5" />
          <circle cx="6" cy="19" r="2.5" />
          <path d="M6 7.5v9" />
          <path d="M6 7.5c0 3 3 4.5 9.5 4.5" />
        </>
      )}
      {kind === "verifier" && (
        <>
          <rect x="5" y="4" width="14" height="17" rx="2" />
          <path d="M9 4V3h6v1" />
          <polyline points="9 13 11 15 15 11" />
        </>
      )}
    </svg>
  );
}
