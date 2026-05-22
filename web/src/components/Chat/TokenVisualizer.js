'use client';
import { useState, useEffect, useCallback } from 'react';
import { Layers, AlertTriangle, Shield, User, Bot, HelpCircle } from 'lucide-react';
import { fetchConversationTokens } from '@/lib/api';

export default function TokenVisualizer({ conversationId, messagesCount = 0 }) {
  const [metrics, setMetrics] = useState(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(null);
  const [showTooltip, setShowTooltip] = useState(false);

  const loadTokens = useCallback(async () => {
    if (!conversationId) return;
    setLoading(true);
    setError(null);
    try {
      const data = await fetchConversationTokens(conversationId);
      setMetrics(data);
    } catch (e) {
      console.warn('Failed to load token metrics:', e);
      setError(e.message);
    } finally {
      setLoading(false);
    }
  }, [conversationId]);

  // Load tokens on mount, when conversationId changes, or when messagesCount changes
  useEffect(() => {
    loadTokens();
  }, [loadTokens, messagesCount]);

  if (!conversationId || !metrics) {
    return null;
  }

  const {
    system_tokens = 0,
    user_tokens = 0,
    assistant_tokens = 0,
    total_tokens = 0,
    context_limit = 128000,
    percentage = 0
  } = metrics;

  // Format numbers nicely with commas
  const fmt = (num) => new Intl.NumberFormat().format(num);

  const sysPct = context_limit > 0 ? (system_tokens / context_limit) * 100 : 0;
  const userPct = context_limit > 0 ? (user_tokens / context_limit) * 100 : 0;
  const asstPct = context_limit > 0 ? (assistant_tokens / context_limit) * 100 : 0;
  const freePct = Math.max(0, 100 - (sysPct + userPct + asstPct));

  const isWarning = percentage > 0.8;
  const isCritical = percentage > 0.92;

  return (
    <div className={`token-visualizer-card ${isCritical ? 'critical-alert' : isWarning ? 'warning-alert' : ''}`}>
      <div className="token-visualizer-header">
        <div className="visualizer-label-group">
          <Layers size={14} className="visualizer-icon" />
          <span className="visualizer-title">Context Headroom</span>
        </div>
        
        <div className="visualizer-metrics-summary" onClick={() => setShowTooltip(!showTooltip)}>
          <span className="token-usage-values">
            <strong>{fmt(total_tokens)}</strong> / {fmt(context_limit)}
          </span>
          <span className="token-percentage-badge">
            {(percentage * 100).toFixed(1)}%
          </span>
        </div>
      </div>

      {/* MULTI-SEGMENT BAR */}
      <div 
        className="token-segmented-bar"
        onMouseEnter={() => setShowTooltip(true)}
        onMouseLeave={() => setShowTooltip(false)}
      >
        <div 
          className="bar-segment segment-system" 
          style={{ width: `${sysPct}%` }}
          title={`System: ${fmt(system_tokens)} tokens (${sysPct.toFixed(1)}%)`}
        />
        <div 
          className="bar-segment segment-user" 
          style={{ width: `${userPct}%` }}
          title={`User: ${fmt(user_tokens)} tokens (${userPct.toFixed(1)}%)`}
        />
        <div 
          className="bar-segment segment-assistant" 
          style={{ width: `${asstPct}%` }}
          title={`Assistant: ${fmt(assistant_tokens)} tokens (${asstPct.toFixed(1)}%)`}
        />
        <div 
          className="bar-segment segment-free" 
          style={{ width: `${freePct}%` }}
          title={`Free Context: ${fmt(context_limit - total_tokens)} tokens (${freePct.toFixed(1)}%)`}
        />
      </div>

      {/* TOOLTIP TABLE */}
      {showTooltip && (
        <div className="token-visualizer-tooltip">
          <h5 className="tooltip-title">Context Space Allocation</h5>
          <div className="tooltip-table">
            <div className="tooltip-row">
              <span className="dot dot-system"></span>
              <span className="row-role"><Shield size={10} /> System Prompt</span>
              <span className="row-count">{fmt(system_tokens)}</span>
              <span className="row-ratio">{sysPct.toFixed(1)}%</span>
            </div>
            <div className="tooltip-row">
              <span className="dot dot-user"></span>
              <span className="row-role"><User size={10} /> User & Tools Context</span>
              <span className="row-count">{fmt(user_tokens)}</span>
              <span className="row-ratio">{userPct.toFixed(1)}%</span>
            </div>
            <div className="tooltip-row">
              <span className="dot dot-assistant"></span>
              <span className="row-role"><Bot size={10} /> Assistant Responses</span>
              <span className="row-count">{fmt(assistant_tokens)}</span>
              <span className="row-ratio">{asstPct.toFixed(1)}%</span>
            </div>
            <div className="tooltip-row divider"></div>
            <div className="tooltip-row highlight-row">
              <span className="dot dot-free"></span>
              <span className="row-role"><HelpCircle size={10} /> Free Headroom</span>
              <span className="row-count">{fmt(context_limit - total_tokens)}</span>
              <span className="row-ratio">{freePct.toFixed(1)}%</span>
            </div>
          </div>
        </div>
      )}

      {/* CRITICAL/WARNING STATE ANNOUNCER */}
      {(isWarning || isCritical) && (
        <div className="visualizer-warning-announcer">
          <AlertTriangle size={12} />
          <span>
            {isCritical 
              ? 'Consolidation urgent: context pressure above 90%!'
              : 'Compaction suggested: context pressure above 80%'
            }
          </span>
        </div>
      )}
    </div>
  );
}
