const SOUVERAINE_URL = process.env.NEXT_PUBLIC_SOUVERAINE_URL || 'http://localhost:8484';

export async function fetchAgents() {
  const res = await fetch(`${SOUVERAINE_URL}/v1/agents`);
  if (!res.ok) throw new Error(`Failed to fetch agents: ${res.statusText}`);
  return res.json();
}

export async function fetchAgent(id) {
  const res = await fetch(`${SOUVERAINE_URL}/v1/agents/${id}`);
  if (!res.ok) throw new Error(`Agent not found: ${res.statusText}`);
  return res.json();
}

export async function createConversation(agentId) {
  const res = await fetch(`${SOUVERAINE_URL}/v1/conversations`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ agent_id: agentId }),
  });
  if (!res.ok) throw new Error(`Failed to create conversation: ${res.statusText}`);
  return res.json();
}

export async function fetchConversations(agentId) {
  const res = await fetch(`${SOUVERAINE_URL}/v1/conversations?agent_id=${agentId}`);
  if (!res.ok) throw new Error(`Failed to fetch conversations: ${res.statusText}`);
  return res.json();
}

/**
 * Send a message to a conversation and process the SSE stream.
 * Souveraine streams responses as Server-Sent Events with event types:
 * - message (assistant_message): streaming text content
 * - reasoning (reasoning_message): model reasoning/thinking
 * - tool_call: tool invocation
 * - tool_return: tool result
 * - souveraine_surfacing: subconscious surfacing events
 * - souveraine_reflection: N+25 reflection events
 * - souveraine_archivist: N+100 archivist events
 * - ping: keep-alive
 */
export async function sendMessage(conversationId, messages, onEvent) {
  const res = await fetch(`${SOUVERAINE_URL}/v1/conversations/${conversationId}/messages`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ messages, stream: true }),
  });

  if (!res.ok) throw new Error(`Message failed: ${res.statusText}`);

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;

    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split('\n');
    buffer = lines.pop() || '';

    for (const line of lines) {
      if (line.startsWith('data: ')) {
        try {
          const data = JSON.parse(line.slice(6));
          onEvent(data);
        } catch (e) {
          // Skip malformed SSE data
        }
      }
    }
  }
}

export async function fetchMemoryList(agentId, prefix) {
  const params = prefix ? `?prefix=${encodeURIComponent(prefix)}` : '';
  const res = await fetch(`${SOUVERAINE_URL}/v1/agents/${agentId}/memory${params}`);
  if (!res.ok) throw new Error(`Failed to list memory: ${res.statusText}`);
  return res.json();
}

export async function fetchMemoryFile(agentId, path) {
  const res = await fetch(`${SOUVERAINE_URL}/v1/agents/${agentId}/memory/${path}`);
  if (!res.ok) throw new Error(`Failed to read memory: ${res.statusText}`);
  return res.json();
}

export async function checkHealth() {
  try {
    const res = await fetch(`${SOUVERAINE_URL}/health`);
    return res.ok;
  } catch {
    return false;
  }
}

/**
 * Connect to the Souveraine event firehose WebSocket.
 * Streams every SensorEvent from the nervous system in real time.
 */
export function createFirehoseConnection(onEvent, onError) {
  const wsUrl = SOUVERAINE_URL.replace(/^http/, 'ws') + '/v1/firehose';
  const ws = new WebSocket(wsUrl);

  ws.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data);
      onEvent(data);
    } catch (e) {
      // Skip malformed events
    }
  };

  ws.onerror = (e) => onError?.(e);
  ws.onclose = () => onError?.(new Error('WebSocket closed'));

  return ws;
}
