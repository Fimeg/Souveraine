'use client';
import { useState, useCallback, useRef } from 'react';
import { sendMessage, createConversation } from '@/lib/api';
import { MESSAGE_TYPES } from '@/lib/constants';

/**
 * Chat state management hook.
 * Handles message history, SSE streaming, conversation lifecycle,
 * and subconscious/reflection/archivist event accumulation.
 */
export function useChat(agentId) {
  const [messages, setMessages] = useState([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [conversationId, setConversationId] = useState(null);
  const [error, setError] = useState(null);
  const [subconsciousEvents, setSubconsciousEvents] = useState([]);
  const streamingContentRef = useRef('');

  const ensureConversation = useCallback(async () => {
    if (conversationId) return conversationId;
    try {
      const conv = await createConversation(agentId);
      setConversationId(conv.id);
      return conv.id;
    } catch (e) {
      setError(`Failed to create conversation: ${e.message}`);
      throw e;
    }
  }, [agentId, conversationId]);

  const send = useCallback(async (content) => {
    if (!content.trim() || isStreaming) return;
    setError(null);

    // Add user message
    const userMsg = { id: Date.now(), role: 'user', content, timestamp: new Date() };
    setMessages(prev => [...prev, userMsg]);

    // Create assistant placeholder
    const assistantId = Date.now() + 1;
    const assistantMsg = {
      id: assistantId,
      role: 'assistant',
      content: '',
      reasoning: '',
      toolCalls: [],
      toolReturns: [],
      timestamp: new Date(),
      isStreaming: true,
    };
    setMessages(prev => [...prev, assistantMsg]);
    setIsStreaming(true);
    streamingContentRef.current = '';

    try {
      const convId = await ensureConversation();
      await sendMessage(
        convId,
        [{ role: 'user', content }],
        (event) => {
          switch (event.message_type) {
            case MESSAGE_TYPES.ASSISTANT:
              streamingContentRef.current += event.content;
              setMessages(prev =>
                prev.map(m =>
                  m.id === assistantId
                    ? { ...m, content: streamingContentRef.current }
                    : m
                )
              );
              break;
            case MESSAGE_TYPES.REASONING:
              setMessages(prev =>
                prev.map(m =>
                  m.id === assistantId
                    ? { ...m, reasoning: (m.reasoning || '') + event.content }
                    : m
                )
              );
              break;
            case 'tool_call':
              setMessages(prev =>
                prev.map(m =>
                  m.id === assistantId
                    ? { ...m, toolCalls: [...(m.toolCalls || []), event.tool_call] }
                    : m
                )
              );
              break;
            case 'tool_return':
              setMessages(prev =>
                prev.map(m =>
                  m.id === assistantId
                    ? { ...m, toolReturns: [...(m.toolReturns || []), event.tool_return] }
                    : m
                )
              );
              break;
            case MESSAGE_TYPES.SURFACING:
              setSubconsciousEvents(prev => [...prev, {
                source: event.source,
                content: event.content,
                priority: event.priority,
                timestamp: new Date(),
              }]);
              break;
            case MESSAGE_TYPES.REFLECTION:
              setSubconsciousEvents(prev => [...prev, {
                source: 'reflection',
                content: event.content,
                timestamp: new Date(),
              }]);
              break;
            case MESSAGE_TYPES.ARCHIVIST:
              setSubconsciousEvents(prev => [...prev, {
                source: 'archivist',
                content: event.synthesis,
                pressure: event.pressure,
                timestamp: new Date(),
              }]);
              break;
          }
        }
      );
    } catch (e) {
      setError(e.message);
    } finally {
      setIsStreaming(false);
      setMessages(prev =>
        prev.map(m =>
          m.id === assistantId ? { ...m, isStreaming: false } : m
        )
      );
    }
  }, [agentId, isStreaming, ensureConversation]);

  const clearChat = useCallback(() => {
    setMessages([]);
    setConversationId(null);
    setSubconsciousEvents([]);
    setError(null);
  }, []);

  return {
    messages,
    isStreaming,
    conversationId,
    error,
    subconsciousEvents,
    send,
    clearChat,
  };
}
