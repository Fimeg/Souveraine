export const SOUVERAINE_URL = process.env.NEXT_PUBLIC_SOUVERAINE_URL || 'http://localhost:8484';

export const MESSAGE_TYPES = {
  ASSISTANT: 'assistant_message',
  REASONING: 'reasoning_message',
  TOOL_CALL: 'tool_call_message',
  TOOL_RETURN: 'tool_return_message',
  SURFACING: 'souveraine_surfacing',
  REFLECTION: 'souveraine_reflection',
  ARCHIVIST: 'souveraine_archivist',
  PING: 'ping',
};

export const CONNECTION_STATUS = {
  CONNECTED: 'connected',
  CONNECTING: 'connecting',
  DISCONNECTED: 'disconnected',
  ERROR: 'error',
};
