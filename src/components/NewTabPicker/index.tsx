import { useEffect, useRef, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import type { TabType } from "../TabBar";
import { bridgeStatus } from "../../ipc/bridge";
import {
  TerminalIcon,
  DatabaseIcon,
  PaintbrushIcon,
  LinkIcon,
  BranchIcon,
  FileTextIcon,
  RefreshIcon,
  CodeIcon,
  LibraryIcon,
  RobotIcon,
} from "../Icons";
import "./index.css";

interface Props {
  /** opts.claudeBridge 只有「新增 Claude Code 分頁」那個選項會帶 true。 */
  onSelect: (type: TabType, opts?: { claudeBridge?: boolean }) => void;
  onClose: () => void;
}

export function NewTabPicker({ onSelect, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const { t } = useLocale();

  // 橋接 server 沒在跑就不給選——建立一個注入了死埠位址的分頁，比不給選更難除錯。
  const [bridgeRunning, setBridgeRunning] = useState(false);
  useEffect(() => {
    bridgeStatus().then((s) => setBridgeRunning(s.running)).catch(() => {});
  }, []);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("keydown", handleKey);
    document.addEventListener("mousedown", handleClick);
    return () => {
      document.removeEventListener("keydown", handleKey);
      document.removeEventListener("mousedown", handleClick);
    };
  }, [onClose]);

  const items: { type: TabType; icon: React.ReactNode; label: string; desc: string }[] = [
    { type: "terminal",      icon: <TerminalIcon size={18} />,    label: t.terminal_tab,       desc: t.new_terminal_desc },
    { type: "database",      icon: <DatabaseIcon size={18} />,    label: t.database_tab,       desc: t.new_database_desc },
    { type: "design",        icon: <PaintbrushIcon size={18} />,  label: t.design_tab,         desc: t.new_design_desc },
    { type: "cross-db",      icon: <LinkIcon size={18} />,        label: t.cross_db_tab,       desc: t.new_cross_db_desc },
    { type: "vcs",           icon: <BranchIcon size={18} />,      label: t.vcs_tab,            desc: t.new_vcs_desc },
    { type: "doc-converter", icon: <FileTextIcon size={18} />,    label: t.doc_converter_tab,  desc: t.new_doc_converter_desc },
    { type: "loop-studio",   icon: <RefreshIcon size={18} />,     label: t.loop_studio_tab,    desc: t.new_loop_studio_desc },
    { type: "code-assistant", icon: <CodeIcon size={18} />,       label: t.code_assistant_tab, desc: t.new_code_assistant_desc },
    { type: "knowledge-base", icon: <LibraryIcon size={18} />,     label: t.knowledge_base_tab, desc: t.new_knowledge_base_desc },
    // The Mail entry is hidden while the feature is not being shipped to
    // users. The "mail" tab type, MailView, and its whole backend remain wired
    // up — restoring it means putting this line back, plus the matching button
    // in SettingsView, and re-adding the MailIcon import above. See the notes in
    // docs/superpowers/specs/2026-08-04-ai-mail-assistant-design.md.
    // { type: "mail",         icon: <MailIcon size={18} />,       label: t.mail_tab,           desc: t.new_mail_desc },
  ];

  return (
    <div className="new-tab-picker" ref={ref}>
      {items.map(({ type, icon, label, desc }) => (
        <button
          key={type}
          className="new-tab-picker__item"
          onClick={() => { onSelect(type); onClose(); }}
        >
          <span className="new-tab-picker__icon">{icon}</span>
          <div>
            <div className="new-tab-picker__label">{label}</div>
            <div className="new-tab-picker__desc">{desc}</div>
          </div>
        </button>
      ))}
      {/* 橋接未啟動時停用——建立一個注入了死埠位址的分頁，比不給選更難除錯。 */}
      <button
        className="new-tab-picker__item"
        disabled={!bridgeRunning}
        title={bridgeRunning ? undefined : t.bridge_new_tab_disabled_hint}
        onClick={() => { onSelect("terminal", { claudeBridge: true }); onClose(); }}
      >
        <span className="new-tab-picker__icon"><RobotIcon size={18} /></span>
        <div>
          <div className="new-tab-picker__label">{t.bridge_new_tab}</div>
          <div className="new-tab-picker__desc">
            {bridgeRunning ? t.bridge_new_tab_desc : t.bridge_new_tab_disabled_hint}
          </div>
        </div>
      </button>
    </div>
  );
}
