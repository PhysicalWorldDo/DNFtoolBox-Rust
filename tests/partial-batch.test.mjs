import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import * as model from '../frontend/assets/model.mjs';

const source = (await readFile(new URL('../frontend/assets/app.mjs', import.meta.url), 'utf8')).replace(/^import .*\n/gm, '');
// This is the minimized capture from .work/qa/partial-batch.event.json.
const capture = JSON.parse(await readFile(new URL('./fixtures/partial-batch.event.json', import.meta.url), 'utf8'));

async function app(operation) {
  const calls = [];
  const snapshot = {protocol: 1, sessionId: 'partial-session', seq: 1, catalog: {tools: [], revision: 1, source: 'cached'}, preferences: {pinnedToolIds: []}, settings: {revision: 1, githubProxy: {enabled: false, urls: []}}, operations: [operation], toolbox: {version: '0.4.0'}, about: {}, directoryWarning: {}, capabilities: {}};
  const nodes = new Map();
  const node = id => nodes.get(id) || nodes.set(id, {id, dataset: {}, classList: {add() {}, remove() {}}, setAttribute() {}, addEventListener() {}, focus() {}, close() {}, showModal() {}, querySelectorAll() {return [];}}).get(id);
  const bridge = {available: true, listen: async () => () => {}, async dispatch(method, params = {}) {calls.push({method, params}); if (method === 'app.bootstrap') return structuredClone(snapshot); return {operationId: 'retry-failed-items'};}};
  const context = vm.createContext({...model, createBridge: () => bridge, location: {hash: '#home'}, history: {pushState() {}}, document: {getElementById: node, activeElement: null, addEventListener() {}}, window: {scrollTo() {}, addEventListener() {}}, requestAnimationFrame(callback) {queueMicrotask(callback);}, setTimeout: () => 1, clearTimeout() {}, console});
  vm.runInContext(source, context);
  for (let i = 0; i < 10; i++) await new Promise(resolve => setImmediate(resolve));
  return {calls, context, row() {return vm.runInContext('operationRow(state.operations[0])', context);}};
}

test('captured partial batch displays its own name, partial result and a failed-items retry', async () => {
  const ui = await app(capture.data);
  const row = ui.row();
  assert.match(row, /<strong>批量更新<\/strong>/);
  assert.doesNotMatch(row, /工具箱本体/);
  assert.match(row, /部分完成/);
  assert.match(row, /data-act="retry-task"[^>]*>重试失败项<\/button>/);
  assert.match(row, /受控批量验收：第二项哈希校验失败/);
  await vm.runInContext(`handleAction({dataset:{act:'retry-task',id:'qa-partial-batch'}})`, ui.context);
  assert.deepEqual(ui.calls.filter(call => call.method === 'operation.retry').map(call => JSON.parse(JSON.stringify(call.params))), [{operationId: 'qa-partial-batch'}]);
  assert.equal(ui.calls.some(call => call.method === 'operation.start'), false);
});

test('a fully completed batch has no failed-items retry', async () => {
  const complete = structuredClone(capture.data);
  complete.result.status = 'complete';
  complete.children.forEach(child => {child.state = 'succeeded'; child.error = null;});
  const row = (await app(complete)).row();
  assert.match(row, /<strong>批量更新<\/strong>/);
  assert.doesNotMatch(row, /部分完成|重试失败项|data-act="retry-task"/);
});
