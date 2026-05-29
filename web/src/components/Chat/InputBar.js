'use client';
import { useState, useRef, useEffect } from 'react';

export default function InputBar({ onSend, isStreaming, onToggleSubconscious, hasSubconsciousEvents }) {
  const [input, setInput] = useState('');
  const textareaRef = useRef(null);

  // Auto-resize textarea as content grows
  useEffect(() => {
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
      textareaRef.current.style.height = Math.min(textareaRef.current.scrollHeight, 200) + 'px';
    }
  }, [input]);

  const handleSubmit = (e) => {
    e?.preventDefault();
    if (!input.trim() || isStreaming) return;
    onSend(input.trim());
    setInput('');
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  };

  const handleKeyDown = (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  return (
    <form className="input-bar" onSubmit={handleSubmit} id="input-bar">
      <div className="input-bar-inner">
        {hasSubconsciousEvents && (
          <button
            type="button"
            className="input-action-btn subconscious-toggle-btn"
            onClick={onToggleSubconscious}
            title="Toggle subconscious panel"
            id="toggle-subconscious-btn"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <circle cx="12" cy="12" r="10"/>
              <path d="M12 16v-4"/>
              <path d="M12 8h.01"/>
            </svg>
            <span className="subconscious-badge" />
          </button>
        )}

        <textarea
          ref={textareaRef}
          className="input-textarea"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={isStreaming ? 'Souveraine is thinking...' : 'Speak to Souveraine...'}
          disabled={isStreaming}
          rows={1}
          id="message-input"
          aria-label="Message input"
        />

        <button
          type="submit"
          className="send-btn"
          disabled={!input.trim() || isStreaming}
          id="send-btn"
          aria-label="Send message"
        >
          {isStreaming ? (
            <div className="send-btn-loading">
              <div className="spinner" />
            </div>
          ) : (
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="m22 2-7 20-4-9-9-4Z"/>
              <path d="M22 2 11 13"/>
            </svg>
          )}
        </button>
      </div>
    </form>
  );
}
