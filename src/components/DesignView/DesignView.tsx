// src/components/DesignView/DesignView.tsx
import { useState, useEffect, useRef, useCallback } from 'react';
import { RobotIcon } from '../Icons';
import { SpecPreview } from './SpecPreview';
import {
  designStartSession,
  designChat,
  designUpdateDraft,
  designLoadSession,
  designListMessages,
  designListSessions,
  designDeleteSession,
} from '../../ipc/design';
import type { DesignSession } from '../../ipc/design';
import type { ChatMessage, ContentPart } from '../../ipc/ai';

function contentToString(content: string | ContentPart[] | null): string {
  if (!content) return "";
  if (typeof content === 'string') return content;
  return content
    .filter((p): p is Extract<ContentPart, { type: 'text' }> => p.type === 'text')
    .map((p) => p.text)
    .join(' ');
}
import { listen } from '@tauri-apps/api/event';
import { useTelegramRemoteControl } from '../../hooks/useTelegramRemoteControl';
import { MessageBubble } from '../AiPanel/MessageBubble';
import { ProviderPalette } from '../ProviderPalette';
import { listProviders } from '../../ipc/provider';
import { extractResponseText } from '../../lib/markdown';
import { getConfig, type SubmitShortcut } from '../../ipc/config';
import { useLocale } from '../../contexts/LocaleContext';
import './DesignView.css';

export function DesignView({ isActive }: { isActive: boolean }) {
  const { t, locale } = useLocale();
  const [session, setSession] = useState<DesignSession | null>(null);
  const [loading, setLoading] = useState(false);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [inputValue, setInputValue] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const [leftWidth, setLeftWidth] = useState(600);
  const [isResizing, setIsResizing] = useState(false);
  const [providerId, setProviderId] = useState<string | undefined>(undefined);
  const [providerName, setProviderName] = useState<string | undefined>(undefined);
  const [showProviderPalette, setShowProviderPalette] = useState(false);
  const [submitShortcut, setSubmitShortcut] = useState<SubmitShortcut>('enter');
  const [historyOpen, setHistoryOpen] = useState(false);
  const [sessionList, setSessionList] = useState<DesignSession[]>([]);

  const containerRef = useRef<HTMLDivElement>(null);
  const messagesListRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    listProviders()
      .then((list) => {
        if (cancelled) return;
        const active = list.find((p) => p.is_default) ?? list[0];
        if (active && !providerId) {
          setProviderId(active.id);
          setProviderName(active.display_name);
        }
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [providerId]);

  // Auto-scroll to bottom
  useEffect(() => {
    getConfig().then(cfg => setSubmitShortcut(cfg.submit_shortcut)).catch(() => {});
  }, []);

  // useNavigate hook wrapper / local import helper (or let's import it)
  // Let's check if useNavigate is already imported. In line 1 it wasn't. Let's make sure it is imported from react-router-dom.
  // Oh, wait! In original code, navigate was defined as const navigate = useNavigate(); but where is the import?
  // Let's verify line 2 in original file: import { useState, useEffect, useRef, useCallback } from 'react';
  // Ah! There was no useNavigate import in original code. Wait, yes, in line 2: it didn't import it, but used it? Let's check if there is an import we missed.
  // Wait, the original code had `import { SpecPreview } from './SpecPreview';` etc. But let's check if we missed `import { useNavigate } from 'react-router-dom';` in original code.
  // Wait, let's look at the original code lines 1 to 30:
  // Yes! The original code did not import `useNavigate`! Wait, it did not use `navigate`?
  // Ah! Yes, in line 19: `const navigate = useNavigate();` was in the original code, but where is the import?
  // Ah, let's view original code line 1-15:
  // No `useNavigate` import? Wait, let's double check if there was any import from 'react-router-dom'.
  // Ah, wait! The original code didn't import it? No, wait, let's check line 31 in original view:
  // `export function DesignView({ isActive }: { isActive: boolean }) {`
  // `  const [session, setSession] = useState<DesignSession | null>(null);`
  // Wait, where is `navigate` used?
  // Ah! Let's check if `navigate` is used in `DesignView.tsx` at all.
  // In our search, we did not find `navigate` in `DesignView.tsx`! Let's check:
  // Ah, `navigate` is NOT used in `DesignView.tsx` at all! Let's verify if the original code had `const navigate = useNavigate();` or if it was just there.
  // Ah, line 19 of original view:
  // `  return content` -> wait, line 19 was in `contentToString` function!
  // Wait, in line 31 of original:
  // `export function DesignView({ isActive }: { isActive: boolean }) {`
  // And there was no `const navigate = useNavigate();` inside it in original code!
  // Oh, wait! In our proposed replacement:
  // `export function DesignView({ isActive }: { isActive: boolean }) {`
  // `  const { t } = useLocale();`
  // `  const navigate = useNavigate();`
  // Wait, did we write `const navigate = useNavigate();`?
  // Yes, in our proposed replacement we wrote `const navigate = useNavigate();`. But since it's not used, let's not define it to avoid unused variable lint errors!
  // That's a very smart catch! Let's not include `const navigate = useNavigate();`.

  useEffect(() => {
    const el = messagesListRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages]);

  const { isRemoteEnabled, setIsRemoteEnabled, sendRemoteResponse } = useTelegramRemoteControl(
    session?.id || '',
    isActive,
    (text) => {
      handleSendMessage(text);
    }
  );

  // Forward last assistant message to Telegram when streaming finishes
  const prevIsStreamingRef = useRef(false);
  useEffect(() => {
    if (prevIsStreamingRef.current && !isStreaming) {
      const lastMsg = messages[messages.length - 1];
      if (lastMsg && lastMsg.role === 'assistant' && sendRemoteResponse) {
        // Strip [UPDATE_*] tags and their content for Telegram display
        let text = contentToString(lastMsg.content);
        for (const tag of ['[UPDATE_PROPOSAL]', '[UPDATE_SPEC]', '[UPDATE_SDD]', '[UPDATE_PLAN]']) {
          const idx = text.indexOf(tag);
          if (idx !== -1) text = text.slice(0, idx).trim();
        }
        sendRemoteResponse(text || contentToString(lastMsg.content));
      }
    }
    prevIsStreamingRef.current = isStreaming;
  }, [isStreaming, messages, sendRemoteResponse]);

  // Handle Resize Logic
  useEffect(() => {
    if (!isResizing) {
      document.body.style.userSelect = '';
      return;
    }

    document.body.style.userSelect = 'none'; // Prevent text selection while dragging

    const onMouseMove = (e: MouseEvent) => {
      if (!containerRef.current) return;
      const containerRect = containerRef.current.getBoundingClientRect();
      const newWidth = e.clientX - containerRect.left;
      const constrainedWidth = Math.max(300, Math.min(newWidth, containerRect.width - 300));
      setLeftWidth(constrainedWidth);
    };

    const onMouseUp = () => setIsResizing(false);

    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
    return () => {
      window.removeEventListener('mousemove', onMouseMove);
      window.removeEventListener('mouseup', onMouseUp);
      document.body.style.userSelect = '';
    };
  }, [isResizing]);

  // Refresh current session data
  const refreshSession = useCallback(async () => {
    if (!session?.id || session.id === 'fallback-id') return;
    try {
      const updated = await designLoadSession(session.id);
      setSession(updated);
    } catch (err) {
      console.error('Failed to refresh session:', err);
    }
  }, [session?.id]);

  // Load session history
  const loadHistory = useCallback(async (sessionId: string) => {
    try {
      const history = await designListMessages(sessionId);
      if (history && history.length > 0) {
        setMessages(history);
      }
    } catch (err) {
      console.error('Failed to load design history:', err);
    }
  }, []);

  // Handle AI Streaming events
  useEffect(() => {
    if (!isActive || !session || session.id === 'fallback-id') return;
    const unlisten = listen<{ session_id: string; delta: string; done: boolean }>(
      'ai-stream',
      (event) => {
        if (event.payload.session_id !== session.id) return;
        setMessages((prev) => {
          const last = prev[prev.length - 1];
          if (last && last.role === 'assistant') {
            const updated = [...prev];
            updated[updated.length - 1] = { ...last, content: last.content + event.payload.delta };
            return updated;
          } else {
            return [...prev, { role: 'assistant', content: event.payload.delta }];
          }
        });
        if (event.payload.done) {
          setIsStreaming(false);
          setTimeout(refreshSession, 500);
        }
      }
    );
    return () => { unlisten.then((fn) => fn()); };
  }, [isActive, session, refreshSession]);

  // Initialize Session: load most recent unfinished session, or create new
  useEffect(() => {
    if (isActive && !session && !loading) {
      setLoading(true);
      designListSessions()
        .then(async (all) => {
          setSessionList(all);
          const unfinished = all.find((s) => s.status !== 'approved');
          if (unfinished) {
            const data = await designLoadSession(unfinished.id);
            setSession(data);
            await loadHistory(unfinished.id);
          } else {
            const id = await designStartSession(t.design_new_title);
            const data = await designLoadSession(id);
            setSession(data);
          }
        })
        .catch(() => {
          setSession({
            id: 'fallback-id', title: '後端未就緒', status: 'draft',
            current_proposal_draft: null,
            current_spec_draft: t.design_backend_err,
            current_sdd_draft: null, current_plan_draft: null, context_summary: null
          });
        })
        .finally(() => setLoading(false));
    }
  }, [isActive, session, loading, loadHistory, t]);

  // Refresh session list after session changes
  const refreshSessionList = useCallback(async () => {
    try {
      const all = await designListSessions();
      setSessionList(all);
    } catch { /* ignore */ }
  }, []);

  const handleNewSession = useCallback(async () => {
    if (isStreaming) return;
    try {
      const id = await designStartSession(t.design_new_title);
      const data = await designLoadSession(id);
      setSession(data);
      setMessages([]);
      await refreshSessionList();
    } catch { /* ignore */ }
  }, [isStreaming, refreshSessionList, t]);

  const handleLoadSession = useCallback(async (s: DesignSession) => {
    if (isStreaming) return;
    try {
      const data = await designLoadSession(s.id);
      setSession(data);
      setMessages([]);
      await loadHistory(s.id);
      setHistoryOpen(false);
    } catch { /* ignore */ }
  }, [isStreaming, loadHistory]);

  const handleDeleteSession = useCallback(async (id: string) => {
    try {
      await designDeleteSession(id);
      if (session?.id === id) {
        setSession(null);
        setMessages([]);
      }
      await refreshSessionList();
    } catch { /* ignore */ }
  }, [session?.id, refreshSessionList]);

  const handleSendMessage = useCallback(async (remoteText?: string) => {
    const text = typeof remoteText === "string" ? remoteText : inputValue;
    if (!text.trim() || !session || session.id === 'fallback-id' || isStreaming) return;
    const userMsg: ChatMessage = { role: 'user', content: text };

    // Combine consecutive messages of the same role to satisfy strict APIs (like Anthropic)
    const combinedMessages = [...messages, userMsg].reduce((acc, curr) => {
      const last = acc[acc.length - 1];
      if (last && last.role === curr.role) {
        last.content += '\n\n' + curr.content;
      } else {
        acc.push({ ...curr });
      }
      return acc;
    }, [] as ChatMessage[]);

    setMessages([...messages, userMsg]); // Keep UI showing individual bubbles
    if (typeof remoteText !== "string") setInputValue('');
    setIsStreaming(true);

    try {
      const response = await designChat(session.id, combinedMessages, providerId, locale);
      const cleanResponseText = extractResponseText(response.content);

      const extractContent = (tag: string, text: string) => {
        const startTag = `[${tag}]`;
        const startIdx = text.indexOf(startTag);
        if (startIdx === -1) return null;

        let endIdx = text.length;
        const otherTags = ['[UPDATE_PROPOSAL]', '[UPDATE_SPEC]', '[UPDATE_SDD]', '[UPDATE_PLAN]'].filter(t => t !== startTag);
        for (const otherTag of otherTags) {
          const idx = text.indexOf(otherTag, startIdx + startTag.length);
          if (idx !== -1 && idx < endIdx) {
            endIdx = idx;
          }
        }

        let content = text.slice(startIdx + startTag.length, endIdx).trim();

        // New Extremely Robust Unwrapper: Ignore any chatter before the first ``` block
        const firstFenceIdx = content.indexOf('```');
        if (firstFenceIdx !== -1) {
          const firstNewline = content.indexOf('\n', firstFenceIdx);
          if (firstNewline !== -1) {
            const firstLine = content.slice(firstFenceIdx, firstNewline).toLowerCase();
            // Check if it's a markdown block
            if (firstLine.includes('markdown') || firstLine.includes('md') || firstLine.trim() === '```') {
              content = content.slice(firstNewline + 1).trim();
              if (content.endsWith('```')) {
                content = content.slice(0, content.length - 3).trim();
              }
            }
          }
        }

        if (!content || content.length < 20) return null;

        return content;
      };

      const proposalContent = extractContent('UPDATE_PROPOSAL', cleanResponseText);
      const specContent = extractContent('UPDATE_SPEC', cleanResponseText);
      const sddContent = extractContent('UPDATE_SDD', cleanResponseText);
      const planContent = extractContent('UPDATE_PLAN', cleanResponseText);

      if (proposalContent) await designUpdateDraft(session.id, 'proposal', proposalContent);
      if (specContent) await designUpdateDraft(session.id, 'spec', specContent);
      if (sddContent) await designUpdateDraft(session.id, 'sdd', sddContent);
      if (planContent) await designUpdateDraft(session.id, 'plan', planContent);
      if (proposalContent || specContent || sddContent || planContent) refreshSession();

    } catch (err) {
      let errMsg = String(err);
      if (err && typeof err === 'object') {
        errMsg = (err as any).reason || (err as any).message || JSON.stringify(err);
      }
      setMessages(prev => [...prev, { role: 'assistant', content: t.design_error(errMsg) }]);
    } finally {
      setIsStreaming(false);
    }
  }, [inputValue, session, messages, isStreaming, refreshSession, providerId, t, locale]);

  const cleanMessageForDisplay = (text: string, streaming = false) => {
    if (!text) return text;
    let cleaned = text;

    const tags = streaming ? [
      { tag: '[UPDATE_PROPOSAL]', msg: t.design_generating_proposal },
      { tag: '[UPDATE_SPEC]', msg: t.design_generating_spec },
      { tag: '[UPDATE_SDD]', msg: t.design_generating_sdd },
      { tag: '[UPDATE_PLAN]', msg: t.design_generating_plan },
    ] : [
      { tag: '[UPDATE_PROPOSAL]', msg: t.design_updated_proposal },
      { tag: '[UPDATE_SPEC]', msg: t.design_updated_spec },
      { tag: '[UPDATE_SDD]', msg: t.design_updated_sdd },
      { tag: '[UPDATE_PLAN]', msg: t.design_updated_plan },
    ];

    for (const { tag, msg } of tags) {
      while (true) {
        const startIdx = cleaned.indexOf(tag);
        if (startIdx === -1) break;

        let endIdx = cleaned.length;
        const otherTags = tags.map(t => t.tag).filter(t => t !== tag);
        for (const otherTag of otherTags) {
          const idx = cleaned.indexOf(otherTag, startIdx + tag.length);
          if (idx !== -1 && idx < endIdx) {
            endIdx = idx;
          }
        }

        cleaned = cleaned.slice(0, startIdx) + '\n\n' + msg + '\n\n' + cleaned.slice(endIdx);
      }
    }

    return cleaned.trim();
  };

  const handleGenerate = useCallback((stage: 'proposal' | 'spec' | 'sdd' | 'plan') => {
    if (isStreaming || !session || session.id === 'fallback-id') return;
    const text = t.design_generate_command(stage);
    handleSendMessage(text);
  }, [isStreaming, session, handleSendMessage, t]);

  const statusLabels: Record<string, string> = {
    draft: t.design_status_draft,
    proposal_approved: t.design_status_proposal_approved,
    spec_approved: t.design_status_spec_approved,
    sdd_approved: t.design_status_sdd_approved,
    approved: t.design_status_approved,
  };

  return (
    <div className="design-view-root" ref={containerRef}>
      <div className="design-left-panel" style={{ width: `${leftWidth}px`, flexBasis: `${leftWidth}px`, flexShrink: 0, flexGrow: 0 }}>
        <div className="design-header-tools">
          <div className="design-session-info">
            {session?.title}
            {session && session.id !== 'fallback-id' && (
              <span style={{ fontSize: 10, color: '#666', marginLeft: 8 }}>
                ({statusLabels[session.status] || session.status})
              </span>
            )}
          </div>
          <div style={{ display: 'flex', gap: '6px', marginLeft: 'auto' }}>
            <button
              className={`aiterm-block-btn${historyOpen ? ' aiterm-agent-toggle--on' : ''}`}
              title={t.design_history}
              onClick={() => { setHistoryOpen((o) => !o); if (!historyOpen) refreshSessionList(); }}
              style={{ padding: "2px 8px", fontSize: 11, background: historyOpen ? "rgba(96, 165, 250, 0.15)" : "transparent", color: historyOpen ? "#60a5fa" : "#666", border: historyOpen ? "1px solid rgba(96, 165, 250, 0.3)" : "1px solid #333", borderRadius: 4, cursor: "pointer" }}
            >
              {t.design_history}
            </button>
            <button
              className="aiterm-block-btn"
              title={t.design_new_session_tooltip}
              onClick={handleNewSession}
              disabled={isStreaming}
              style={{ padding: "2px 8px", fontSize: 11, background: "transparent", color: "#666", border: "1px solid #333", borderRadius: 4, cursor: "pointer" }}
            >
              {t.design_new_session}
            </button>
            <button
              className={`aiterm-block-btn ${isRemoteEnabled ? 'aiterm-agent-toggle--on' : ''}`}
              title={t.design_remote}
              onClick={() => setIsRemoteEnabled(!isRemoteEnabled)}
              style={{ padding: "2px 8px", fontSize: 11, background: isRemoteEnabled ? "rgba(52, 211, 153, 0.15)" : "transparent", color: isRemoteEnabled ? "#34d399" : "#666", border: isRemoteEnabled ? "1px solid rgba(52, 211, 153, 0.3)" : "1px solid #333", borderRadius: 4, cursor: "pointer" }}
            >
              {t.design_remote}
            </button>
          </div>
        </div>

        {/* History side panel */}
        {historyOpen && (
          <div style={{ background: '#111', borderBottom: '1px solid #333', maxHeight: '200px', overflowY: 'auto', padding: '8px' }}>
            {sessionList.length === 0 && (
              <div style={{ color: '#666', fontSize: 12, textAlign: 'center', padding: '12px' }}>{t.design_no_history}</div>
            )}
            {sessionList.map((s) => (
              <div
                key={s.id}
                style={{
                  display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                  padding: '6px 8px', borderRadius: 4, cursor: 'pointer',
                  background: s.id === session?.id ? 'rgba(96, 165, 250, 0.1)' : 'transparent',
                  borderLeft: s.id === session?.id ? '2px solid #60a5fa' : '2px solid transparent',
                }}
                onClick={() => handleLoadSession(s)}
              >
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 12, color: '#ccc', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {s.title}
                  </div>
                  <div style={{ fontSize: 10, color: '#666' }}>
                    {statusLabels[s.status] || s.status}
                  </div>
                </div>
                <button
                  style={{ background: 'none', border: 'none', color: '#666', cursor: 'pointer', fontSize: 14, padding: '0 4px', flexShrink: 0 }}
                  title={t.design_delete_tooltip}
                  onClick={(e) => { e.stopPropagation(); handleDeleteSession(s.id); }}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        )}

        <div className="design-interaction-area">
          <div ref={messagesListRef} className="design-messages-list">
            {messages.length === 0 && (
              <div className="design-welcome-card">
                <div className="design-welcome-icon-wrapper">
                  <div className="design-welcome-glow" />
                  <svg className="design-welcome-logo" width="44" height="44" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
                    <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
                    <line x1="12" y1="22.08" x2="12" y2="12" />
                  </svg>
                </div>
                <h3>{t.design_welcome_title}</h3>
                <p className="design-welcome-desc">{t.design_welcome_hint}</p>
                
                <div className="design-welcome-features">
                  <div className="design-feature-item">
                    <div className="design-feature-icon">💬</div>
                    <div className="design-feature-text">
                      <h5>{t.design_feature_1_title}</h5>
                      <p>{t.design_feature_1_desc}</p>
                    </div>
                  </div>
                  <div className="design-feature-item">
                    <div className="design-feature-icon">🪄</div>
                    <div className="design-feature-text">
                      <h5>{t.design_feature_2_title}</h5>
                      <p>{t.design_feature_2_desc}</p>
                    </div>
                  </div>
                  <div className="design-feature-item">
                    <div className="design-feature-icon">💾</div>
                    <div className="design-feature-text">
                      <h5>{t.design_feature_3_title}</h5>
                      <p>{t.design_feature_3_desc}</p>
                    </div>
                  </div>
                </div>
              </div>
            )}
            {messages.map((m, i) => {
              const isLastAssistant = m.role === 'assistant' && i === messages.length - 1;
              return (
                <MessageBubble
                  key={i}
                  role={m.role as 'user' | 'assistant'}
                  content={cleanMessageForDisplay(contentToString(m.content), isLastAssistant && isStreaming)}
                  onExecuteCommand={() => {}}
                />
              );
            })}
            {isStreaming && (messages.length === 0 || messages[messages.length - 1].role !== 'assistant') && (
              <div className="design-thinking-bubble">
                <span /><span /><span />
              </div>
            )}
          </div>

          <div className="design-input-section">
            <div className="design-tool-row" style={{ padding: '8px 16px 0 16px' }}>
              <button
                className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                style={{ display: "flex", alignItems: "center", gap: 6 }}
                onClick={() => setShowProviderPalette(true)}
              >
                <RobotIcon size={13} style={{ color: "var(--accent, #a855f7)", flexShrink: 0 }} />
                <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 200 }}>
                  {providerName ? t.design_model(providerName) : t.design_default_model}
                </span>
              </button>
            </div>
            <div className="design-input-container">
              <textarea
                className="design-chat-input" placeholder={t.design_input_placeholder}
                value={inputValue} onChange={(e) => setInputValue(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    const shouldSubmit =
                      (submitShortcut === 'enter' && !e.shiftKey && !e.ctrlKey && !e.metaKey) ||
                      (submitShortcut === 'shift-enter' && e.shiftKey && !e.ctrlKey) ||
                      (submitShortcut === 'ctrl-enter' && (e.ctrlKey || e.metaKey) && !e.shiftKey);
                    if (shouldSubmit) { e.preventDefault(); handleSendMessage(); }
                  }
                }}
                disabled={isStreaming}
              />
              <button className="design-send-btn aiterm-btn aiterm-btn--primary aiterm-btn--icon" onClick={() => handleSendMessage()} disabled={!inputValue.trim() || isStreaming} title={t.design_send}>
                ▲
              </button>
            </div>
          </div>
        </div>
      </div>

      <div className="design-resizer" onMouseDown={(e) => { e.preventDefault(); setIsResizing(true); }} />

      <div className="design-right-panel" style={{ flex: 1 }}>
        <SpecPreview
          title={session?.title}
          proposal={session?.current_proposal_draft || null}
          spec={session?.current_spec_draft || null}
          sdd={session?.current_sdd_draft || null}
          plan={session?.current_plan_draft || null}
          onGenerate={handleGenerate}
          isStreaming={isStreaming}
        />
      </div>

      {showProviderPalette && (
        <ProviderPalette
          onClose={() => setShowProviderPalette(false)}
          onSwitch={(p) => {
            setProviderId(p.id);
            setProviderName(p.display_name);
            setShowProviderPalette(false);
          }}
        />
      )}
    </div>
  );
}

