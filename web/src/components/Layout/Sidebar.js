'use client';

export default function Sidebar({ agents, selectedAgent, onSelectAgent, onNewChat, isCollapsed, onToggleCollapse }) {
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
          {/* New Chat */}
          <button className="sidebar-new-chat" onClick={onNewChat} id="new-chat-btn">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 5v14"/>
              <path d="M5 12h14"/>
            </svg>
            <span>New Conversation</span>
          </button>

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
