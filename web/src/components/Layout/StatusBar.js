'use client';

export default function StatusBar({ agent, connectionStatus, firehoseEvents }) {
  const lastEvent = firehoseEvents?.[firehoseEvents.length - 1];

  return (
    <footer className="status-bar" id="status-bar">
      <div className="status-bar-left">
        <span className="status-item">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2"/>
          </svg>
          {connectionStatus === 'connected' ? 'Online' : 'Offline'}
        </span>
        {agent?.llm_config && (
          <span className="status-item status-model">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
            </svg>
            {agent.llm_config.model}
          </span>
        )}
      </div>

      <div className="status-bar-right">
        {lastEvent && (
          <span className="status-item status-event">
            Last event: {lastEvent.sensor_name || 'system'}
          </span>
        )}
        <span className="status-item status-version">Souveraine α</span>
      </div>
    </footer>
  );
}
