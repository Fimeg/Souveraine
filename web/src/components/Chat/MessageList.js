'use client';
import { useEffect, useRef } from 'react';
import MessageBubble from './MessageBubble';

export default function MessageList({ messages, isStreaming }) {
  const bottomRef = useRef(null);
  const containerRef = useRef(null);

  useEffect(() => {
    if (bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' });
    }
  }, [messages, isStreaming]);

  if (messages.length === 0) {
    return (
      <div className="message-list-empty" id="empty-chat">
        <div className="empty-chat-icon">
          <svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="round" strokeLinejoin="round">
            <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>
            <path d="M20 3v4"/>
            <path d="M22 5h-4"/>
          </svg>
        </div>
        <h2 className="empty-chat-title">Begin a conversation</h2>
        <p className="empty-chat-subtitle">Ani is ready. Speak, and the world listens.</p>
      </div>
    );
  }

  return (
    <div className="message-list" ref={containerRef} id="message-list">
      {messages.map((msg) => (
        <MessageBubble key={msg.id} message={msg} />
      ))}
      <div ref={bottomRef} className="scroll-anchor" />
    </div>
  );
}
