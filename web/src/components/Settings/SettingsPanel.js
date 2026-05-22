'use client';
import { useState, useEffect, useCallback } from 'react';
import { 
  Sliders, 
  History, 
  Save, 
  Loader2, 
  Cpu, 
  Database, 
  GitCommit, 
  AlertTriangle,
  Check,
  Search,
  Filter,
  ChevronRight,
  Shield,
  Layers,
  Sparkles
} from 'lucide-react';
import { fetchServerConfig, updateServerConfig, fetchCompactionLogs } from '@/lib/api';

export default function SettingsPanel({ agent }) {
  const [activeSubTab, setActiveSubTab] = useState('physics'); // 'physics' | 'compaction-logs'
  const [config, setConfig] = useState(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState(null);
  const [successMsg, setSuccessMsg] = useState(null);

  // Compaction logs state
  const [logs, setLogs] = useState([]);
  const [loadingLogs, setLoadingLogs] = useState(false);
  const [logSearch, setLogSearch] = useState('');
  const [expandedLog, setExpandedLog] = useState(null);

  // Load server TOML config
  const loadConfig = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await fetchServerConfig();
      setConfig(data);
    } catch (e) {
      setError(`Failed to load server configurations: ${e.message}`);
    } finally {
      setLoading(false);
    }
  }, []);

  // Load compaction logs from disk
  const loadLogs = useCallback(async () => {
    setLoadingLogs(true);
    try {
      const data = await fetchCompactionLogs();
      setLogs(data.logs || []);
    } catch (e) {
      console.warn('Failed to load compaction logs history:', e);
    } finally {
      setLoadingLogs(false);
    }
  }, []);

  useEffect(() => {
    loadConfig();
  }, [loadConfig]);

  useEffect(() => {
    if (activeSubTab === 'compaction-logs') {
      loadLogs();
    }
  }, [activeSubTab, loadLogs]);

  // Handle value change for nested TOML properties
  const handleNestedChange = (section, field, value) => {
    setConfig(prev => ({
      ...prev,
      [section]: {
        ...prev[section],
        [field]: value
      }
    }));
  };

  // Save configurations to souveraine.toml
  const handleSave = async (e) => {
    e.preventDefault();
    if (!config) return;
    setSaving(true);
    setError(null);
    setSuccessMsg(null);
    try {
      const updated = await updateServerConfig(config);
      setConfig(updated);
      setSuccessMsg('Settings saved and hot-reloaded live on the server successfully!');
      setTimeout(() => setSuccessMsg(null), 3000);
    } catch (e) {
      setError(`Failed to save settings: ${e.message}`);
    } finally {
      setSaving(false);
    }
  };

  // Filter logs based on search string
  const filteredLogs = logs.filter(log => {
    const text = log.content || log.payload?.content || JSON.stringify(log);
    return logSearch.trim() === '' || text.toLowerCase().includes(logSearch.toLowerCase());
  });

  return (
    <div className="settings-panel-container">
      {/* PANEL TABS SWITCHER */}
      <div className="settings-panel-header">
        <div className="panel-tab-group">
          <button 
            className={`panel-tab-btn ${activeSubTab === 'physics' ? 'active' : ''}`}
            onClick={() => setActiveSubTab('physics')}
          >
            <Sliders size={16} />
            <span>Model & Cognitive Physics</span>
          </button>
          <button 
            className={`panel-tab-btn ${activeSubTab === 'compaction-logs' ? 'active' : ''}`}
            onClick={() => setActiveSubTab('compaction-logs')}
          >
            <History size={16} />
            <span>Compaction Logs Explorer</span>
          </button>
        </div>

        {activeSubTab === 'physics' && config && (
          <button 
            className="btn-primary btn-settings-save" 
            onClick={handleSave}
            disabled={saving}
          >
            {saving ? <Loader2 className="animate-spin" size={14} /> : <Save size={14} />}
            <span>{saving ? 'Saving...' : 'Save & Hot-Reload'}</span>
          </button>
        )}
      </div>

      {/* ALERTS MESSAGE */}
      {error && (
        <div className="alert-message error-alert">
          <AlertTriangle size={16} />
          <span>{error}</span>
        </div>
      )}
      {successMsg && (
        <div className="alert-message success-alert">
          <Check size={16} />
          <span>{successMsg}</span>
        </div>
      )}

      {/* VIEWPORT AREA */}
      <div className="settings-panel-viewport">
        {loading ? (
          <div className="panel-loader">
            <Loader2 className="animate-spin" size={32} />
            <span>Loading active configurations...</span>
          </div>
        ) : activeSubTab === 'physics' && config ? (
          <form className="settings-forms-flow" onSubmit={handleSave}>
            
            {/* SECTION 1: BIFROST INFERENCE GATEWAY */}
            <div className="settings-section-card">
              <div className="settings-card-header">
                <Cpu size={16} className="text-cyan" />
                <h4>Inference Gateway (Bifrost)</h4>
              </div>
              <div className="settings-card-body inputs-grid-2">
                <div className="form-group">
                  <label>Base URL Endpoint</label>
                  <input 
                    type="text" 
                    value={config.bifrost?.base_url || ''} 
                    onChange={(e) => handleNestedChange('bifrost', 'base_url', e.target.value)}
                    placeholder="http://127.0.0.1:3360"
                  />
                </div>
                <div className="form-group">
                  <label>Gateway Key / Bearer Token</label>
                  <input 
                    type="password" 
                    value={config.bifrost?.api_key || ''} 
                    onChange={(e) => handleNestedChange('bifrost', 'api_key', e.target.value)}
                    placeholder="bifrost_api_key_bearer"
                  />
                </div>
                <div className="form-group">
                  <label>Primary Model Override</label>
                  <input 
                    type="text" 
                    value={config.bifrost?.primary_model || ''} 
                    onChange={(e) => handleNestedChange('bifrost', 'primary_model', e.target.value)}
                    placeholder="openai/kimi-k2.6"
                  />
                </div>
                <div className="form-group">
                  <label>Timeout Ceiling (Seconds)</label>
                  <input 
                    type="number" 
                    value={config.bifrost?.timeout_secs || 120} 
                    onChange={(e) => handleNestedChange('bifrost', 'timeout_secs', parseInt(e.target.value, 10) || 120)}
                  />
                </div>
              </div>
            </div>

            {/* SECTION 2: SUBCONSCIOUS (N+1) */}
            <div className="settings-section-card">
              <div className="settings-card-header">
                <Sparkles size={16} className="text-purple" />
                <h4>Subconscious Processor (N+1)</h4>
              </div>
              <div className="settings-card-body">
                <div className="toggles-row">
                  <label className="checkbox-form-group">
                    <input 
                      type="checkbox" 
                      checked={config.subconscious?.n1_enabled ?? true} 
                      onChange={(e) => handleNestedChange('subconscious', 'n1_enabled', e.target.checked)}
                    />
                    <div>
                      <strong>Enable N+1 Subconscious</strong>
                      <p>Wakes up the agent's subconscious pass dynamically to process inbox events.</p>
                    </div>
                  </label>
                  
                  <label className="checkbox-form-group">
                    <input 
                      type="checkbox" 
                      checked={config.subconscious?.inbox_enabled ?? true} 
                      onChange={(e) => handleNestedChange('subconscious', 'inbox_enabled', e.target.checked)}
                    />
                    <div>
                      <strong>Enable Active Inbox</strong>
                      <p>Permits active surfacing cron alerts and background sensor events queue.</p>
                    </div>
                  </label>
                </div>

                <div className="inputs-grid-2 margin-top-md">
                  <div className="form-group">
                    <label>Subconscious Model Override (Optional)</label>
                    <input 
                      type="text" 
                      value={config.subconscious?.model || ''} 
                      onChange={(e) => handleNestedChange('subconscious', 'model', e.target.value || null)}
                      placeholder="Use Primary Model if empty"
                    />
                  </div>
                  <div className="form-group">
                    <label>Subconscious Output Token Ceiling</label>
                    <input 
                      type="number" 
                      value={config.subconscious?.max_tokens || ''} 
                      onChange={(e) => handleNestedChange('subconscious', 'max_tokens', parseInt(e.target.value, 10) || null)}
                      placeholder="e.g. 4096 (Uncapped if empty)"
                    />
                  </div>
                </div>

                <div className="form-group margin-top-md">
                  <label>Subconscious System Prompt</label>
                  <textarea 
                    value={config.subconscious?.system_prompt || ''} 
                    onChange={(e) => handleNestedChange('subconscious', 'system_prompt', e.target.value || null)}
                    placeholder="Customize subconscious orientation..."
                    rows={3}
                  />
                </div>
              </div>
            </div>

            {/* SECTION 3: ARCHIVIST COMPACTION */}
            <div className="settings-section-card">
              <div className="settings-card-header">
                <Layers size={16} className="text-yellow" />
                <h4>Archivist Compaction & Thresholds</h4>
              </div>
              <div className="settings-card-body">
                <label className="checkbox-form-group">
                  <input 
                    type="checkbox" 
                    checked={config.archivist?.enabled ?? true} 
                    onChange={(e) => handleNestedChange('archivist', 'enabled', e.target.checked)}
                  />
                  <div>
                    <strong>Enable Automatic Memory Compaction</strong>
                    <p>Allows the Archivist to synthesize and summarize conversation tokens to prevent context overflow.</p>
                  </div>
                </label>

                <div className="inputs-grid-2 margin-top-md">
                  <div className="form-group">
                    <label>Compaction Interval (Turns count)</label>
                    <input 
                      type="number" 
                      value={config.archivist?.interval || 100} 
                      onChange={(e) => handleNestedChange('archivist', 'interval', parseInt(e.target.value, 10) || 100)}
                    />
                  </div>
                  <div className="form-group">
                    <label>Compression Model</label>
                    <input 
                      type="text" 
                      value={config.archivist?.compression_model || 'auto'} 
                      onChange={(e) => handleNestedChange('archivist', 'compression_model', e.target.value || 'auto')}
                    />
                  </div>
                </div>

                <div className="form-group margin-top-md">
                  <div className="slider-header">
                    <label>Compaction Threshold: {(config.archivist?.threshold * 100).toFixed(0)}%</label>
                    <span className="slider-hint">Triggers warnings when context pressure exceeds this percentage.</span>
                  </div>
                  <input 
                    type="range" 
                    min="0.1" 
                    max="0.95" 
                    step="0.05"
                    value={config.archivist?.threshold || 0.7} 
                    onChange={(e) => handleNestedChange('archivist', 'threshold', parseFloat(e.target.value))}
                    className="slider-input"
                  />
                </div>
              </div>
            </div>

            {/* SECTION 4: REFLECTION (N+25) */}
            <div className="settings-section-card">
              <div className="settings-card-header">
                <Bot size={16} className="text-purple" />
                <h4>Self-Reflection Physics (N+25)</h4>
              </div>
              <div className="settings-card-body">
                <label className="checkbox-form-group">
                  <input 
                    type="checkbox" 
                    checked={config.reflection?.enabled ?? true} 
                    onChange={(e) => handleNestedChange('reflection', 'enabled', e.target.checked)}
                  />
                  <div>
                    <strong>Enable Active Self-Reflection</strong>
                    <p>Fires background self-awareness assessments to audit goals, constraints, and priorities.</p>
                  </div>
                </label>

                <div className="inputs-grid-2 margin-top-md">
                  <div className="form-group">
                    <label>Reflection Step Interval (Turns)</label>
                    <input 
                      type="number" 
                      value={config.reflection?.message_interval || 25} 
                      onChange={(e) => handleNestedChange('reflection', 'message_interval', parseInt(e.target.value, 10) || 25)}
                    />
                  </div>
                  <div className="form-group">
                    <label>Trigger Mode</label>
                    <select
                      value={config.reflection?.trigger || 'step_count'}
                      onChange={(e) => handleNestedChange('reflection', 'trigger', e.target.value)}
                    >
                      <option value="step_count">step_count (Fixed turn intervals)</option>
                      <option value="compaction_event">compaction_event (Consolidation post-run)</option>
                      <option value="off">off (Disabled)</option>
                    </select>
                  </div>
                </div>
              </div>
            </div>

            {/* SECTION 5: GIT REPO MEMORY BACKEND */}
            <div className="settings-section-card">
              <div className="settings-card-header">
                <GitCommit size={16} className="text-blue" />
                <h4>Memory Git Sync Controls</h4>
              </div>
              <div className="settings-card-body">
                <div className="toggles-row">
                  <label className="checkbox-form-group">
                    <input 
                      type="checkbox" 
                      checked={config.memory?.git_enabled ?? true} 
                      onChange={(e) => handleNestedChange('memory', 'git_enabled', e.target.checked)}
                    />
                    <div>
                      <strong>Enable Git Backed Memory</strong>
                      <p>Saves all memory space text edits as standard commits.</p>
                    </div>
                  </label>

                  <label className="checkbox-form-group">
                    <input 
                      type="checkbox" 
                      checked={config.memory?.auto_commit ?? true} 
                      onChange={(e) => handleNestedChange('memory', 'auto_commit', e.target.checked)}
                    />
                    <div>
                      <strong>Enable Auto-Commit</strong>
                      <p>Skips custom git modals and auto-commits using a default message.</p>
                    </div>
                  </label>

                  <label className="checkbox-form-group">
                    <input 
                      type="checkbox" 
                      checked={config.memory?.auto_push ?? false} 
                      onChange={(e) => handleNestedChange('memory', 'auto_push', e.target.checked)}
                    />
                    <div>
                      <strong>Auto-Push to Remote Fork</strong>
                      <p>Pushes commits immediately to origin/ani-studio on GitHub.</p>
                    </div>
                  </label>
                </div>
              </div>
            </div>

          </form>
        ) : (
          /* COMPACTION LOGS EXPLORER SUB-TAB */
          <div className="compaction-explorer-inner">
            
            {/* Search filter panel */}
            <div className="explorer-filters-bar">
              <div className="search-bar-container settings-search-log">
                <Search size={14} className="search-icon" />
                <input 
                  type="text" 
                  placeholder="Search compaction logs..."
                  value={logSearch}
                  onChange={(e) => setLogSearch(e.target.value)}
                />
              </div>
              <button className="btn-secondary" onClick={loadLogs} disabled={loadingLogs}>
                <span>Refresh Archives</span>
              </button>
            </div>

            {/* Scrolling logs timeline */}
            <div className="compaction-timeline">
              {loadingLogs ? (
                <div className="panel-loader">
                  <Loader2 className="animate-spin" size={24} />
                  <span>Loading compaction logs...</span>
                </div>
              ) : filteredLogs.length > 0 ? (
                <div className="timeline-items-flow">
                  {filteredLogs.map((log, idx) => {
                    const receivedStr = log.timestamp 
                      ? new Date(log.timestamp).toLocaleString()
                      : new Date().toLocaleString();

                    const isExpanded = expandedLog === log;
                    const contentText = log.payload?.content || log.content || JSON.stringify(log);
                    
                    return (
                      <div 
                        key={idx} 
                        className={`timeline-log-item ${isExpanded ? 'expanded-log' : ''}`}
                        onClick={() => setExpandedLog(isExpanded ? null : log)}
                      >
                        <div className="log-item-header">
                          <div className="log-badge-group">
                            <span className="log-indicator-dot dot-yellow"></span>
                            <span className="log-date">{receivedStr}</span>
                          </div>
                          
                          {log.payload?.pressure !== undefined && (
                            <span className="log-pressure-tag">
                              {(log.payload.pressure * 100).toFixed(0)}% pressure
                            </span>
                          )}
                          <ChevronRight size={14} className="expand-chevron" />
                        </div>

                        <div className="log-item-body">
                          <p className="log-synthesis-text">{contentText}</p>

                          {isExpanded && (
                            <div className="log-raw-payload-box">
                              <h6>Raw Event Payload:</h6>
                              <pre>{JSON.stringify(log, null, 2)}</pre>
                            </div>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              ) : (
                <div className="empty-terminal-state">
                  <History size={32} className="pulse-slow" />
                  <p>No historical compaction logs found.</p>
                  <span>Consolidation archives will appear here once the agent hits turn cycles and compiles summaries.</span>
                </div>
              )}
            </div>

          </div>
        )}
      </div>
    </div>
  );
}
