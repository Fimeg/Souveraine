'use client';
import { useState } from 'react';
import ReactMarkdown from 'react-markdown';
import rehypeHighlight from 'rehype-highlight';
import remarkGfm from 'remark-gfm';

export default function MessageBubble({ message }) {
  const [showReasoning, setShowReasoning] = useState(false);
  const [expandedTools, setExpandedTools] = useState({});
  const isUser = message.role === 'user';

  const toggleTool = (index) => {
    setExpandedTools(prev => ({ ...prev, [index]: !prev[index] }));
  };

  return (
    <div className={`message-bubble ${isUser ? 'message-user' : 'message-assistant'}`}
         id={`message-${message.id}`}>
      {/* Avatar */}
      <div className="message-avatar">
        <div className={`avatar ${isUser ? 'avatar-user' : 'avatar-assistant'}`}>
          {isUser ? (
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/>
              <circle cx="12" cy="7" r="4"/>
            </svg>
          ) : (
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>
            </svg>
          )}
        </div>
      </div>

      {/* Content */}
      <div className="message-content">
        <div className="message-header">
          <span className="message-sender">{isUser ? 'You' : 'Ani'}</span>
          {message.timestamp && (
            <span className="message-time">
              {new Date(message.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
            </span>
          )}
        </div>

        {/* Reasoning (collapsible) */}
        {message.reasoning && (
          <button
            className="reasoning-toggle"
            onClick={() => setShowReasoning(!showReasoning)}
            id={`reasoning-toggle-${message.id}`}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/>
              <path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/>
            </svg>
            {showReasoning ? 'Hide' : 'Show'} reasoning
          </button>
        )}
        {showReasoning && message.reasoning && (
          <div className="reasoning-block">
            <ReactMarkdown rehypePlugins={[rehypeHighlight]} remarkPlugins={[remarkGfm]}>
              {message.reasoning}
            </ReactMarkdown>
          </div>
        )}

        {/* Tool calls */}
        {message.toolCalls && message.toolCalls.length > 0 && (
          <div className="tool-calls">
            {message.toolCalls.map((tc, i) => (
              <div key={i} className="tool-call-card">
                <button
                  className="tool-call-header"
                  onClick={() => toggleTool(i)}
                  id={`tool-call-${message.id}-${i}`}
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>
                  </svg>
                  <span className="tool-name">{tc.function?.name || 'tool'}</span>
                  <svg className={`tool-chevron ${expandedTools[i] ? 'expanded' : ''}`} width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="m6 9 6 6 6-6"/>
                  </svg>
                </button>
                {expandedTools[i] && (
                  <div className="tool-call-body">
                    <pre className="tool-call-args">
                      {(() => {
                        try {
                          return JSON.stringify(JSON.parse(tc.function?.arguments || '{}'), null, 2);
                        } catch {
                          return tc.function?.arguments || '';
                        }
                      })()}
                    </pre>
                    {message.toolReturns && message.toolReturns[i] && (
                      <div className={`tool-return ${message.toolReturns[i].status === 'error' ? 'tool-return-error' : 'tool-return-success'}`}>
                        <span className="tool-return-status">
                          {message.toolReturns[i].status === 'error' ? '✗' : '✓'} {message.toolReturns[i].status}
                        </span>
                        <pre className="tool-return-output">{message.toolReturns[i].output}</pre>
                      </div>
                    )}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}

        {/* Main content */}
        {message.content && (
          <div className="message-text">
            {isUser ? (
              <p>{message.content}</p>
            ) : (
              <ReactMarkdown rehypePlugins={[rehypeHighlight]} remarkPlugins={[remarkGfm]}>
                {message.content}
              </ReactMarkdown>
            )}
          </div>
        )}

        {/* Streaming indicator */}
        {message.isStreaming && !message.content && (
          <div className="streaming-indicator" id="streaming-indicator">
            <div className="streaming-dot" />
            <div className="streaming-dot" />
            <div className="streaming-dot" />
          </div>
        )}
      </div>
    </div>
  );
}
