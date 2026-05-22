'use client';
import { useState } from 'react';
import MessageList from './MessageList';
import InputBar from './InputBar';
import SubconsciousPanel from './SubconsciousPanel';
import TokenVisualizer from './TokenVisualizer';

export default function ChatContainer({ messages, isStreaming, onSend, error, subconsciousEvents, conversationId }) {
  const [showSubconscious, setShowSubconscious] = useState(false);

  return (
    <main className="chat-container" id="chat-container">
      <TokenVisualizer conversationId={conversationId} messagesCount={messages.length} />
      {error && (
        <div className="chat-error" role="alert" id="chat-error">
          <span className="chat-error-icon">⚠</span>
          <span>{error}</span>
        </div>
      )}

      <div className="chat-body">
        <div className="chat-messages-area">
          <MessageList messages={messages} isStreaming={isStreaming} />
        </div>

        {showSubconscious && subconsciousEvents.length > 0 && (
          <aside className="subconscious-sidebar" id="subconscious-panel">
            <SubconsciousPanel events={subconsciousEvents} onClose={() => setShowSubconscious(false)} />
          </aside>
        )}
      </div>

      <InputBar
        onSend={onSend}
        isStreaming={isStreaming}
        onToggleSubconscious={() => setShowSubconscious(s => !s)}
        hasSubconsciousEvents={subconsciousEvents.length > 0}
      />
    </main>
  );
}
