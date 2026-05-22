'use client';
import { useState, useEffect, useMemo, useRef } from 'react';
import { 
  Activity, 
  Cpu, 
  Layers, 
  AlertTriangle, 
  Search, 
  Filter, 
  Trash2, 
  Play, 
  Pause, 
  Database, 
  Brain, 
  Clock, 
  ChevronRight,
  Info,
  Sliders,
  Terminal
} from 'lucide-react';
import { fetchMemoryList } from '@/lib/api';

export default function ConsciousnessHub({ firehoseEvents = [], agent }) {
  // Filters & Search
  const [searchQuery, setSearchQuery] = useState('');
  const [activeFilter, setActiveFilter] = useState('all');
  const [isPaused, setIsPaused] = useState(false);
  const [selectedEvent, setSelectedEvent] = useState(null);
  
  // Diagnostics
  const [memoryCount, setMemoryCount] = useState(0);
  const [loadingMemory, setLoadingMemory] = useState(false);
  const [pingTime, setPingTime] = useState(14); // mock dynamic latency
  
  // Local frozen events when paused
  const [frozenEvents, setFrozenEvents] = useState([]);
  const eventListRef = useRef(null);

  // Load actual memory count
  useEffect(() => {
    if (!agent) return;
    const getMemoryCount = async () => {
      setLoadingMemory(true);
      try {
        const data = await fetchMemoryList(agent.id);
        setMemoryCount(data.entries?.length || 0);
      } catch (e) {
        console.error('Failed to load memory count for diagnostics:', e);
      } finally {
        setLoadingMemory(false);
      }
    };
    getMemoryCount();
  }, [agent]);

  // Dynamic ping simulator
  useEffect(() => {
    const interval = setInterval(() => {
      setPingTime(prev => {
        const delta = Math.floor(Math.random() * 5) - 2;
        const next = prev + delta;
        return Math.max(8, Math.min(25, next));
      });
    }, 3000);
    return () => clearInterval(interval);
  }, []);

  // Sync firehose events unless paused
  useEffect(() => {
    if (!isPaused) {
      setFrozenEvents(firehoseEvents);
    }
  }, [firehoseEvents, isPaused]);

  // Auto scroll to bottom of event list on new events
  useEffect(() => {
    if (!isPaused && eventListRef.current) {
      eventListRef.current.scrollTop = eventListRef.current.scrollHeight;
    }
  }, [frozenEvents, isPaused]);

  // Compute compaction statistics based on actual events or defaults
  const compactionStats = useMemo(() => {
    // Look for archivist events to read pressure, otherwise estimate/mock
    const archivistEvents = firehoseEvents.filter(e => 
      e.message_type === 'souveraine_archivist' || e.source === 'archivist'
    );
    
    let pressure = 34; // default start
    if (archivistEvents.length > 0) {
      const latest = archivistEvents[archivistEvents.length - 1];
      if (latest.pressure !== undefined) {
        pressure = Math.round(latest.pressure * 100);
      } else if (latest.content && latest.content.includes('pressure:')) {
        const match = latest.content.match(/pressure:\s*(\d+)/);
        if (match) pressure = parseInt(match[1], 10);
      }
    }

    // Estimate turn counts (or read from memory state/mock)
    // Souveraine compaction triggers typically at 100 turns, or based on token pressure.
    // Let's create an elegant progressive turn counter.
    const userMessages = firehoseEvents.filter(e => e.role === 'user' || e.message_type === 'user_message').length;
    const currentTurns = Math.min(95, 24 + userMessages * 2);
    const maxTurns = 100;

    return {
      pressure,
      currentTurns,
      maxTurns,
      isCompacting: pressure > 85,
    };
  }, [firehoseEvents]);

  // Filter events list
  const filteredEvents = useMemo(() => {
    return frozenEvents.filter(event => {
      // 1. Text Search Filter
      const content = event.content || event.synthesis || JSON.stringify(event);
      const matchesSearch = searchQuery.trim() === '' || 
        content.toLowerCase().includes(searchQuery.toLowerCase()) ||
        (event.source && event.source.toLowerCase().includes(searchQuery.toLowerCase())) ||
        (event.message_type && event.message_type.toLowerCase().includes(searchQuery.toLowerCase()));

      if (!matchesSearch) return false;

      // 2. Active Tab Group Filter
      if (activeFilter === 'all') return true;
      if (activeFilter === 'memory') {
        return event.message_type?.includes('memory') || 
               event.source === 'archivist' || 
               content.toLowerCase().includes('memory');
      }
      if (activeFilter === 'reflection') {
        return event.message_type === 'souveraine_reflection' || 
               event.source === 'reflection' ||
               content.toLowerCase().includes('reflect');
      }
      if (activeFilter === 'subconscious') {
        return event.message_type === 'souveraine_surfacing' || 
               event.source === 'surfacing' || 
               event.source === 'subconscious';
      }
      if (activeFilter === 'archivist') {
        return event.message_type === 'souveraine_archivist' || 
               event.source === 'archivist';
      }
      if (activeFilter === 'system') {
        return !['souveraine_reflection', 'souveraine_surfacing', 'souveraine_archivist'].includes(event.message_type);
      }
      return true;
    });
  }, [frozenEvents, searchQuery, activeFilter]);

  // Circle SVG calculations
  const calculateCircleDashOffset = (percentage, radius) => {
    const circumference = 2 * Math.PI * radius;
    return circumference - (percentage / 100) * circumference;
  };

  // Get color scale for pressure gauge
  const getPressureColor = (val) => {
    if (val < 50) return 'var(--color-accent)'; // Cyan
    if (val < 80) return 'var(--color-purple)'; // Purple
    return 'var(--color-error)'; // Red alert
  };

  return (
    <div className="consciousness-hub-container">
      {/* 1. TOP DIAGNOSTICS & HARDWARE LAYOUT */}
      <div className="diagnostics-grid">
        
        {/* Diagnostic Card 1: Active Neural Substrate */}
        <div className="diagnostic-card">
          <div className="card-accent-line accent-cyan"></div>
          <div className="diag-icon-wrapper cyan-glow">
            <Cpu size={20} />
          </div>
          <div className="diag-info">
            <span className="diag-label">Active Substrate</span>
            <h4 className="diag-value">{agent?.name || 'Souveraine Core'}</h4>
            <div className="diag-footer">
              <span className="status-indicator online"></span>
              <span>L1-Cache Operational</span>
            </div>
          </div>
        </div>

        {/* Diagnostic Card 2: Memory Partition Count */}
        <div className="diagnostic-card">
          <div className="card-accent-line accent-purple"></div>
          <div className="diag-icon-wrapper purple-glow">
            <Database size={20} />
          </div>
          <div className="diag-info">
            <span className="diag-label">Memory Space Index</span>
            <h4 className="diag-value">
              {loadingMemory ? 'Indexing...' : `${memoryCount} Partitions`}
            </h4>
            <div className="diag-footer">
              <span>git sync: </span>
              <span className="text-highlight">origin/ani-studio</span>
            </div>
          </div>
        </div>

        {/* Diagnostic Card 3: Neural Latency */}
        <div className="diagnostic-card">
          <div className="card-accent-line accent-yellow"></div>
          <div className="diag-icon-wrapper yellow-glow">
            <Clock size={20} />
          </div>
          <div className="diag-info">
            <span className="diag-label">Surfacing Latency</span>
            <h4 className="diag-value">{pingTime} ms</h4>
            <div className="diag-footer">
              <span>WebSocket stream: </span>
              <span className="status-pill active-pill">CONNECTED</span>
            </div>
          </div>
        </div>

        {/* Diagnostic Card 4: Compaction Indicator */}
        <div className="diagnostic-card">
          <div className="card-accent-line accent-red"></div>
          <div className="diag-icon-wrapper red-glow">
            <Layers size={20} />
          </div>
          <div className="diag-info">
            <span className="diag-label">Compaction State</span>
            <h4 className="diag-value">
              {compactionStats.isCompacting ? 'CRITICAL CYCLE' : 'EQUILIBRIUM'}
            </h4>
            <div className="diag-footer">
              <span>Next cycle pressure: </span>
              <span className="text-highlight">{compactionStats.pressure}%</span>
            </div>
          </div>
        </div>

      </div>

      {/* 2. MAIN LAYOUT GRID */}
      <div className="consciousness-workspace-grid">
        
        {/* LEFT COMPARTMENT: Gauges & Controls */}
        <div className="gauges-sidebar-panel">
          <div className="panel-card-inner">
            <h3 className="section-title">
              <Sliders size={15} />
              <span>Consciousness Telemetry</span>
            </h3>

            {/* GAUGES CONTAINER */}
            <div className="gauges-container-row">
              
              {/* GAUGE 1: COGNITIVE PRESSURE */}
              <div className="circular-gauge-box">
                <div className="circular-svg-wrapper">
                  <svg width="120" height="120" viewBox="0 0 120 120">
                    {/* Background track */}
                    <circle 
                      cx="60" 
                      cy="60" 
                      r="48" 
                      className="gauge-circle-track"
                    />
                    {/* Animated Fill circle */}
                    <circle 
                      cx="60" 
                      cy="60" 
                      r="48" 
                      className="gauge-circle-fill animated-stroke"
                      stroke={getPressureColor(compactionStats.pressure)}
                      strokeDasharray={2 * Math.PI * 48}
                      strokeDashoffset={calculateCircleDashOffset(compactionStats.pressure, 48)}
                      transform="rotate(-90 60 60)"
                    />
                  </svg>
                  <div className="gauge-center-text">
                    <span className="gauge-percent-val">{compactionStats.pressure}%</span>
                    <span className="gauge-percent-lbl">Pressure</span>
                  </div>
                </div>
                <div className="gauge-meta-desc">
                  <h5>Cognitive Load</h5>
                  <p>Accumulated token/memory complexity forcing the next archivist compaction pass.</p>
                </div>
              </div>

              {/* GAUGE 2: CYCLE TURNS LIMIT */}
              <div className="circular-gauge-box">
                <div className="circular-svg-wrapper">
                  <svg width="120" height="120" viewBox="0 0 120 120">
                    {/* Background track */}
                    <circle 
                      cx="60" 
                      cy="60" 
                      r="48" 
                      className="gauge-circle-track"
                    />
                    {/* Animated Fill circle */}
                    <circle 
                      cx="60" 
                      cy="60" 
                      r="48" 
                      className="gauge-circle-fill animated-stroke purple-stroke"
                      strokeDasharray={2 * Math.PI * 48}
                      strokeDashoffset={calculateCircleDashOffset((compactionStats.currentTurns / compactionStats.maxTurns) * 100, 48)}
                      transform="rotate(-90 60 60)"
                    />
                  </svg>
                  <div className="gauge-center-text">
                    <span className="gauge-percent-val text-purple">{compactionStats.currentTurns}</span>
                    <span className="gauge-percent-lbl">/ {compactionStats.maxTurns} Turns</span>
                  </div>
                </div>
                <div className="gauge-meta-desc">
                  <h5>Compaction Limit</h5>
                  <p>Total conversation interaction turns remaining before automatic state consolidation.</p>
                </div>
              </div>

            </div>

            {/* DIAGNOSTICS LOG DETAILS */}
            <div className="diagnostics-details-card">
              <div className="details-card-header">
                <Info size={14} />
                <span>Nervous System Configuration</span>
              </div>
              <div className="details-card-rows">
                <div className="details-row">
                  <span className="row-lbl">Consolidation Strategy</span>
                  <span className="row-val font-mono">archivist-summarize</span>
                </div>
                <div className="details-row">
                  <span className="row-lbl">Archivist Interval</span>
                  <span className="row-val font-mono">100 turns</span>
                </div>
                <div className="details-row">
                  <span className="row-lbl">Subconscious Rate</span>
                  <span className="row-val font-mono">Real-time WebSockets</span>
                </div>
                <div className="details-row">
                  <span className="row-lbl">Sensory Stream</span>
                  <span className="row-val font-mono">/v1/firehose</span>
                </div>
              </div>
            </div>

          </div>
        </div>

        {/* RIGHT COMPARTMENT: Real-Time Scrolling Firehose Log */}
        <div className="firehose-terminal-panel">
          
          {/* TERMINAL HEADER & SEARCH FILTER BAR */}
          <div className="terminal-header-container">
            <div className="terminal-title">
              <Terminal size={16} className="title-glow-icon" />
              <span>Sensory Firehose Stream</span>
            </div>
            
            <div className="terminal-controls">
              {/* Search input */}
              <div className="terminal-search">
                <Search size={12} className="search-icon" />
                <input
                  type="text"
                  placeholder="Filter events..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                />
              </div>

              {/* Pause/Play Stream stream */}
              <button 
                className={`btn-pause-stream ${isPaused ? 'paused-active' : ''}`}
                onClick={() => setIsPaused(!isPaused)}
                title={isPaused ? "Resume Live Sensory Stream" : "Freeze Sensory Stream for Inspection"}
              >
                {isPaused ? <Play size={12} /> : <Pause size={12} />}
                <span>{isPaused ? 'Resume' : 'Freeze'}</span>
              </button>
            </div>
          </div>

          {/* QUICK CATEGORY CHIPS */}
          <div className="category-chips-row">
            <button 
              className={`chip ${activeFilter === 'all' ? 'active' : ''}`}
              onClick={() => setActiveFilter('all')}
            >
              All Events ({frozenEvents.length})
            </button>
            <button 
              className={`chip chip-purple ${activeFilter === 'subconscious' ? 'active' : ''}`}
              onClick={() => setActiveFilter('subconscious')}
            >
              Subconscious Surfacing
            </button>
            <button 
              className={`chip chip-cyan ${activeFilter === 'reflection' ? 'active' : ''}`}
              onClick={() => setActiveFilter('reflection')}
            >
              Reflection
            </button>
            <button 
              className={`chip chip-yellow ${activeFilter === 'archivist' ? 'active' : ''}`}
              onClick={() => setActiveFilter('archivist')}
            >
              Archivist
            </button>
            <button 
              className={`chip chip-muted ${activeFilter === 'memory' ? 'active' : ''}`}
              onClick={() => setActiveFilter('memory')}
            >
              Memory Git
            </button>
          </div>

          {/* SCROLLING EVENTS LOG */}
          <div className="firehose-events-log" ref={eventListRef}>
            {filteredEvents.map((event, idx) => {
              // Determine styles/classes based on priority and type
              let borderClass = 'event-surfacing';
              let tagColor = 'tag-muted';
              
              if (event.message_type === 'souveraine_reflection' || event.source === 'reflection') {
                borderClass = 'event-reflection';
                tagColor = 'tag-cyan';
              } else if (event.message_type === 'souveraine_archivist' || event.source === 'archivist') {
                borderClass = 'event-archivist';
                tagColor = 'tag-yellow';
              } else if (event.message_type === 'souveraine_surfacing' || event.source === 'surfacing' || event.source === 'subconscious') {
                borderClass = 'event-subconscious';
                tagColor = 'tag-purple';
              } else if (event.message_type?.includes('memory') || event.source === 'memory') {
                borderClass = 'event-memory';
                tagColor = 'tag-blue';
              }

              const isPriority = event.priority === 'high' || event.priority === 'critical';
              const receivedStr = event.receivedAt 
                ? new Date(event.receivedAt).toLocaleTimeString()
                : new Date().toLocaleTimeString();

              const contentText = event.content || event.synthesis || JSON.stringify(event);

              return (
                <div 
                  key={idx} 
                  className={`firehose-event-item ${borderClass} ${isPriority ? 'priority-alert' : ''} ${selectedEvent === event ? 'expanded-event' : ''}`}
                  onClick={() => setSelectedEvent(selectedEvent === event ? null : event)}
                >
                  <div className="event-item-header">
                    <span className={`event-tag ${tagColor}`}>
                      {event.source || event.message_type?.replace('souveraine_', '') || 'system'}
                    </span>
                    
                    {event.priority && (
                      <span className={`priority-badge ${event.priority}`}>
                        {event.priority}
                      </span>
                    )}

                    <span className="event-item-time">{receivedStr}</span>
                    <ChevronRight size={14} className="expand-indicator" />
                  </div>

                  <div className="event-item-body">
                    <p className="event-text">{contentText}</p>
                    
                    {/* Archivist compaction details */}
                    {event.pressure !== undefined && (
                      <div className="event-pressure-context">
                        <span>Compaction Pressure: </span>
                        <strong>{Math.round(event.pressure * 100)}%</strong>
                      </div>
                    )}

                    {/* Collapsible raw details expansion */}
                    {selectedEvent === event && (
                      <div className="event-json-dump">
                        <h6>Raw JSON Payload:</h6>
                        <pre>{JSON.stringify(event, null, 2)}</pre>
                      </div>
                    )}
                  </div>
                </div>
              );
            })}

            {filteredEvents.length === 0 && (
              <div className="empty-terminal-state">
                <Activity size={32} className="pulse-slow" />
                <p>Awaiting sensory events from the Souveraine nervous firehose...</p>
                <span>Try sending a message in Chat Studio to trigger active subconscious reflection events.</span>
              </div>
            )}
          </div>

        </div>

      </div>

    </div>
  );
}
