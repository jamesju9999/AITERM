import { useEffect, useRef } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import type { TabType } from "../TabBar";
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
} from "../Icons";
import "./index.css";

interface Props {
  onSelect: (type: TabType) => void;
  onClose: () => void;
}

export function NewTabPicker({ onSelect, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const { t } = useLocale();

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
    </div>
  );
}
