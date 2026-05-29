'use client';

export default function Header({ agent, connectionStatus, onRefresh }) {
  const statusColors = {
    connected: 'var(--color-success)',
    connecting: 'var(--color-warning)',
    disconnected: 'var(--color-muted)',
    error: 'var(--color-error)',
  };

  return (
    <header className="header" id="app-header">
      <div className="header-left">
        <div className="header-logo">
          <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="var(--color-accent)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>
            <path d="M20 3v4"/>
            <path d="M22 5h-4"/>
          </svg>
          <h1 className="header-title">Souveraine Studio</h1>
        </div>
        {agent && (
          <div className="header-agent-info">
            <span className="header-divider">/</span>
            <span className="header-agent-name">{agent.name}</span>
            {agent.llm_config && (
              <span className="header-model-badge">{agent.llm_config.model}</span>
            )}
          </div>
        )}
      </div>

      <div className="header-right">
        <button
          className="header-action-btn"
          onClick={onRefresh}
          title="Refresh connection"
          id="refresh-btn"
          aria-label="Refresh connection"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/>
            <path d="M3 3v5h5"/>
            <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16"/>
            <path d="M16 16h5v5"/>
          </svg>
        </button>

        <div className="connection-indicator" id="connection-status">
          <div
            className="connection-dot"
            style={{ backgroundColor: statusColors[connectionStatus] || statusColors.disconnected }}
          />
          <span className="connection-label">{connectionStatus}</span>
        </div>
      </div>
    </header>
  );
}
