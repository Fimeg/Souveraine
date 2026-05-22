'use client';
import { useState } from 'react';
import { useSouveraine } from '@/hooks/useSouveraine';
import { useChat } from '@/hooks/useChat';
import Header from '@/components/Layout/Header';
import Sidebar from '@/components/Layout/Sidebar';
import StatusBar from '@/components/Layout/StatusBar';
import ChatContainer from '@/components/Chat/ChatContainer';
import WelcomeScreen from '@/components/WelcomeScreen';

/**
 * Ani Studio — Main Application Page
 *
 * Orchestrates the full UI: sidebar, header, chat area, and status bar.
 * Connects to the Souveraine server via REST + SSE + WebSocket.
 */
export default function Home() {
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);

  const {
    agents,
    selectedAgent,
    setSelectedAgent,
    connectionStatus,
    firehoseEvents,
    connect,
    refreshAgents,
  } = useSouveraine();

  const {
    messages,
    isStreaming,
    error,
    subconsciousEvents,
    send,
    clearChat,
  } = useChat(selectedAgent?.id);

  const isConnected = connectionStatus === 'connected';

  const handleSelectAgent = (agent) => {
    setSelectedAgent(agent);
    clearChat();
  };

  const handleNewChat = () => {
    clearChat();
  };

  return (
    <div className="app-shell">
      <Header
        agent={selectedAgent}
        connectionStatus={connectionStatus}
        onRefresh={connect}
      />

      <div className="app-content">
        <Sidebar
          agents={agents}
          selectedAgent={selectedAgent}
          onSelectAgent={handleSelectAgent}
          onNewChat={handleNewChat}
          isCollapsed={sidebarCollapsed}
          onToggleCollapse={() => setSidebarCollapsed(s => !s)}
        />

        {isConnected && selectedAgent ? (
          <ChatContainer
            messages={messages}
            isStreaming={isStreaming}
            onSend={send}
            error={error}
            subconsciousEvents={subconsciousEvents}
          />
        ) : (
          <WelcomeScreen
            connectionStatus={connectionStatus}
            onRetry={connect}
          />
        )}
      </div>

      <StatusBar
        agent={selectedAgent}
        connectionStatus={connectionStatus}
        firehoseEvents={firehoseEvents}
      />
    </div>
  );
}
