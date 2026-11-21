// The host owns catalog, lifecycle, preferences, and settings. This module only
// holds the last acknowledged snapshot and applies ordered host notifications.
export function createState() {
  return {sessionId: null, seq: 0, catalog: {tools: [], categories: [], source: 'empty', revision: 0}, preferences: {pinnedToolIds: [], lastSelectedToolId: null}, settings: null, operations: [], toolbox: {}, about: {}, directoryWarning: null, capabilities: {}, window: {}};
}

export function applyBootstrap(state, snapshot) {
  const newSession = state.sessionId !== snapshot.sessionId;
  state.sessionId = snapshot.sessionId;
  state.seq = snapshot.seq;
  for (const field of ['catalog', 'settings', 'operations', 'toolbox', 'about', 'directoryWarning', 'capabilities']) {
    if (snapshot[field] !== undefined) state[field] = snapshot[field];
  }
  if (snapshot.preferences !== undefined) {
    if (newSession) state.preferences = snapshot.preferences;
    else applyPreferences(state, snapshot.preferences);
  }
  return state;
}

export function applyPreferences(state, preferences) {
  if ((preferences.revision ?? 0) < (state.preferences.revision ?? 0)) return false;
  state.preferences = preferences;
  return true;
}

export function applyEvent(state, message) {
  if (message.v !== 1 || message.sessionId !== state.sessionId) return {resync: true};
  if (message.seq <= state.seq) return {ignored: true};
  if (message.seq !== state.seq + 1) return {resync: true};
  state.seq = message.seq;
  const fields = {'catalog.changed': 'catalog', 'settings.changed': 'settings', 'toolbox.changed': 'toolbox'};
  if (fields[message.event]) state[fields[message.event]] = message.data;
  if (message.event === 'preferences.changed') applyPreferences(state, message.data);
  if (message.event === 'window.changed') state.window = {...state.window, ...message.data};
  if (message.event === 'operation.changed') {
    const index = state.operations.findIndex(operation => operation.id === message.data.id);
    if (index === -1) state.operations.push(message.data);
    else state.operations[index] = {...state.operations[index], ...message.data};
  }
  if (message.event === 'operation.removed') state.operations = state.operations.filter(operation => operation.id !== message.data.id);
  return {changed: message.event, terminal: message.event === 'operation.changed' && !isActive(message.data)};
}

export function isActive(operation) {
  return ['queued', 'running'].includes(operation.state);
}

export function primaryAction(tool, operations) {
  const operation = operations.find(item => item.toolId === tool.id && isActive(item));
  if (operation) return {action: 'task', label: operation.state === 'queued' ? '排队中' : '处理中', disabled: true, reason: '可在安装与更新中查看进度。'};
  const actions = tool.actions || [];
  if (actions.includes('launch')) return {action: 'launch', label: '启动工具', disabled: false};
  if (actions.includes('install')) return {action: 'install', label: '安装工具', disabled: false};
  if (actions.includes('reinstall') && ['missing', 'recoveryRequired'].includes(tool.entryState)) return {action: 'reinstall', label: '修复安装', disabled: false, version: tool.installedVersion};
  return {action: '', label: tool.entryState === 'recoveryRequired' ? '需要恢复' : tool.entryState === 'missing' ? '入口缺失' : '暂不可用', disabled: true, reason: tool.disabledReason || '当前状态没有可执行的操作。'};
}

export function filterTools(tools, {query = '', category = '全部', filter = 'all'}, pins) {
  const needle = query.trim().toLocaleLowerCase();
  return tools.filter(tool => (category === '全部' || tool.category === category) &&
    (filter === 'all' || ({installed: Boolean(tool.installed), uninstalled: !tool.installed, updates: tool.hasUpdate, pinned: pins.includes(tool.id)})[filter]) &&
    [tool.name, tool.id, tool.category, tool.description].some(value => String(value || '').toLocaleLowerCase().includes(needle)));
}
