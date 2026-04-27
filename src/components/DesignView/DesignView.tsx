// src/components/DesignView/DesignView.tsx
import { useState, useEffect, useRef, useCallback } from 'react';
import { SpecPreview } from './SpecPreview';
import { 
  designStartSession, 
  designChat, 
  designUpdateDraft, 
  designLoadSession, 
  designListMessages
} from '../../ipc/design';
import type { DesignSession } from '../../ipc/design';
import type { ChatMessage } from '../../ipc/ai';
import { listen } from '@tauri-apps/api/event';
import { useTelegramRemoteControl } from '../../hooks/useTelegramRemoteControl';
import { MessageBubble } from '../AiPanel/MessageBubble';
import { ProviderPalette } from '../ProviderPalette';
import { extractResponseText } from '../../lib/markdown';
import './DesignView.css';
export function DesignView({ isActive }: { isActive: boolean }) {
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

  const containerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  const { isRemoteEnabled, setIsRemoteEnabled, sendRemoteResponse } = useTelegramRemoteControl(
    session?.id || '',
    isActive,
    (text) => {
      handleSendMessage(text);
    }
  );

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
          setMessages((prev) => {
            const last = prev[prev.length - 1];
            if (last && last.role === 'assistant') {
              sendRemoteResponse(cleanMessageForDisplay(last.content));
            }
            return prev;
          });
        }
      }
    );
    return () => { unlisten.then((fn) => fn()); };
  }, [isActive, session, refreshSession]);

  // Initialize Session
  useEffect(() => {
    if (isActive && !session && !loading) {
      setLoading(true);
      designStartSession('新需求討論')
        .then(async (id) => {
          const data = await designLoadSession(id);
          setSession(data);
          await loadHistory(id);
        })
        .catch(() => {
          setSession({
            id: 'fallback-id', title: '後端未就緒', status: 'draft',
            current_spec_draft: '## 提示\n請重啟 `npm run tauri:dev` 以載入後端指令。',
            current_sdd_draft: null, current_plan_draft: null, context_summary: null
          });
        })
        .finally(() => setLoading(false));
    }
  }, [isActive, session, loading, loadHistory]);

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
      const response = await designChat(session.id, combinedMessages, providerId);
      const cleanResponseText = extractResponseText(response.content);

      const extractContent = (tag: string, text: string) => {
        const startTag = `[${tag}]`;
        const startIdx = text.indexOf(startTag);
        if (startIdx === -1) return null;

        let endIdx = text.length;
        const otherTags = ['[UPDATE_SPEC]', '[UPDATE_SDD]', '[UPDATE_PLAN]'].filter(t => t !== startTag);
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

        return content || null;
      };

      const specContent = extractContent('UPDATE_SPEC', cleanResponseText);
      const sddContent = extractContent('UPDATE_SDD', cleanResponseText);
      const planContent = extractContent('UPDATE_PLAN', cleanResponseText);

      if (specContent) await designUpdateDraft(session.id, 'spec', specContent);
      if (sddContent) await designUpdateDraft(session.id, 'sdd', sddContent);
      if (planContent) await designUpdateDraft(session.id, 'plan', planContent);
      if (specContent || sddContent || planContent) refreshSession();

    } catch (err) {
      let errMsg = String(err);
      if (err && typeof err === 'object') {
        errMsg = (err as any).reason || (err as any).message || JSON.stringify(err);
      }
      setMessages(prev => [...prev, { role: 'assistant', content: `❌ 錯誤: ${errMsg}` }]);
      setIsStreaming(false);
    }
  }, [inputValue, session, messages, isStreaming, refreshSession, providerId]);

  const cleanMessageForDisplay = (text: string) => {
    if (!text) return text;
    let cleaned = text;

    const tags = [
      { tag: '[UPDATE_SPEC]', msg: '> ✨ **已更新右側「規格 (Spec)」草稿**' },
      { tag: '[UPDATE_SDD]', msg: '> ✨ **已更新右側「系統設計 (SDD)」草稿**' },
      { tag: '[UPDATE_PLAN]', msg: '> ✨ **已更新右側「實作計畫 (Plan)」草稿**' },
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

  return (
    <div className="design-view-root" ref={containerRef}>
      <div className="design-left-panel" style={{ width: `${leftWidth}px`, flexBasis: `${leftWidth}px`, flexShrink: 0, flexGrow: 0 }}>
        <div className="design-header-tools">
          <div className="design-session-info">
            {session?.title}
          </div>
          <button
            className={`aiterm-block-btn ${isRemoteEnabled ? 'aiterm-agent-toggle--on' : ''}`}
            title="啟用/停用 Telegram 遠端控制"
            onClick={() => setIsRemoteEnabled(!isRemoteEnabled)}
            style={{ marginLeft: "auto", padding: "2px 8px", fontSize: 11, background: isRemoteEnabled ? "rgba(52, 211, 153, 0.15)" : "transparent", color: isRemoteEnabled ? "#34d399" : "#666", border: isRemoteEnabled ? "1px solid rgba(52, 211, 153, 0.3)" : "1px solid #333", borderRadius: 4, cursor: "pointer" }}
          >
            📱 Remote
          </button>
        </div>

        <div className="design-interaction-area">
          <div className="design-messages-list">
            {messages.length === 0 && (
              <div className="design-welcome-hero">
                <h3>👋 SDD 設計中心</h3>
                <p>請描述您想開發的需求...</p>
              </div>
            )}
            {messages.map((m, i) => (
              <MessageBubble 
                key={i} 
                role={m.role as 'user' | 'assistant'} 
                content={cleanMessageForDisplay(m.content)} 
                onExecuteCommand={() => {}}
              />
            ))}
            <div ref={messagesEndRef} />
          </div>

          <div className="design-input-section">
            <div className="design-tool-row" style={{ padding: '8px 16px 0 16px' }}>
              <button 
                className="design-provider-btn" 
                onClick={() => setShowProviderPalette(true)}
              >
                🤖 {providerName ? `模型: ${providerName}` : '預設模型'}
              </button>
            </div>
            <div className="design-input-container">
              <textarea
                className="design-chat-input" placeholder="輸入需求..."
                value={inputValue} onChange={(e) => setInputValue(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSendMessage(); } }}
                disabled={isStreaming}
              />
              <button className="design-send-btn" onClick={() => handleSendMessage()} disabled={!inputValue.trim() || isStreaming}>
                {isStreaming ? '...' : '送出'}
              </button>
            </div>
          </div>
        </div>
      </div>

      <div className="design-resizer" onMouseDown={(e) => { e.preventDefault(); setIsResizing(true); }} />

      <div className="design-right-panel" style={{ flex: 1 }}>
        <SpecPreview title={session?.title} spec={session?.current_spec_draft || null} sdd={session?.current_sdd_draft || null} plan={session?.current_plan_draft || null} />
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
