'use client';

export default function Sidebar({
  agents,
  selectedAgent,
  onSelectAgent,
  onNewChat,
  isCollapsed,
  onToggleCollapse,
  activeTab = 'chat',
  onSelectTab,
}) {
  return (
    <nav className={`sidebar ${isCollapsed ? 'sidebar-collapsed' : ''}`} id="sidebar" aria-label="Main navigation">
      {/* Toggle button */}
      <button
        className="sidebar-toggle"
        onClick={onToggleCollapse}
        id="sidebar-toggle"
        aria-label={isCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          {isCollapsed ? (
            <>
              <rect width="18" height="18" x="3" y="3" rx="2"/>
              <path d="M9 3v18"/>
              <path d="m14 9 3 3-3 3"/>
            </>
          ) : (
            <>
              <rect width="18" height="18" x="3" y="3" rx="2"/>
              <path d="M9 3v18"/>
              <path d="m16 15-3-3 3-3"/>
            </>
          )}
        </svg>
      </button>

      {!isCollapsed && (
        <>
          {/* Navigation Tabs */}
          <div className="sidebar-section nav-tabs-section">
            <div className="sidebar-nav-tabs">
              <button
                className={`sidebar-nav-tab ${activeTab === 'chat' ? 'active' : ''}`}
                onClick={() => onSelectTab?.('chat')}
                id="nav-tab-chat"
              >
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
                </svg>
                <span>Chat Studio</span>
              </button>

              <button
                className={`sidebar-nav-tab ${activeTab === 'memory' ? 'active' : ''}`}
                onClick={() => onSelectTab?.('memory')}
                id="nav-tab-memory"
              >
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2z"/>
                  <path d="M2 10h20"/>
                </svg>
                <span>Memory Space</span>
              </button>

              <button
                className={`sidebar-nav-tab ${activeTab === 'consciousness' ? 'active' : ''}`}
                onClick={() => onSelectTab?.('consciousness')}
                id="nav-tab-consciousness"
              >
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M22 12h-4l-3 9L9 3l-3 9H2"/>
                </svg>
                <span>Consciousness Hub</span>
              </button>

              <button
                className={`sidebar-nav-tab ${activeTab === 'settings' ? 'active' : ''}`}
                onClick={() => onSelectTab?.('settings')}
                id="nav-tab-settings"
              >
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <line x1="4" y1="21" x2="4" y2="14"/>
                  <line x1="4" y1="10" x2="4" y2="3"/>
                  <line x1="12" y1="21" x2="12" y2="12"/>
                  <line x1="12" y1="8" x2="12" y2="3"/>
                  <line x1="20" y1="21" x2="20" y2="16"/>
                  <line x1="20" y1="12" x2="20" y2="3"/>
                  <line x1="2" y1="14" x2="6" y2="14"/>
                  <line x1="10" y1="8" x2="14" y2="8"/>
                  <line x1="18" y1="16" x2="22" y2="16"/>
                </svg>
                <span>Cognitive Settings</span>
              </button>
            </div>
          </div>

          {activeTab === 'chat' && (
            <button className="sidebar-new-chat" onClick={onNewChat} id="new-chat-btn">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M12 5v14"/>
                <path d="M5 12h14"/>
              </svg>
              <span>New Conversation</span>
            </button>
          )}

          {/* Agent Selector */}
          <div className="sidebar-section">
            <h3 className="sidebar-section-title">Agents</h3>
            <div className="sidebar-agent-list">
              {agents.map((agent) => (
                <button
                  key={agent.id}
                  className={`sidebar-agent-item ${selectedAgent?.id === agent.id ? 'active' : ''}`}
                  onClick={() => onSelectAgent(agent)}
                  id={`agent-${agent.id}`}
                >
                  <div className="sidebar-agent-avatar">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>
                    </svg>
                  </div>
                  <div className="sidebar-agent-info">
                    <span className="sidebar-agent-name">{agent.name}</span>
                    {agent.description && (
                      <span className="sidebar-agent-desc">{agent.description}</span>
                    )}
                  </div>
                </button>
              ))}

              {agents.length === 0 && (
                <div className="sidebar-empty">
                  <p>No agents found</p>
                  <p className="sidebar-empty-hint">Start Souveraine server first</p>
                </div>
              )}
            </div>
          </div>
        </>
      )}
    </nav>
  );
}
