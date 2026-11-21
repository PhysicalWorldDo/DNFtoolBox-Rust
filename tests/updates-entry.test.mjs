import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import * as model from '../frontend/assets/model.mjs';

const source = (await readFile(new URL('../frontend/assets/app.mjs', import.meta.url), 'utf8'))
  .replace(/^import .*\n/gm, '');

async function app(hash = '#home', operations = []) {
  const calls = [];
  const snapshot = {protocol: 1, sessionId: 's-1', seq: 0, catalog: {tools: [], revision: 1, source: 'empty'}, preferences: {pinnedToolIds: []}, settings: {revision: 1, githubProxy: {enabled: false, urls: []}}, operations, toolbox: {version: '0.4.0'}, about: {}, directoryWarning: {}, capabilities: {}};
  const nodes = new Map();
  const node = id => nodes.get(id) || nodes.set(id, {id, dataset: {}, classList: {add() {}, remove() {}}, setAttribute() {}, addEventListener() {}, focus() {}, close() {}, showModal() {}, querySelectorAll() {return [];}}).get(id);
  const location = {hash};
  const listeners = {};
  const bridge = {available: true, listen: async () => () => {}, async dispatch(method, params = {}) {calls.push({method, params}); if (method === 'app.bootstrap') return structuredClone(snapshot); if (method === 'catalog.refresh') return {operationId: 'check-1'}; return {accepted: true};}};
  const context = vm.createContext({...model, createBridge: () => bridge, location, history: {pushState(_state, _title, hash) {location.hash = hash;}}, document: {getElementById: node, activeElement: null, addEventListener() {}}, window: {scrollTo() {}, addEventListener(event, callback) {listeners[event] = callback;}}, requestAnimationFrame(callback) {queueMicrotask(callback);}, setTimeout: () => 1, clearTimeout() {}, console});
  vm.runInContext(source, context);
  const settle = async () => {for (let i = 0; i < 10; i++) await new Promise(resolve => setImmediate(resolve));};
  await settle();
  return {calls, snapshot, context, listeners, settle, navigate(page) {vm.runInContext(`go(${JSON.stringify(page)})`, context);}};
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
