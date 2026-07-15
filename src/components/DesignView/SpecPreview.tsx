// src/components/DesignView/SpecPreview.tsx
import { useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { useLocale } from '../../contexts/LocaleContext';
import { MarkdownText } from '../../lib/markdown';
import { designSaveFile } from '../../ipc/design';

interface SpecPreviewProps {
  title?: string;
  proposal: string | null;
  spec: string | null;
  sdd: string | null;
  plan: string | null;
  onGenerate?: (stage: 'proposal' | 'spec' | 'sdd' | 'plan') => void;
  isStreaming?: boolean;
}

type TabId = 'proposal' | 'spec' | 'sdd' | 'plan';

export function SpecPreview({ title, proposal, spec, sdd, plan, onGenerate, isStreaming }: SpecPreviewProps) {
  const { t } = useLocale();
  const [activeTab, setActiveTab] = useState<TabId>('proposal');

  const TAB_META = [
    { id: 'proposal' as TabId, label: t.design_tab_proposal, generateLabel: t.design_generate_proposal, regenerateLabel: t.design_regenerate },
    { id: 'spec' as TabId, label: t.design_tab_spec, generateLabel: t.design_generate_spec, regenerateLabel: t.design_regenerate },
    { id: 'sdd' as TabId, label: t.design_tab_sdd, generateLabel: t.design_generate_sdd, regenerateLabel: t.design_regenerate },
    { id: 'plan' as TabId, label: t.design_tab_plan, generateLabel: t.design_generate_plan, regenerateLabel: t.design_regenerate },
  ];
  const [saving, setSaving] = useState(false);

  // Custom dialog state
  const [promptOpen, setPromptOpen] = useState(false);
  const [paths, setPaths] = useState({ dir: '', proposal: '', spec: '', sdd: '', plan: '' });
  const [alertMsg, setAlertMsg] = useState<{ type: 'success'|'error', text: string } | null>(null);

  const contentMap: Record<TabId, string | null> = { proposal, spec, sdd, plan };

  const rawContent = contentMap[activeTab];

  // Strip outer markdown fences for legacy database entries that were saved with them
  let activeContent = rawContent;
  if (activeContent) {
    const trimmed = activeContent.trim();
    if (trimmed.startsWith('```markdown') || trimmed.startsWith('```md')) {
      const firstNewline = trimmed.indexOf('\n');
      if (firstNewline !== -1) {
        activeContent = trimmed.slice(firstNewline + 1).trim();
        if (activeContent.endsWith('```')) {
          activeContent = activeContent.slice(0, activeContent.length - 3).trim();
        }
      }
    }
  }

  const inputStyle = {
    width: '100%', padding: '8px 10px',
    backgroundColor: '#000', border: '1px solid #333',
    color: '#fff', borderRadius: '4px', fontFamily: 'monospace',
    fontSize: '0.8rem', boxSizing: 'border-box' as const
  };

  const hasAnyContent = proposal || spec || sdd || plan;

  const handleSaveClick = () => {
    if (!hasAnyContent) return;

    const safeTitle = (title || 'feature').toLowerCase().replace(/\s+/g, '-').replace(/[^a-z0-9-]/g, '');

    setPaths({
      dir: `openspec/changes/${safeTitle}`,
      proposal: 'proposal.md',
      spec: 'spec.md',
      sdd: 'design.md',
      plan: 'tasks.md',
    });
    setPromptOpen(true);
    setAlertMsg(null);
  };

  /** Split spec by ## Capability: headings into individual files under specs/ */
  const saveSpecFiles = async (dir: string, specContent: string): Promise<string[]> => {
    const saved: string[] = [];
    const capabilityRegex = /^## Capability:\s*(.+)$/gm;
    const matches = [...specContent.matchAll(capabilityRegex)];

    if (matches.length === 0) {
      // No capability headings — save as single file
      const full = `${dir}/specs/spec.md`;
      await designSaveFile(full, specContent);
      saved.push(full);
    } else {
      for (let i = 0; i < matches.length; i++) {
        const name = matches[i][1].trim().toLowerCase().replace(/\s+/g, '-').replace(/[^a-z0-9-]/g, '');
        const start = matches[i].index!;
        const end = i + 1 < matches.length ? matches[i + 1].index! : specContent.length;
        const content = specContent.slice(start, end).trim();
        const full = `${dir}/specs/${name}.md`;
        await designSaveFile(full, content);
        saved.push(full);
      }
    }
    return saved;
  };

  const executeSave = async () => {
    setPromptOpen(false);
    setSaving(true);
    setAlertMsg(null);
    try {
      const savedPaths: string[] = [];
      const dir = paths.dir.replace(/\/+$/, '');

      if (proposal && paths.proposal) {
        const full = `${dir}/${paths.proposal}`;
        await designSaveFile(full, proposal);
        savedPaths.push(full);
      }
      if (spec) {
        const specPaths = await saveSpecFiles(dir, spec);
        savedPaths.push(...specPaths);
      }
      if (sdd && paths.sdd) {
        const full = `${dir}/${paths.sdd}`;
        await designSaveFile(full, sdd);
        savedPaths.push(full);
      }
      if (plan && paths.plan) {
        const full = `${dir}/${paths.plan}`;
        await designSaveFile(full, plan);
        savedPaths.push(full);
      }

      setAlertMsg({ type: 'success', text: t.design_save_success(savedPaths.join('\n')) });
      setTimeout(() => setAlertMsg(null), 8000);
    } catch (err) {
      setAlertMsg({ type: 'error', text: t.design_save_error(String(err)) });
    } finally {
      setSaving(false);
    }
  };

  const activeTabMeta = TAB_META.find((t) => t.id === activeTab)!;

  return (
    <div className="design-preview-container" style={{ position: 'relative' }}>
      <div className="design-preview-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <div className="design-tabs">
          {TAB_META.map((tab) => (
            <button
              key={tab.id}
              className={`design-tab-btn ${activeTab === tab.id ? 'active' : ''}`}
              onClick={() => setActiveTab(tab.id)}
            >
              {tab.label}
            </button>
          ))}
        </div>
        <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
          {activeContent && onGenerate && (
            <button
              className="design-provider-btn"
              style={{ padding: '4px 10px', fontSize: '0.75rem' }}
              onClick={() => onGenerate(activeTab)}
              disabled={isStreaming}
              title={t.design_regenerate_title(activeTabMeta.label)}
            >
              {activeTabMeta.regenerateLabel}
            </button>
          )}
          {hasAnyContent && (
            <button
              className="design-provider-btn"
              style={{ padding: '4px 12px', display: 'flex', alignItems: 'center', gap: '6px' }}
              onClick={handleSaveClick}
              disabled={saving}
            >
              {saving ? t.design_saving : t.design_save_to_project}
            </button>
          )}
        </div>
      </div>
      <div className="design-preview-body">
        {alertMsg && (
          <div style={{
            padding: '12px',
            marginBottom: '16px',
            borderRadius: '6px',
            backgroundColor: alertMsg.type === 'success' ? 'rgba(76, 175, 80, 0.1)' : 'rgba(244, 67, 54, 0.1)',
            border: `1px solid ${alertMsg.type === 'success' ? '#4caf50' : '#f44336'}`,
            color: alertMsg.type === 'success' ? '#81c784' : '#e57373',
            whiteSpace: 'pre-wrap',
            fontSize: '0.9rem'
          }}>
            {alertMsg.text}
          </div>
        )}

        {activeContent ? (
          <MarkdownText text={activeContent} />
        ) : (
          <div className="design-empty-state" style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: '16px' }}>
            <p>{t.design_empty_state}</p>
            {onGenerate && (
              <button
                className="design-provider-btn"
                style={{ padding: '8px 20px', fontSize: '0.9rem' }}
                onClick={() => onGenerate(activeTab)}
                disabled={isStreaming}
              >
                {activeTabMeta.generateLabel}
              </button>
            )}
          </div>
        )}
      </div>

      {/* Save Prompt Modal */}
      {promptOpen && (
        <div style={{
          position: 'absolute', top: 0, left: 0, right: 0, bottom: 0,
          backgroundColor: 'rgba(0,0,0,0.7)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          zIndex: 100
        }}>
          <div style={{
            backgroundColor: '#1a1a1a', border: '1px solid #333',
            borderRadius: '8px', padding: '24px', width: '90%', maxWidth: '600px',
            boxShadow: '0 10px 30px rgba(0,0,0,0.5)'
          }}>
            <h3 style={{ margin: '0 0 16px 0', fontSize: '1.1rem', color: '#eee' }}>{t.design_confirm_save_title}</h3>
            <p style={{ color: '#888', fontSize: '0.85rem', marginBottom: '20px' }}>
              {t.design_confirm_save_hint}
            </p>

            <div style={{ display: 'flex', flexDirection: 'column', gap: '16px', marginBottom: '24px' }}>
              <div>
                <label style={{ display: 'block', color: '#aaa', fontSize: '0.75rem', marginBottom: '4px' }}>{t.design_save_dir}</label>
                <div style={{ display: 'flex', gap: '8px' }}>
                  <input type="text" value={paths.dir} onChange={e => setPaths({...paths, dir: e.target.value})} style={{ ...inputStyle, flex: 1 }} />
                  <button
                    onClick={async () => {
                      const selected = await open({ directory: true, multiple: false });
                      if (typeof selected === 'string') setPaths({...paths, dir: selected});
                    }}
                    title={t.design_browse_folder}
                    style={{ background: '#2a2a2a', border: '1px solid #444', color: '#ccc', padding: '0 10px', borderRadius: '4px', cursor: 'pointer', fontSize: '1rem', lineHeight: 1 }}
                  >📂</button>
                </div>
              </div>
              <div style={{ borderTop: '1px solid #333', paddingTop: '12px' }}>
                <label style={{ display: 'block', color: '#666', fontSize: '0.7rem', marginBottom: '8px' }}>{t.design_save_filenames_hint}</label>
              </div>
              {proposal !== null && (
                <div>
                  <label style={{ display: 'block', color: '#aaa', fontSize: '0.75rem', marginBottom: '4px' }}>{t.design_save_proposal_label}</label>
                  <input type="text" value={paths.proposal} onChange={e => setPaths({...paths, proposal: e.target.value})} style={inputStyle} />
                </div>
              )}
              {spec !== null && (
                <div>
                  <label style={{ display: 'block', color: '#aaa', fontSize: '0.75rem', marginBottom: '4px' }}>{t.design_save_spec_label}</label>
                  <input type="text" value={paths.spec} onChange={e => setPaths({...paths, spec: e.target.value})} style={inputStyle} disabled />
                </div>
              )}
              {sdd !== null && (
                <div>
                  <label style={{ display: 'block', color: '#aaa', fontSize: '0.75rem', marginBottom: '4px' }}>{t.design_save_sdd_label}</label>
                  <input type="text" value={paths.sdd} onChange={e => setPaths({...paths, sdd: e.target.value})} style={inputStyle} />
                </div>
              )}
              {plan !== null && (
                <div>
                  <label style={{ display: 'block', color: '#aaa', fontSize: '0.75rem', marginBottom: '4px' }}>{t.design_save_plan_label}</label>
                  <input type="text" value={paths.plan} onChange={e => setPaths({...paths, plan: e.target.value})} style={inputStyle} />
                </div>
              )}
            </div>

            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '12px' }}>
              <button
                onClick={() => setPromptOpen(false)}
                style={{ background: 'transparent', border: '1px solid #555', color: '#aaa', padding: '6px 16px', borderRadius: '4px', cursor: 'pointer' }}
              >{t.cancel}</button>
              <button
                onClick={executeSave}
                style={{ background: '#4caf50', border: 'none', color: '#fff', padding: '6px 16px', borderRadius: '4px', cursor: 'pointer', fontWeight: 'bold' }}
              >{t.design_confirm_save}</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
