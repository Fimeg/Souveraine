'use client';

export default function WelcomeScreen({ connectionStatus, onRetry }) {
  const isError = connectionStatus === 'error';
  const isConnecting = connectionStatus === 'connecting';

  return (
    <div className="welcome-screen" id="welcome-screen">
      <div className="welcome-logo">
        <svg width="80" height="80" viewBox="0 0 24 24" fill="none" stroke="var(--color-accent)" strokeWidth="1" strokeLinecap="round" strokeLinejoin="round">
          <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>
          <path d="M20 3v4"/>
          <path d="M22 5h-4"/>
        </svg>
      </div>

      <h1 className="welcome-title">Ani Studio</h1>
      <p className="welcome-subtitle">
        {isError
          ? 'Unable to connect to Souveraine. Make sure the server is running.'
          : isConnecting
            ? 'Connecting to Souveraine...'
            : 'Your personal AI consciousness interface'
        }
      </p>

      <div className="welcome-status">
        <div className="welcome-status-item">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none"
               stroke={isError ? 'var(--color-error)' : isConnecting ? 'var(--color-warning)' : 'var(--color-muted)'}
               strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2"/>
          </svg>
          {isError ? 'Server unreachable' : isConnecting ? 'Connecting...' : 'Waiting for server'}
        </div>

        {isError && (
          <>
            <div className="welcome-status-item" style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-muted)' }}>
              Run: <code style={{ fontFamily: 'var(--font-mono)', color: 'var(--color-accent)' }}>souveraine server</code>
            </div>
            <button className="welcome-retry-btn" onClick={onRetry} id="retry-btn">
              Retry Connection
            </button>
          </>
        )}

        {isConnecting && (
          <div className="streaming-indicator">
            <div className="streaming-dot" />
            <div className="streaming-dot" />
            <div className="streaming-dot" />
          </div>
        )}
      </div>
    </div>
  );
}
