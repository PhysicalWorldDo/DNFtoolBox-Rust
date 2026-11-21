import test from 'node:test';
import assert from 'node:assert/strict';

let model;
let bridge;
try { model = await import('../frontend/assets/model.mjs'); } catch {}
try { bridge = await import('../frontend/assets/bridge.mjs'); } catch {}

const snapshot = () => ({
  protocol: 1, sessionId: 'test-session', seq: 4,
  catalog: {revision: 7, source: 'cached', tools: [], categories: []},
  preferences: {pinnedToolIds: [], lastSelectedToolId: null, revision: 1},
  settings: {revision: 2}, operations: [], toolbox: {version: '0.4.0'},
  about: {}, directoryWarning: {show: false}, capabilities: {launch: true}
});

test('bootstrap uses host snapshot without inventing tools, tasks, or pins', () => {
  assert.ok(model, 'production snapshot model must exist');
  const state = model.createState();
  model.applyBootstrap(state, snapshot());
  assert.deepEqual(state.catalog.tools, []);
  assert.deepEqual(state.operations, []);
  assert.deepEqual(state.preferences.pinnedToolIds, []);
  assert.equal(state.sessionId, 'test-session');
  assert.equal(state.seq, 4);
});

test('events reject stale messages and resync when session or sequence changes', () => {
  assert.ok(model, 'production snapshot model must exist');
  const state = model.createState();
  model.applyBootstrap(state, snapshot());
  assert.equal(model.applyEvent(state, {v: 1, sessionId: 'test-session', seq: 4, event: 'settings.changed', data: {revision: 8}}).ignored, true);
  assert.equal(state.settings.revision, 2);
  assert.equal(model.applyEvent(state, {v: 1, sessionId: 'test-session', seq: 6, event: 'settings.changed', data: {revision: 8}}).resync, true);
  assert.equal(state.settings.revision, 2);
  assert.equal(model.applyEvent(state, {v: 1, sessionId: 'new-session', seq: 1, event: 'settings.changed', data: {}}).resync, true);
});

test('operation events replace only their own task and preserve host progress', () => {
  assert.ok(model, 'production snapshot model must exist');
  const state = model.createState();
  model.applyBootstrap(state, snapshot());
  model.applyEvent(state, {v: 1, sessionId: 'test-session', seq: 5, event: 'operation.changed', data: {id: 'op-a', state: 'running', stage: 'download', doneBytes: 6, totalBytes: null, canCancel: true}});
  model.applyEvent(state, {v: 1, sessionId: 'test-session', seq: 6, event: 'operation.changed', data: {id: 'op-b', state: 'queued', doneBytes: 0, totalBytes: 10}});
  model.applyEvent(state, {v: 1, sessionId: 'test-session', seq: 7, event: 'operation.changed', data: {id: 'op-a', state: 'failed', error: {code: 'NETWORK', retryable: true}}});
  assert.equal(state.operations.length, 2);
  assert.equal(state.operations.find(x => x.id === 'op-a').state, 'failed');
  assert.equal(state.operations.find(x => x.id === 'op-b').state, 'queued');
});

test('same-session bootstrap keeps newer preferences while a new session can restart its revision', () => {
  const state = model.createState();
  model.applyBootstrap(state, snapshot());
  model.applyEvent(state, {v: 1, sessionId: 'test-session', seq: 5, event: 'preferences.changed', data: {pinnedToolIds: ['new-pin'], revision: 3}});
  model.applyBootstrap(state, snapshot());
  assert.deepEqual(state.preferences.pinnedToolIds, ['new-pin']);
  model.applyBootstrap(state, {...snapshot(), sessionId: 'new-session'});
  assert.deepEqual(state.preferences.pinnedToolIds, []);
  assert.equal(state.preferences.revision, 1);
});

test('available primary actions come from host actions and active operations', () => {
  assert.ok(model, 'production snapshot model must exist');
  const tool = {id: 'real-tool', installedVersion: '1.0', entryState: 'ready', actions: ['launch'], disabledReason: null};
  assert.equal(model.primaryAction(tool, []).action, 'launch');
  assert.equal(model.primaryAction({...tool, actions: [], disabledReason: '需要恢复'}, []).disabled, true);
  assert.equal(model.primaryAction(tool, [{id: 'op', toolId: tool.id, state: 'running', stage: 'commit'}]).disabled, true);
  assert.equal(model.primaryAction({...tool, installedVersion: null, actions: ['install']}, []).action, 'install');
  assert.equal(model.primaryAction({...tool, entryState: 'missing', actions: ['reinstall'], latestVersion: '2.0'}, []).version, '1.0', 'repair must reinstall the installed version');
});

test('library filters use real installed/update/pin data', () => {
  assert.ok(model, 'production snapshot model must exist');
  const tools = [{id: 'a', name: '中文工具', description: '录像', category: '资源', installed: true, hasUpdate: true}, {id: 'b', name: 'Other', category: '编辑', installed: false, hasUpdate: false}];
  assert.deepEqual(model.filterTools(tools, {query: '录像', category: '全部', filter: 'updates'}, []).map(x => x.id), ['a']);
  assert.deepEqual(model.filterTools(tools, {query: '', category: '全部', filter: 'pinned'}, ['b']).map(x => x.id), ['b']);
});

test('bridge sends versioned dispatcher envelope and subscribes to main window', async () => {
  assert.ok(bridge, 'production Tauri bridge must exist');
  const calls = [];
  const native = {core: {invoke: async (command, args) => {calls.push({command, args}); return {v: 1, id: args.request.id, ok: true, result: {accepted: true}};}}, event: {listen: async (...args) => {calls.push({listen: args}); return () => {};}}};
  const client = bridge.createBridge(native);
  assert.deepEqual(await client.dispatch('tool.launch', {toolId: 'real-tool'}), {accepted: true});
  assert.equal(calls[0].command, 'toolbox_dispatch');
  assert.equal(calls[0].args.request.v, 1);
  assert.equal(calls[0].args.request.method, 'tool.launch');
  assert.deepEqual(calls[0].args.request.params, {toolId: 'real-tool'});
  await client.listen(() => {});
  assert.equal(calls[1].listen[0], 'toolbox://event');
  assert.deepEqual(calls[1].listen[2], {target: 'main'});
});

test('bridge preserves business errors and reports absent native host', async () => {
  assert.ok(bridge, 'production Tauri bridge must exist');
  const native = {core: {invoke: async (_name, {request}) => ({v: 1, id: request.id, ok: false, error: {code: 'FILE_IN_USE', message: '请关闭工具后重试', retryable: true}})}};
  await assert.rejects(bridge.createBridge(native).dispatch('operation.start', {}), error => error.code === 'FILE_IN_USE' && error.retryable === true);
  await assert.rejects(bridge.createBridge(null).dispatch('app.bootstrap'), error => error.code === 'HOST_UNAVAILABLE');
});
