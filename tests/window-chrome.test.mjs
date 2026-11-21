import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import * as model from '../frontend/assets/model.mjs';

const source = (await readFile(new URL('../frontend/assets/app.mjs', import.meta.url), 'utf8')).replace(/^import .*\r?\n/gm, '');

async function app({busy = false, stateEventDuringQuery = false} = {}) {
  const calls = [], listeners = {}, nodes = new Map();
  const snapshot = {protocol: 1, sessionId: 'window-session', seq: 0, catalog: {tools: [], revision: 1, source: 'empty'}, preferences: {pinnedToolIds: []}, settings: {revision: 1, githubProxy: {enabled: false, urls: []}}, operations: busy ? [{id: 'active-install', state: 'running', canCancel: true}] : [], toolbox: {version: '0.4.0'}, about: {}, directoryWarning: {}, capabilities: {}};
  const node = id => nodes.get(id) || nodes.set(id, {id, dataset: {}, attributes: {}, classList: {add() {}, remove() {}}, setAttribute(name, value) {this.attributes[name] = String(value);}, addEventListener() {}, focus() {}, close() {this.open = false;}, showModal() {this.open = true;}, querySelectorAll() {return [];}}).get(id);
  const bridge = {available: true, listen: async callback => {listeners.host = callback; return () => {};}, async dispatch(method, params = {}) {
    calls.push({method, params});
    if (method === 'app.bootstrap') return structuredClone(snapshot);
    if (method === 'window.state') {
      if (stateEventDuringQuery) listeners.host({v: 1, sessionId: 'window-session', seq: 1, event: 'window.changed', data: {maximized: false, active: true}});
      return {maximized: true, active: false};
    }
    if (method === 'window.toggleMaximize') return {accepted: true, maximized: false};
    if (method === 'window.requestClose' && busy && !params.cancelTasks) throw Object.assign(new Error('有任务正在进行'), {code: 'BUSY'});
    return {accepted: true};
  }};
  const windowListeners = {}, appWindow = {scrollY: 0, scrollTo() {}, addEventListener(name, listener) {windowListeners[name] = listener;}};
  const context = vm.createContext({...model, createBridge: () => bridge, location: {hash: '#home'}, history: {pushState() {}}, document: {getElementById: node, activeElement: null, addEventListener(name, listener) {(listeners[name] ||= []).push(listener);}}, window: appWindow, requestAnimationFrame(callback) {queueMicrotask(callback);}, setTimeout: () => 1, clearTimeout() {}, console});
  vm.runInContext(source, context);
  const settle = async () => {for (let i = 0; i < 10; i++) await new Promise(resolve => setImmediate(resolve));};
  await settle();
  return {calls, context, node, settle, emit: listeners.host, scroll(y) {appWindow.scrollY = y; windowListeners.scroll?.();}, click(action) {for (const listener of listeners.click || []) listener({target: {closest: () => ({dataset: {window: action}, hasAttribute: name => name === 'data-window'})}});}};
}

test('window notifications preserve maximize and focus state across sparse messages', () => {
  const state = model.createState();
  model.applyBootstrap(state, {sessionId: 'window-session', seq: 0});
  model.applyEvent(state, {v: 1, sessionId: 'window-session', seq: 1, event: 'window.changed', data: {maximized: true, active: true}});
  model.applyEvent(state, {v: 1, sessionId: 'window-session', seq: 2, event: 'window.changed', data: {active: false}});
  model.applyEvent(state, {v: 1, sessionId: 'window-session', seq: 3, event: 'window.changed', data: {closeRequested: true}});
  assert.equal(state.window.maximized, true);
  assert.equal(state.window.active, false);
});

test('startup and native window events synchronize the accessible maximize action', async () => {
  const ui = await app();
  assert.equal(ui.calls.filter(call => call.method === 'window.state').length, 1);
  assert.equal(ui.node('window-maximize').attributes['aria-label'], '还原窗口');
  assert.equal(ui.node('app-shell').dataset.windowActive, 'false');
  ui.emit({v: 1, sessionId: 'window-session', seq: 1, event: 'window.changed', data: {maximized: false, active: true}});
  await ui.settle();
  assert.equal(ui.node('window-maximize').attributes['aria-label'], '最大化窗口');
  assert.equal(ui.node('app-shell').dataset.windowActive, 'true');
});

test('window buttons dispatch one semantic action and use the acknowledged maximize state', async () => {
  const ui = await app();
  ui.click('minimize'); ui.click('toggleMaximize');
  await ui.settle();
  assert.equal(ui.calls.filter(call => call.method === 'window.minimize').length, 1);
  assert.equal(ui.calls.filter(call => call.method === 'window.toggleMaximize').length, 1);
  assert.equal(ui.node('window-maximize').attributes['aria-label'], '最大化窗口');
});

test('native resize and focus notifications retain the current editable surface', async () => {
  const ui = await app();
  ui.node('surface').innerHTML = 'current form DOM and selection';
  ui.emit({v: 1, sessionId: 'window-session', seq: 1, event: 'window.changed', data: {active: true}});
  await ui.settle();
  assert.equal(ui.node('surface').innerHTML, 'current form DOM and selection');
});

test('a native notification received during the initial state query takes precedence', async () => {
  const ui = await app({stateEventDuringQuery: true});
  assert.equal(ui.node('window-maximize').attributes['aria-label'], '最大化窗口');
  assert.equal(ui.node('app-shell').dataset.windowActive, 'true');
});

test('scrolling changes only the title contrast state and restores the transparent home state at the top', async () => {
  const ui = await app();
  assert.equal(ui.node('app-shell').dataset.scrolled, 'false');
  ui.node('surface').innerHTML = 'current editable surface';
  ui.scroll(440);
  await ui.settle();
  assert.equal(ui.node('app-shell').dataset.scrolled, 'true');
  assert.equal(ui.node('surface').innerHTML, 'current editable surface');
  ui.scroll(0);
  await ui.settle();
  assert.equal(ui.node('app-shell').dataset.scrolled, 'false');
  assert.equal(ui.node('surface').innerHTML, 'current editable surface');
});

test('close buttons retain task confirmation and send cancelTasks only after confirmation', async () => {
  const ui = await app({busy: true});
  ui.click('requestClose');
  await ui.settle();
  assert.equal(ui.node('dialog').open, true);
  assert.match(ui.node('dialog-body').innerHTML, /取消任务并退出/);
  assert.deepEqual(ui.calls.filter(call => call.method === 'window.requestClose').map(call => JSON.parse(JSON.stringify(call.params))), [{}]);
  await vm.runInContext("handleAction({dataset:{act:'cancel-close'}})", ui.context);
  assert.deepEqual(ui.calls.filter(call => call.method === 'window.requestClose').map(call => JSON.parse(JSON.stringify(call.params))), [{}, {cancelTasks: true}]);
});
