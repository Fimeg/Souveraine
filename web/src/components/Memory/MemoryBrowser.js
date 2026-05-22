'use client';
import { useState, useEffect, useCallback } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';
import { 
  Folder, 
  FileText, 
  Save, 
  Trash2, 
  Plus, 
  ChevronRight, 
  ChevronDown, 
  Eye, 
  EyeOff, 
  Lock, 
  Unlock, 
  GitCommit, 
  History, 
  Loader2, 
  Search,
  Check,
  AlertCircle
} from 'lucide-react';
import { fetchMemoryList, fetchMemoryFile, writeMemoryFile, deleteMemoryFile } from '@/lib/api';

export default function MemoryBrowser({ agent }) {
  // Navigation & File Tree
  const [files, setFiles] = useState([]);
  const [loading, setLoading] = useState(true);
  const [expandedFolders, setExpandedFolders] = useState({ system: true, journal: true, ledger: true });
  const [searchQuery, setSearchQuery] = useState('');

  // Selected File details
  const [selectedPath, setSelectedPath] = useState(null);
  const [selectedFile, setSelectedFile] = useState(null);
  const [fileLoading, setFileLoading] = useState(false);

  // Editor states
  const [editedBody, setEditedBody] = useState('');
  const [editedDescription, setEditedDescription] = useState('');
  const [editedReadOnly, setEditedReadOnly] = useState('false');
  const [editedTags, setEditedTags] = useState('');
  const [editedLimit, setEditedLimit] = useState('');
  const [hasChanges, setHasChanges] = useState(false);

  // Git Commit Configs
  const [autoCommit, setAutoCommit] = useState(true);
  const [showCommitModal, setShowCommitModal] = useState(false);
  const [commitMessage, setCommitMessage] = useState('');

  // Layout Configuration (Resizable/Collapsible Right Preview Panel)
  const [showPreview, setShowPreview] = useState(true);

  // Dialog / Action overlays
  const [isCreating, setIsCreating] = useState(false);
  const [newFilePath, setNewFilePath] = useState('');
  const [newFileFolder, setNewFileFolder] = useState('journal');
  const [newFileDescription, setNewFileDescription] = useState('');
  
  const [error, setError] = useState(null);
  const [successMsg, setSuccessMsg] = useState(null);

  // Load files
  const loadFiles = useCallback(async () => {
    if (!agent) return;
    setLoading(true);
    setError(null);
    try {
      const data = await fetchMemoryList(agent.id);
      setFiles(data.entries || []);
    } catch (e) {
      setError(`Failed to load memory index: ${e.message}`);
    } finally {
      setLoading(false);
    }
  }, [agent]);

  useEffect(() => {
    loadFiles();
  }, [loadFiles]);

  // Load single file
  const selectFile = async (path) => {
    setFileLoading(true);
    setError(null);
    setSuccessMsg(null);
    try {
      const data = await fetchMemoryFile(agent.id, path);
      setSelectedPath(path);
      setSelectedFile(data);
      setEditedBody(data.body || '');
      setEditedDescription(data.frontmatter?.description || '');
      setEditedReadOnly(data.frontmatter?.read_only || 'false');
      setEditedTags(data.frontmatter?.tags?.join(', ') || '');
      setEditedLimit(data.frontmatter?.limit?.toString() || '');
      setHasChanges(false);
    } catch (e) {
      setError(`Failed to read file: ${e.message}`);
    } finally {
      setFileLoading(false);
    }
  };

  // Check changes
  useEffect(() => {
    if (!selectedFile) return;
    const bodyChanged = editedBody !== (selectedFile.body || '');
    const descChanged = editedDescription !== (selectedFile.frontmatter?.description || '');
    const roChanged = editedReadOnly !== (selectedFile.frontmatter?.read_only || 'false');
    const tagsChanged = editedTags !== (selectedFile.frontmatter?.tags?.join(', ') || '');
    const limitChanged = editedLimit !== (selectedFile.frontmatter?.limit?.toString() || '');

    setHasChanges(bodyChanged || descChanged || roChanged || tagsChanged || limitChanged);
  }, [editedBody, editedDescription, editedReadOnly, editedTags, editedLimit, selectedFile]);

  // Save changes
  const handleSave = async (forceCommitMsg = null) => {
    if (!selectedPath || !agent) return;
    setError(null);
    setSuccessMsg(null);

    // Format YAML frontmatter
    const tagsArray = editedTags
      .split(',')
      .map(t => t.trim())
      .filter(t => t.length > 0);

    const fm = {
      description: editedDescription.trim(),
    };

    if (editedReadOnly === 'true') {
      fm.read_only = 'true';
    }
    if (tagsArray.length > 0) {
      fm.tags = tagsArray;
    }
    const limitVal = parseInt(editedLimit, 10);
    if (!isNaN(limitVal)) {
      fm.limit = limitVal;
    }

    // Convert fm to yaml
    let yaml = '---\n';
    yaml += `description: "${fm.description.replace(/"/g, '\\"')}"\n`;
    if (fm.read_only) yaml += `read_only: "true"\n`;
    if (fm.tags) {
      yaml += `tags:\n`;
      fm.tags.forEach(t => {
        yaml += `  - ${t}\n`;
      });
    }
    if (fm.limit) yaml += `limit: ${fm.limit}\n`;
    yaml += '---\n\n';

    const fullContent = yaml + editedBody;

    try {
      await writeMemoryFile(agent.id, selectedPath, fullContent);
      
      // Update local state to match saved content
      const updatedFile = {
        path: selectedPath,
        frontmatter: {
          description: editedDescription,
          read_only: editedReadOnly,
          tags: tagsArray,
          limit: limitVal || null,
        },
        body: editedBody,
      };
      setSelectedFile(updatedFile);
      setHasChanges(false);
      setSuccessMsg(`File "${selectedPath}" committed successfully!`);
      setTimeout(() => setSuccessMsg(null), 3000);
      setShowCommitModal(false);
    } catch (e) {
      setError(`Failed to save: ${e.message}`);
    }
  };

  const handleSaveTrigger = () => {
    if (autoCommit) {
      handleSave();
    } else {
      setCommitMessage(`feat(memory): update ${selectedPath}`);
      setShowCommitModal(true);
    }
  };

  // Delete file
  const handleDelete = async () => {
    if (!selectedPath || !confirm(`Are you absolutely sure you want to delete ${selectedPath}?`)) return;
    setError(null);
    try {
      await deleteMemoryFile(agent.id, selectedPath);
      setSelectedPath(null);
      setSelectedFile(null);
      setSuccessMsg('File deleted successfully.');
      setTimeout(() => setSuccessMsg(null), 3000);
      loadFiles();
    } catch (e) {
      setError(`Delete failed: ${e.message}`);
    }
  };

  // Create file
  const handleCreate = async (e) => {
    e.preventDefault();
    if (!newFilePath.trim() || !agent) return;
    
    setError(null);
    setSuccessMsg(null);

    let cleanPath = newFilePath.trim();
    if (!cleanPath.endsWith('.md')) {
      cleanPath += '.md';
    }

    const fullPath = `${newFileFolder}/${cleanPath}`;

    const defaultContent = `---\ndescription: "${newFileDescription || 'Memory file'}"\nread_only: false\ntags:\n  - ${newFileFolder}\n---\n\n# New Memory\n`;

    try {
      await writeMemoryFile(agent.id, fullPath, defaultContent);
      setNewFilePath('');
      setNewFileDescription('');
      setIsCreating(false);
      await loadFiles();
      selectFile(fullPath);
    } catch (e) {
      setError(`Create file failed: ${e.message}`);
    }
  };

  // Toggle folder expansion
  const toggleFolder = (folder) => {
    setExpandedFolders(prev => ({ ...prev, [folder]: !prev[folder] }));
  };

  // Group files by root directory
  const groupFiles = () => {
    const tree = {};
    const filtered = files.filter(f => f.toLowerCase().includes(searchQuery.toLowerCase()));

    filtered.forEach(file => {
      const parts = file.split('/');
      if (parts.length > 1) {
        const folder = parts[0];
        const fileName = parts.slice(1).join('/');
        if (!tree[folder]) tree[folder] = [];
        tree[folder].push({ fullName: file, name: fileName });
      } else {
        if (!tree['/']) tree['/'] = [];
        tree['/'].push({ fullName: file, name: file });
      }
    });
    return tree;
  };

  const fileTree = groupFiles();

  return (
    <div className="memory-browser-container">
      {/* LEFT SIDEBAR: File Tree Panel */}
      <div className="memory-tree-panel">
        <div className="panel-header">
          <h2>Memory Directory</h2>
          <button className="btn-add-file" onClick={() => setIsCreating(true)} title="New File">
            <Plus size={16} />
            <span>New File</span>
          </button>
        </div>

        <div className="search-bar-container">
          <Search size={14} className="search-icon" />
          <input
            type="text"
            placeholder="Search memory files..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
        </div>

        <div className="memory-tree-content">
          {loading ? (
            <div className="panel-loader">
              <Loader2 className="animate-spin" size={24} />
              <span>Indexing repository...</span>
            </div>
          ) : (
            <div className="tree-explorer">
              {Object.keys(fileTree).map(folder => {
                if (folder === '/') return null;
                const isExpanded = expandedFolders[folder];
                return (
                  <div key={folder} className="tree-folder-group">
                    <button className="tree-folder-btn" onClick={() => toggleFolder(folder)}>
                      {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                      <Folder size={14} className="folder-icon" />
                      <span>{folder}/</span>
                    </button>
                    {isExpanded && (
                      <div className="tree-folder-children">
                        {fileTree[folder].map(file => (
                          <button
                            key={file.fullName}
                            className={`tree-file-btn ${selectedPath === file.fullName ? 'active' : ''}`}
                            onClick={() => selectFile(file.fullName)}
                          >
                            <FileText size={13} className="file-icon" />
                            <span>{file.name.replace(/\.md$/, '')}</span>
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                );
              })}

              {/* Root files */}
              {fileTree['/'] && fileTree['/'].map(file => (
                <button
                  key={file.fullName}
                  className={`tree-file-btn root-file ${selectedPath === file.fullName ? 'active' : ''}`}
                  onClick={() => selectFile(file.fullName)}
                >
                  <FileText size={13} className="file-icon" />
                  <span>{file.name.replace(/\.md$/, '')}</span>
                </button>
              ))}

              {files.length === 0 && (
                <div className="tree-empty">No memory files indexed.</div>
              )}
            </div>
          )}
        </div>
      </div>

      {/* MIDDLE & RIGHT PANELS: Workspace */}
      <div className="memory-workspace-panel">
        {selectedPath ? (
          <div className="workspace-inner">
            {/* WORKSPACE HEADER */}
            <div className="workspace-header">
              <div className="file-title-info">
                <FileText size={18} className="title-icon" />
                <div>
                  <span className="file-path">{selectedPath}</span>
                  {selectedFile?.frontmatter?.read_only === 'true' && (
                    <span className="badge-readonly">
                      <Lock size={10} /> READ-ONLY
                    </span>
                  )}
                </div>
              </div>

              <div className="workspace-actions">
                {/* Auto commit toggle */}
                <label className="commit-toggle-label" title="Automatically commit using default message on Save">
                  <input
                    type="checkbox"
                    checked={autoCommit}
                    onChange={(e) => setAutoCommit(e.target.checked)}
                  />
                  <span className="toggle-custom-label">Auto-Commit</span>
                </label>

                <button 
                  className={`btn-save ${hasChanges ? 'glow-accent' : ''}`} 
                  onClick={handleSaveTrigger}
                  disabled={!hasChanges || selectedFile?.frontmatter?.read_only === 'true'}
                >
                  <Save size={14} />
                  <span>{autoCommit ? 'Save & Commit' : 'Save...'}</span>
                </button>

                <button 
                  className="btn-delete" 
                  onClick={handleDelete}
                  disabled={selectedFile?.frontmatter?.read_only === 'true'}
                >
                  <Trash2 size={14} />
                </button>

                <button 
                  className={`btn-toggle-preview ${showPreview ? 'active' : ''}`} 
                  onClick={() => setShowPreview(!showPreview)}
                  title={showPreview ? "Close Preview Panel" : "Open Preview Panel"}
                >
                  {showPreview ? <EyeOff size={16} /> : <Eye size={16} />}
                </button>
              </div>
            </div>

            {/* ERROR / SUCCESS ALERTS */}
            {error && (
              <div className="alert-message error-alert">
                <AlertCircle size={16} />
                <span>{error}</span>
              </div>
            )}
            {successMsg && (
              <div className="alert-message success-alert">
                <Check size={16} />
                <span>{successMsg}</span>
              </div>
            )}

            {/* THREE PANEL GRID LAYOUT */}
            <div className={`workspace-editor-grid ${showPreview ? 'preview-open' : 'preview-closed'}`}>
              
              {/* MIDDLE PANEL: File Editor */}
              <div className="editor-side">
                {fileLoading ? (
                  <div className="editor-loader">
                    <Loader2 className="animate-spin" size={32} />
                    <span>Loading file contents...</span>
                  </div>
                ) : (
                  <div className="editor-content-flow">
                    {/* Frontmatter Editor (Collapsible details) */}
                    <div className="frontmatter-details-card">
                      <div className="details-header">
                        <GitCommit size={14} />
                        <span>YAML Metadata (Frontmatter)</span>
                      </div>
                      <div className="frontmatter-inputs-grid">
                        <div className="fm-input-group">
                          <label>Description</label>
                          <input
                            type="text"
                            value={editedDescription}
                            onChange={(e) => setEditedDescription(e.target.value)}
                            disabled={selectedFile?.frontmatter?.read_only === 'true'}
                            placeholder="Enter file purpose description..."
                          />
                        </div>
                        <div className="fm-input-group">
                          <label>Access Mode</label>
                          <select
                            value={editedReadOnly}
                            onChange={(e) => setEditedReadOnly(e.target.value)}
                            disabled={selectedFile?.frontmatter?.read_only === 'true'}
                          >
                            <option value="false">Writable</option>
                            <option value="true">Read-Only</option>
                          </select>
                        </div>
                        <div className="fm-input-group">
                          <label>Tags (Comma separated)</label>
                          <input
                            type="text"
                            value={editedTags}
                            onChange={(e) => setEditedTags(e.target.value)}
                            disabled={selectedFile?.frontmatter?.read_only === 'true'}
                            placeholder="system, core, journal"
                          />
                        </div>
                        <div className="fm-input-group">
                          <label>Char Limit</label>
                          <input
                            type="number"
                            value={editedLimit}
                            onChange={(e) => setEditedLimit(e.target.value)}
                            disabled={selectedFile?.frontmatter?.read_only === 'true'}
                            placeholder="e.g. 4000"
                          />
                        </div>
                      </div>
                    </div>

                    {/* Monospace Markdown Body Textarea */}
                    <div className="markdown-body-editor">
                      <textarea
                        value={editedBody}
                        onChange={(e) => setEditedBody(e.target.value)}
                        disabled={selectedFile?.frontmatter?.read_only === 'true'}
                        placeholder="# Heading&#10;&#10;Write markdown memory contents here..."
                        className="editor-textarea"
                      />
                    </div>
                  </div>
                )}
              </div>

              {/* RIGHT PANEL: Collapsible HTML Markdown Preview */}
              {showPreview && (
                <div className="preview-side">
                  <div className="preview-header-bar">Live Markdown Render</div>
                  <div className="preview-body-rendered markdown-body-rendered">
                    <ReactMarkdown 
                      remarkPlugins={[remarkGfm]} 
                      rehypePlugins={[rehypeHighlight]}
                    >
                      {editedBody || '*No content written yet. Write content in the editor to see rendered preview.*'}
                    </ReactMarkdown>
                  </div>
                </div>
              )}

            </div>
          </div>
        ) : (
          <div className="workspace-empty">
            <FileText size={48} className="empty-icon" />
            <h3>No File Selected</h3>
            <p>Select a memory file from the explorer on the left or create a new file to start editing.</p>
            <button className="btn-add-file secondary-btn" onClick={() => setIsCreating(true)}>
              <Plus size={16} />
              <span>Create New File</span>
            </button>
          </div>
        )}
      </div>

      {/* MODAL: Git Manual Commit message input */}
      {showCommitModal && (
        <div className="modal-overlay">
          <div className="modal-container">
            <div className="modal-header">
              <GitCommit size={18} />
              <h3>Commit Memory Changes</h3>
            </div>
            <div className="modal-body">
              <p>Enter a Git commit message to describe your edits to the agent's memory repo:</p>
              <input
                type="text"
                value={commitMessage}
                onChange={(e) => setCommitMessage(e.target.value)}
                placeholder="feat(memory): updates"
                className="modal-input"
              />
            </div>
            <div className="modal-actions">
              <button className="btn-secondary" onClick={() => setShowCommitModal(false)}>Cancel</button>
              <button className="btn-primary" onClick={() => handleSave(commitMessage)}>Commit Changes</button>
            </div>
          </div>
        </div>
      )}

      {/* MODAL: New File Dialogue */}
      {isCreating && (
        <div className="modal-overlay">
          <form className="modal-container" onSubmit={handleCreate}>
            <div className="modal-header">
              <Plus size={18} />
              <h3>Create New Memory File</h3>
            </div>
            <div className="modal-body">
              <div className="form-group">
                <label>Folder Partition</label>
                <select
                  value={newFileFolder}
                  onChange={(e) => setNewFileFolder(e.target.value)}
                >
                  <option value="journal">journal/</option>
                  <option value="system">system/</option>
                  <option value="ledger">ledger/</option>
                  <option value="reference">reference/</option>
                </select>
              </div>

              <div className="form-group">
                <label>Filename (without extension)</label>
                <input
                  type="text"
                  required
                  placeholder="e.g. key-discoveries"
                  value={newFilePath}
                  onChange={(e) => setNewFilePath(e.target.value)}
                />
              </div>

              <div className="form-group">
                <label>Description (YAML Frontmatter)</label>
                <input
                  type="text"
                  placeholder="e.g. Critical information regarding X"
                  value={newFileDescription}
                  onChange={(e) => setNewFileDescription(e.target.value)}
                />
              </div>
            </div>
            <div className="modal-actions">
              <button type="button" className="btn-secondary" onClick={() => setIsCreating(false)}>Cancel</button>
              <button type="submit" className="btn-primary">Create Memory</button>
            </div>
          </form>
        </div>
      )}
    </div>
  );
}
