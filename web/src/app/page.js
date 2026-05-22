'use client';
import { useState } from 'react';
import { useSouveraine } from '@/hooks/useSouveraine';
import { useChat } from '@/hooks/useChat';
import Header from '@/components/Layout/Header';
import Sidebar from '@/components/Layout/Sidebar';
import StatusBar from '@/components/Layout/StatusBar';
import ChatContainer from '@/components/Chat/ChatContainer';
import WelcomeScreen from '@/components/WelcomeScreen';
import MemoryBrowser from '@/components/Memory/MemoryBrowser';
import ConsciousnessHub from '@/components/Consciousness/ConsciousnessHub';
import SettingsPanel from '@/components/Settings/SettingsPanel';

/**
 * Ani Studio — Main Application Page
 *
 * Orchestrates the full UI: sidebar, header, chat area, and status bar.
 * Connects to the Souveraine server via REST + SSE + WebSocket.
 */
export default function Home() {
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [activeTab, setActiveTab] = useState('chat');

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
    conversationId,
  } = useChat(selectedAgent?.id);

  const isConnected = connectionStatus === 'connected';

  const handleSelectAgent = (agent) => {
    setSelectedAgent(agent);
    clearChat();
  };

  const handleNewChat = () => {
    clearChat();
  };

  const renderActiveContent = () => {
    if (!isConnected || !selectedAgent) {
      return (
        <WelcomeScreen
          connectionStatus={connectionStatus}
          onRetry={connect}
        />
      );
    }

    switch (activeTab) {
      case 'memory':
        return <MemoryBrowser agent={selectedAgent} />;
      case 'consciousness':
        return (
          <ConsciousnessHub
            firehoseEvents={firehoseEvents}
            agent={selectedAgent}
          />
        );
      case 'settings':
        return <SettingsPanel agent={selectedAgent} />;
      case 'chat':
      default:
        return (
          <ChatContainer
            messages={messages}
            isStreaming={isStreaming}
            onSend={send}
            error={error}
            subconsciousEvents={subconsciousEvents}
            conversationId={conversationId}
          />
        );
    }
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
          activeTab={activeTab}
          onSelectTab={setActiveTab}
        />

        {renderActiveContent()}
      </div>

      <StatusBar
        agent={selectedAgent}
        connectionStatus={connectionStatus}
        firehoseEvents={firehoseEvents}
      />
    </div>
  );
}

