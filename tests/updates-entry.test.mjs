import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import * as model from '../frontend/assets/model.mjs';

const source = (await readFile(new URL('../frontend/assets/app.mjs', import.meta.url), 'utf8'))
  .replace(/^import .*\r?\n/gm, '');

async function app(hash = '#home', operations = []) {
  const calls = [];
  const snapshot = {protocol: 1, sessionId: 's-1', seq: 0, catalog: {tools: [], revision: 1, source: 'empty'}, preferences: {pinnedToolIds: []}, settings: {revision: 1, githubProxy: {enabled: false, urls: []}}, operations, toolbox: {version: '0.4.0'}, about: {}, directoryWarning: {}, capabilities: {}};
  const nodes = new Map();
  const node = id => nodes.get(id) || nodes.set(id, {id, dataset: {}, classList: {add() {}, remove() {}}, setAttribute() {}, addEventListener() {}, focus() {}, close() {}, showModal() {}, querySelectorAll() {return [];}}).get(id);
  const location = {hash};
  const listeners = {};
  const bridge = {available: true, listen: async () => () => {}, async dispatch(method, params = {}) {
    calls.push({method, params});
    if (method === 'app.bootstrap') return structuredClone(snapshot);
    if (method === 'catalog.refresh') return {operationId: 'check-1'};
    if (method === 'toolbox.checkUpdate') {
      snapshot.toolbox = {version: '0.4.0', checked: true, latestVersion: '0.4.2', hasUpdate: true, canDownload: true, changelog: ['发布说明']};
      return snapshot.toolbox;
    }
    if (method === 'operation.delete') snapshot.operations = snapshot.operations.filter(operation => operation.id !== params.operationId);
    return {accepted: true};
  }};
  const context = vm.createContext({...model, createBridge: () => bridge, location, history: {pushState(_state, _title, hash) {location.hash = hash;}}, document: {getElementById: node, activeElement: null, addEventListener() {}}, window: {scrollTo() {}, addEventListener(event, callback) {listeners[event] = callback;}}, requestAnimationFrame(callback) {queueMicrotask(callback);}, setTimeout: () => 1, clearTimeout() {}, console});
  vm.runInContext(source, context);
  const settle = async () => {for (let i = 0; i < 10; i++) await new Promise(resolve => setImmediate(resolve));};
  await settle();
  return {
    calls, snapshot, context, listeners, settle, node,
    navigate(page) {vm.runInContext(`go(${JSON.stringify(page)})`, context);},
    action(act, id = '') {return vm.runInContext('handleAction(' + JSON.stringify({dataset: {act, id}}) + ')', context);}
  };
}

test('the home page stays offline until updates is first entered, even with an empty catalog', async () => {
  const ui = await app();
  assert.equal(ui.calls.filter(call => call.method === 'catalog.refresh').length, 0);
  ui.navigate('updates');
  await ui.settle();
  assert.deepEqual(ui.calls.filter(call => call.method === 'catalog.refresh').map(call => call.params.reason), ['updatesPage']);
  ui.navigate('home'); ui.navigate('updates');
  await ui.settle();
  assert.equal(ui.calls.filter(call => call.method === 'catalog.refresh').length, 1);
});

test('direct updates startup checks once and bootstrap rerenders do not loop', async () => {
  const ui = await app('#updates');
  await vm.runInContext('refreshSnapshot()', ui.context);
  await ui.settle();
  assert.equal(ui.calls.filter(call => call.method === 'catalog.refresh').length, 1);
  assert.equal(ui.calls.find(call => call.method === 'catalog.refresh').params.reason, 'updatesPage');
});

test('a manual check or an existing session check suppresses the automatic request', async () => {
  const manual = await app();
  await vm.runInContext('refreshCatalog()', manual.context);
  manual.navigate('updates');
  await manual.settle();
  assert.equal(manual.calls.filter(call => call.method === 'catalog.refresh').length, 1);
  const existing = await app('#updates', [{id: 'check-1', kind: 'refresh', state: 'running', request: {sessionId: 's-1'}}]);
  assert.equal(existing.calls.filter(call => call.method === 'catalog.refresh').length, 0);
});

test('a persisted refresh from an earlier process does not suppress this session check', async () => {
  const ui = await app('#updates', [{id: 'old-check', kind: 'refresh', state: 'failed', request: {sessionId: 'old-session'}}]);
  assert.equal(ui.calls.filter(call => call.method === 'catalog.refresh').length, 1);
});

test('a new host session gets its own first-entry check', async () => {
  const ui = await app('#updates');
  ui.snapshot.sessionId = 's-2';
  await vm.runInContext('refreshSnapshot()', ui.context);
  await ui.settle();
  assert.equal(ui.calls.filter(call => call.method === 'catalog.refresh').length, 2);
});

test('checking the toolbox uses GitHub release metadata independently of the tool catalog', async () => {
  const ui = await app('#settings');
  await vm.runInContext(`handleAction({dataset: {act: 'self-check'}})`, ui.context);
  await ui.settle();
  assert.equal(ui.calls.filter(call => call.method === 'toolbox.checkUpdate').length, 1);
  assert.equal(ui.calls.filter(call => call.method === 'catalog.refresh').length, 0);
  assert.match(vm.runInContext('selfUpdateBody()', ui.context), /下载 v0\.4\.2/);
});

test('a staged update can be applied after an offline restart', async () => {
  const ui = await app('#settings', [{id: 'prepared', kind: 'toolboxUpdate', version: '0.4.2', state: 'succeeded', result: {readyToApply: true}}]);
  assert.match(vm.runInContext('selfUpdateBody()', ui.context), /应用更新并重启/);
  await ui.action('apply-self', 'prepared');
  await ui.action('confirm-apply-self', 'prepared');
  assert.deepEqual(JSON.parse(JSON.stringify(ui.calls.find(call => call.method === 'toolbox.applyUpdate').params)), {operationId: 'prepared', confirmed: true});
});

test('the updates page keeps the detected version and download button visible', async () => {
  const ui = await app('#updates');
  await ui.action('self-check');
  const page = vm.runInContext('renderUpdates()', ui.context);
  assert.match(page, /发现新版 v0\.4\.2/);
  assert.match(page, /data-act="download-self"[^>]*>下载 v0\.4\.2<\/button>/);
  assert.equal(vm.runInContext('ui.modal', ui.context), null);
  assert.equal(ui.calls.filter(call => call.method === 'catalog.refresh').length, 1);
});

test('a finished download offers an apply confirmation and sends only the confirmed request', async () => {
  const ui = await app('#updates');
  await ui.action('self-check');
  await ui.action('download-self');
  ui.snapshot.operations.push({id: 'prepared', kind: 'toolboxUpdate', version: '0.4.2', state: 'succeeded', result: {readyToApply: true}});
  await vm.runInContext('refreshSnapshot()', ui.context);
  assert.match(vm.runInContext('renderUpdates()', ui.context), /等待确认/);
  await ui.action('confirm-apply-self', 'prepared');
  assert.equal(ui.calls.some(call => call.method === 'toolbox.applyUpdate'), false);
  await ui.action('apply-self', 'prepared');
  assert.match(ui.node('dialog-body').innerHTML, /v0\.4\.2.*备份并替换主程序/);
  assert.equal(ui.calls.some(call => call.method === 'toolbox.applyUpdate'), false);
  await ui.action('close');
  await ui.action('apply-self', 'prepared');
  await ui.action('confirm-apply-self', 'prepared');
  assert.deepEqual(ui.calls.filter(call => call.method === 'toolbox.applyUpdate').map(call => JSON.parse(JSON.stringify(call.params))), [{operationId: 'prepared', confirmed: true}]);
});

test('the refresh task is labeled as a tool directory refresh', async () => {
  const ui = await app('#updates', [{id: 'refresh-1', kind: 'refresh', state: 'succeeded'}]);
  const row = vm.runInContext('operationRow(state.operations[0])', ui.context);
  assert.match(row, /<strong>工具目录刷新<\/strong>/);
  assert.doesNotMatch(row, /工具箱本体|>refresh</);
});

test('deleting a finished task first asks for confirmation and removes only that row', async () => {
  const ui = await app('#updates', [
    {id: 'finished', kind: 'install', state: 'failed', error: {message: '下载失败'}},
    {id: 'working', kind: 'install', state: 'running'}
  ]);
  assert.match(vm.runInContext('operationRow(state.operations[0])', ui.context), /删除记录/);
  assert.doesNotMatch(vm.runInContext('operationRow(state.operations[1])', ui.context), /删除记录/);
  await ui.action('open-log', 'finished');
  assert.match(ui.node('dialog-body').innerHTML, /下载失败/);
  await ui.action('close');
  await ui.action('confirm-delete-task', 'finished');
  assert.equal(ui.calls.some(call => call.method === 'operation.delete'), false);
  await ui.action('delete-task', 'finished');
  assert.match(ui.node('dialog-body').innerHTML, /删除.*任务记录/);
  assert.equal(ui.calls.some(call => call.method === 'operation.delete'), false);
  await ui.action('confirm-delete-task', 'finished');
  assert.deepEqual(ui.calls.filter(call => call.method === 'operation.delete').map(call => JSON.parse(JSON.stringify(call.params))), [{operationId: 'finished', confirmed: true}]);
  assert.equal(vm.runInContext('state.operations.map(operation => operation.id).join()', ui.context), 'working');
});

test('a blocked exit names local file writes and recovery', async () => {
  const ui = await app('#updates', [{id: 'working', kind: 'install', state: 'running', stage: 'commit', canCancel: false}]);
  const row = vm.runInContext('operationRow(state.operations[0])', ui.context);
  assert.match(row, /写入本地文件/);
  assert.doesNotMatch(row, /提交/);
  vm.runInContext("openModal('busy-close')", ui.context);
  assert.match(ui.node('dialog-body').innerHTML, /正在写入或恢复本地文件/);
});
