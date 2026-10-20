import { useEffect, useLayoutEffect, useState, type RefObject } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { AgentRow } from "../../ipc/agents";
import type { AccessLink, Resource } from "./resources";
import { linkPath, railItems } from "./resources";
import { RoleIcon } from "./RoleIcon";

/** 資源區與最下面那張 Agent 卡的距離；資源區最寬多寬才換行（桌布座標）。
 *  資源一律橫排：直排時線要穿過上面的膠囊才接得到下面的（實機截圖）。 */
const RAIL_GAP = 64;
const RAIL_MAX_W = 1200;

interface Pos {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * 桌布底部的資源區（多位 Agent 共用的外部資源）與存取線。放在桌布座標裡，跟著縮放與「全部顯示」。
 * 線平常不畫：只有查詢中、等核准、剛被擋下，或滑鼠停在資源上（看誰有權限）時才出現。
 * 原型與設計說明見 .superpowers/brainstorm/agent-db-resource。
 */
export function ResourceLayer({
  containerRef,
  agents,
  resources,
  links,
  agentName,
  revision,
  onExtent,
}: {
  containerRef: RefObject<HTMLElement | null>;
  agents: AgentRow[];
  resources: Resource[];
  links: AccessLink[];
  agentName: (id: string) => string;
  /** 卡片內容一變高度就可能變，用它觸發重新量測。 */
  revision: unknown;
  /** 資源區（含展開的清單）的右緣與底端；桌布大小要把它算進去。 */
  onExtent: (e: { right: number; bottom: number }) => void;
}) {
  const { t } = useLocale();
  const [hover, setHover] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [geo, setGeo] = useState<{ rail: Pos; railW: number; agents: Record<string, Pos>; pills: Record<string, Pos> } | null>(null);

  const busy = new Set(links.filter((l) => l.state !== "denied").map((l) => l.resourceId));
  /** 正在讀這個參考資料夾的人。 */
  const readers = (id: string) =>
    [...new Set(links.filter((l) => l.resourceId === id && l.state === "query").map((l) => agentName(l.agentId)))].join("、");
  const items = railItems(resources, busy);
  const stateOf = (id: string) =>
    links.some((l) => l.resourceId === id && l.state === "wait") ? "wait" : links.some((l) => l.resourceId === id && l.state === "query") ? "query" : null;

  // 量 Agent 卡與膠囊的位置：卡片高度隨內容變，膠囊寬度隨名稱變。有變才更新，不會無限迴圈。
  useLayoutEffect(() => {
    const root = containerRef.current;
    if (!root || resources.length === 0) return;
    const agentPos: Record<string, Pos> = {};
    let bottom = 0;
    let left = Infinity;
    let right = 0;
    for (const a of agents) {
      const el = root.querySelector<HTMLElement>(`[data-agent-id="${a.id}"]`);
      const p = { x: a.canvas_x, y: a.canvas_y, w: el?.offsetWidth || 248, h: el?.offsetHeight || 120 };
      agentPos[a.id] = p;
      bottom = Math.max(bottom, p.y + p.h);
      left = Math.min(left, p.x);
      right = Math.max(right, p.x + p.w);
    }
    if (!Number.isFinite(left)) left = 24;
    const rail = { x: left, y: bottom + RAIL_GAP, w: Math.max(RAIL_MAX_W, right - left), h: 0 };
    const railEl = root.querySelector<HTMLElement>("[data-resource-rail]");
    rail.h = railEl?.offsetHeight || 72;
    const railW = railEl?.offsetWidth || 0;
    const pills: Record<string, Pos> = {};
    root.querySelectorAll<HTMLElement>("[data-resource-id]").forEach((el) => {
      pills[el.dataset.resourceId as string] = { x: rail.x + el.offsetLeft, y: rail.y + el.offsetTop, w: el.offsetWidth, h: el.offsetHeight };
    });
    const next = { rail, railW, agents: agentPos, pills };
    if (JSON.stringify(next) !== JSON.stringify(geo)) setGeo(next);
  }, [containerRef, agents, resources, links, revision, open, geo]);

  useEffect(() => {
    if (!geo) return;
    const pop = containerRef.current?.querySelector<HTMLElement>("[data-resource-pop]");
    onExtent({ right: geo.rail.x + Math.max(geo.railW, pop?.offsetWidth ?? 0), bottom: geo.rail.y + geo.rail.h + (pop?.offsetHeight ?? 0) });
  }, [geo, open, containerRef, onExtent]);

  // 點別處就收起清單。
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest?.("[data-resource-group]")) setOpen(null);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);

  if (resources.length === 0) return null;
  const modeLabel = (m: Resource["holders"][number]["mode"]) => t.ac_db_mode_short(m);

  // 要畫的線：使用中的狀態，加上滑鼠停著那個資源的所有權限（灰色）。
  // 標籤寫「誰 權限」，在資源上方往上疊，好幾位共用時才不會疊在一起。
  const drawn: { agentId: string; resourceId: string; state: string; label?: string; row?: number }[] = links.map((l) => ({ ...l }));
  if (hover) {
    (resources.find((r) => r.id === hover)?.holders ?? []).forEach((h, row) => {
      const label = `${agentName(h.agentId)} ${modeLabel(h.mode)}`;
      const existing = drawn.find((d) => d.agentId === h.agentId && d.resourceId === hover);
      if (existing) Object.assign(existing, { label, row });
      else drawn.push({ agentId: h.agentId, resourceId: hover, state: "hover", label, row });
    });
  }

  return (
    <>
      <svg className="ac-res-edges" aria-hidden>
        {geo &&
          drawn.map((d) => {
            const a = geo.agents[d.agentId];
            const p = geo.pills[d.resourceId];
            if (!a || !p) return null; // 收合中的閒置資源沒有線
            const gapMid = geo.rail.y - RAIL_GAP / 2;
            const others = Object.entries(geo.agents).filter(([id]) => id !== d.agentId).map(([, r]) => r);
            const x2 = p.x + p.w / 2;
            return (
              <g key={`${d.agentId}-${d.resourceId}`} data-testid={`access-${d.agentId}-${d.resourceId}`} data-state={d.state}>
                <path className={`ac-res-link ac-res-link--${d.state}`} d={linkPath(a, p, others, gapMid)} />
                {d.label && (
                  // 標籤放在資源區上方的空白裡（資源正上方），不壓到資源區的框，也不疊在卡片上。
                  <text className="ac-res-tag" x={x2 + 8} y={gapMid - 8 - (d.row ?? 0) * 15}>
                    {d.label}
                  </text>
                )}
              </g>
            );
          })}
      </svg>
      <div
        className="ac-res-rail"
        data-resource-rail
        data-testid="ac-resource-rail"
        // 寬度跟著內容橫排（不受桌布目前的寬度限制），超過上限才換行；桌布再依它長大。
        style={geo ? { left: geo.rail.x, top: geo.rail.y, width: "max-content", maxWidth: geo.rail.w } : { visibility: "hidden" }}
      >
        <span className="ac-res-rail-label" title={t.ac_res_rail_tip}>
          {t.ac_res_rail}
        </span>
        {items.map((it) => {
          if (it.kind === "one") {
            const r = it.resource;
            const st = stateOf(r.id);
            return (
              <div
                key={r.id}
                className={`ac-res${st === "query" ? " ac-res--hot" : st === "wait" ? " ac-res--wait" : ""}`}
                data-resource-id={r.id}
                data-testid={`resource-${r.id}`}
                onMouseEnter={() => setHover(r.id)}
                onMouseLeave={() => setHover((h) => (h === r.id ? null : h))}
              >
                <span className="ac-res-ico" aria-hidden>
                  <RoleIcon name={r.type === "folder" ? "folder" : "database"} size={16} />
                </span>
                <span className="ac-res-text">
                  <span className="ac-res-name">{r.name}</span>
                  {r.sub && <span className="ac-res-sub">{r.sub}</span>}
                </span>
                <span className={st ? "ac-res-state" : "ac-res-count"}>
                  {st === "query"
                    ? r.type === "folder"
                      ? `● ${t.ac_res_reading(readers(r.id))}`
                      : `● ${t.ac_res_querying}`
                    : st === "wait"
                      ? `⏳ ${t.ac_res_waiting}`
                      : r.everyone
                        ? t.ac_res_everyone
                        : r.holders.length === 1
                        ? `${agentName(r.holders[0].agentId)} · ${modeLabel(r.holders[0].mode)}`
                        : t.ac_res_holders(r.holders.length)}
                </span>
              </div>
            );
          }
          return (
            <div key={`g-${it.type}`} className="ac-res ac-res--group" data-resource-group data-testid={`resource-group-${it.type}`}>
              <button type="button" className="ac-res-group-btn" onClick={() => setOpen((o) => (o === it.type ? null : it.type))}>
                <span className="ac-res-ico" aria-hidden>
                  <RoleIcon name={it.type === "folder" ? "folder" : "database"} size={16} />
                </span>
                <span className="ac-res-text">
                  <span className="ac-res-name">
                    {it.type === "folder" ? t.ac_res_folders : t.ac_skill_db} ×{it.resources.length}
                  </span>
                  <span className="ac-res-sub">{t.ac_res_idle_hint}</span>
                </span>
              </button>
              {open === it.type && (
                <div className="ac-res-pop" data-resource-pop role="list">
                  {it.resources.map((r) => (
                    <div key={r.id} className="ac-res-pop-item" role="listitem">
                      <RoleIcon name={r.type === "folder" ? "folder" : "database"} size={13} />
                      <span>{r.name}</span>
                      <span className="ac-res-pop-who">
                        {r.everyone ? t.ac_res_everyone : r.holders.map((h) => `${agentName(h.agentId)}（${modeLabel(h.mode)}）`).join("、")}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </>
  );
}
