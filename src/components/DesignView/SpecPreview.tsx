// src/components/DesignView/SpecPreview.tsx
import { useState } from 'react';
import { MarkdownText } from '../../lib/markdown';
import { designSaveFile } from '../../ipc/design';

interface SpecPreviewProps {
  title?: string;
  spec: string | null;
  sdd: string | null;
  plan: string | null;
}

export function SpecPreview({ title, spec, sdd, plan }: SpecPreviewProps) {
  const [activeTab, setActiveTab] = useState<'spec' | 'sdd' | 'plan'>('spec');
  const [saving, setSaving] = useState(false);
  
  // Custom dialog state
  const [promptOpen, setPromptOpen] = useState(false);
  const [paths, setPaths] = useState({ spec: '', sdd: '', plan: '' });
  const [alertMsg, setAlertMsg] = useState<{ type: 'success'|'error', text: string } | null>(null);

  const tabs = [
    { id: 'spec', label: '規格 (Spec)', content: spec },
    { id: 'sdd', label: '架構 (SDD)', content: sdd },
    { id: 'plan', label: '計畫 (Plan)', content: plan },
  ] as const;

  const rawContent = tabs.find((t) => t.id === activeTab)?.content;
  
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

  const handleSaveClick = () => {
    if (!spec && !sdd && !plan) return;

    const dateStr = new Date().toISOString().split('T')[0];
    const safeTitle = (title || 'feature').toLowerCase().replace(/\s+/g, '-').replace(/[^a-z0-9-]/g, '');
    
    setPaths({
      spec: `docs/superpowers/specs/${dateStr}-${safeTitle}-design.md`,
      sdd: `docs/superpowers/specs/${dateStr}-${safeTitle}-architecture.md`,
      plan: `docs/superpowers/plans/${dateStr}-${safeTitle}-plan.md`
    });
    setPromptOpen(true);
    setAlertMsg(null);
  };

  const executeSave = async () => {
    setPromptOpen(false);
    setSaving(true);
    setAlertMsg(null);
    try {
      const savedPaths: string[] = [];
      
      if (spec && paths.spec) {
        await designSaveFile(paths.spec, spec);
        savedPaths.push(paths.spec);
      }
      if (sdd && paths.sdd) {
        await designSaveFile(paths.sdd, sdd);
        savedPaths.push(paths.sdd);
      }
      if (plan && paths.plan) {
        await designSaveFile(paths.plan, plan);
        savedPaths.push(paths.plan);
      }

      setAlertMsg({ type: 'success', text: `✅ 儲存成功！\n已寫入檔案：\n${savedPaths.join('\n')}` });
      setTimeout(() => setAlertMsg(null), 8000);
    } catch (err) {
      setAlertMsg({ type: 'error', text: '❌ 儲存失敗：\n' + String(err) });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="design-preview-container" style={{ position: 'relative' }}>
      <div className="design-preview-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <div className="design-tabs">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              className={`design-tab-btn ${activeTab === tab.id ? 'active' : ''}`}
              onClick={() => setActiveTab(tab.id)}
            >
              {tab.label}
            </button>
          ))}
        </div>
        {(spec || sdd || plan) && (
          <button 
            className="design-provider-btn" 
            style={{ padding: '4px 12px', display: 'flex', alignItems: 'center', gap: '6px' }}
            onClick={handleSaveClick}
            disabled={saving}
          >
            {saving ? '儲存中...' : '💾 儲存至專案'}
          </button>
        )}
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
          <div className="design-empty-state">
            <p>尚未產生內容，請在左側開始討論。</p>
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
            <h3 style={{ margin: '0 0 16px 0', fontSize: '1.1rem', color: '#eee' }}>確認儲存路徑</h3>
            <p style={{ color: '#888', fontSize: '0.85rem', marginBottom: '20px' }}>
              您可以手動修改各文件的儲存路徑。內容為空的部分將不會建立檔案。
            </p>
            
            <div style={{ display: 'flex', flexDirection: 'column', gap: '16px', marginBottom: '24px' }}>
              {spec !== null && (
                <div>
                  <label style={{ display: 'block', color: '#aaa', fontSize: '0.75rem', marginBottom: '4px' }}>📄 規格 (Spec) 儲存路徑</label>
                  <input type="text" value={paths.spec} onChange={e => setPaths({...paths, spec: e.target.value})} style={inputStyle} />
                </div>
              )}
              {sdd !== null && (
                <div>
                  <label style={{ display: 'block', color: '#aaa', fontSize: '0.75rem', marginBottom: '4px' }}>🏗️ 架構 (SDD) 儲存路徑</label>
                  <input type="text" value={paths.sdd} onChange={e => setPaths({...paths, sdd: e.target.value})} style={inputStyle} />
                </div>
              )}
              {plan !== null && (
                <div>
                  <label style={{ display: 'block', color: '#aaa', fontSize: '0.75rem', marginBottom: '4px' }}>📅 計畫 (Plan) 儲存路徑</label>
                  <input type="text" value={paths.plan} onChange={e => setPaths({...paths, plan: e.target.value})} style={inputStyle} />
                </div>
              )}
            </div>

            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '12px' }}>
              <button 
                onClick={() => setPromptOpen(false)}
                style={{ background: 'transparent', border: '1px solid #555', color: '#aaa', padding: '6px 16px', borderRadius: '4px', cursor: 'pointer' }}
              >取消</button>
              <button 
                onClick={executeSave}
                style={{ background: '#4caf50', border: 'none', color: '#fff', padding: '6px 16px', borderRadius: '4px', cursor: 'pointer', fontWeight: 'bold' }}
              >確認儲存</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}