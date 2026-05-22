'use client';

export default function SubconsciousPanel({ events, onClose }) {
  const getEventIcon = (source) => {
    switch (source) {
      case 'subconscious': return '🧠';
      case 'reflection': return '🔮';
      case 'archivist': return '📚';
      default: return '💭';
    }
  };

  const getEventClass = (source) => {
    switch (source) {
      case 'subconscious': return 'event-subconscious';
      case 'reflection': return 'event-reflection';
      case 'archivist': return 'event-archivist';
      default: return 'event-surfacing';
    }
  };

  return (
    <div className="subconscious-panel">
      <div className="subconscious-header">
        <h3 className="subconscious-title">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="12" r="10"/>
            <path d="M12 16v-4"/>
            <path d="M12 8h.01"/>
          </svg>
          Inner Voice
        </h3>
        <button className="subconscious-close" onClick={onClose} id="close-subconscious" aria-label="Close panel">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M18 6 6 18"/>
            <path d="m6 6 12 12"/>
          </svg>
        </button>
      </div>

      <div className="subconscious-events">
        {[...events].reverse().map((event, i) => (
          <div key={i} className={`subconscious-event ${getEventClass(event.source)}`}>
            <div className="event-header">
              <span className="event-icon">{getEventIcon(event.source)}</span>
              <span className="event-source">{event.source}</span>
              {event.priority && <span className="event-priority">{event.priority}</span>}
              <span className="event-time">
                {event.timestamp?.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
              </span>
            </div>
            <p className="event-content">{event.content}</p>
            {event.pressure !== undefined && (
              <div className="event-pressure">
                <div className="pressure-bar">
                  <div className="pressure-fill" style={{ width: `${event.pressure * 100}%` }} />
                </div>
                <span className="pressure-label">{(event.pressure * 100).toFixed(0)}% pressure</span>
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
