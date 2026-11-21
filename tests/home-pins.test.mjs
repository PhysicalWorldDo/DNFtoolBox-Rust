import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import * as model from '../frontend/assets/model.mjs';

const source = (await readFile(new URL('../frontend/assets/app.mjs', import.meta.url), 'utf8')).replace(/^import .*\n/gm, '');
const tool = (id, name, hasUpdate = false) => ({id, name, category: '图像工具', description: `${name}介绍`, installed: true, installedVersion: '1.0', latestVersion: hasUpdate ? '2.0' : '1.0', hasUpdate, actions: ['launch', 'update'], availableVersions: [{version: hasUpdate ? '2.0' : '1.0', date: '2026-11-21', canInstall: true}]});

async function app({pinnedToolIds = ['rep_player', 'dnf_skill_prism', 'dnf_avatar_studio'], lastSelectedToolId = 'dnf_palette_prism', catalog, deferPreferences = false} = {}) {
  const calls = [], nodes = new Map(), listeners = {}, responses = [];
  const snapshot = {sessionId: 'home-session', seq: 0, catalog: {source: 'cached', revision: 1, tools: catalog || [tool('dnf_palette_prism', '阿拉德调色', true), tool('rep_player', 'REP 播放器', true), tool('dnf_skill_prism', '技能染色工坊'), tool('dnf_avatar_studio', '薛定谔的化裁间', true)]}, preferences: {schemaVersion: 1, revision: 1, pinnedToolIds, lastSelectedToolId}, settings: {revision: 1, githubProxy: {enabled: false, urls: []}}, operations: [], toolbox: {}, about: {}, directoryWarning: {}, capabilities: {}};
  const node = id => nodes.get(id) || nodes.set(id, {id, dataset: {}, classList: {add() {}, remove() {}}, setAttribute() {}, addEventListener() {}, focus() {}, close() {this.open = false;}, showModal() {this.open = true;}, querySelectorAll() {return [];}}).get(id);
  const bridge = {available: true, listen: async callback => {listeners.host = callback; return () => {};}, async dispatch(method, params = {}) {
    calls.push({method, params});
    if (method === 'app.bootstrap') return structuredClone(snapshot);
    if (method === 'preferences.setPinned') {
      const ids = snapshot.preferences.pinnedToolIds;
      snapshot.preferences = {...snapshot.preferences, pinnedToolIds: params.pinned ? ids.includes(params.toolId) ? ids : [...ids, params.toolId] : ids.filter(id => id !== params.toolId), revision: snapshot.preferences.revision + 1};
    }
    if (method === 'preferences.selectTool') snapshot.preferences = {...snapshot.preferences, lastSelectedToolId: params.toolId, revision: snapshot.preferences.revision + 1};
    if (method.startsWith('preferences.')) {
      const result = structuredClone(snapshot.preferences);
      if (deferPreferences) await new Promise(resolve => responses.push({method, params, result, resolve}));
      return result;
    }
    return {accepted: true};
  }};
  const location = {hash: '#home'};
  const context = vm.createContext({...model, createBridge: () => bridge, location, history: {pushState(_state, _title, hash) {location.hash = hash;}}, document: {getElementById: node, activeElement: null, addEventListener() {}}, window: {scrollY: 0, scrollTo() {}, addEventListener() {}}, requestAnimationFrame(callback) {queueMicrotask(callback);}, setTimeout: () => 1, clearTimeout() {}, console});
  vm.runInContext(source, context);
  const settle = async () => {for (let i = 0; i < 10; i++) await new Promise(resolve => setImmediate(resolve));};
  const evaluate = script => vm.runInContext(script, context);
  await settle();
  return {calls, node, snapshot, responses, settle, evaluate, emit: listeners.host, html: () => node('surface').innerHTML};
}

test('old unpinned selection never enters the home hero, slides, cards, or release news', async () => {
  const ui = await app();
  assert.equal(ui.evaluate('selectedTool().id'), 'rep_player');
  assert.deepEqual(Array.from(ui.evaluate('featureTools().map(tool => tool.id)')), ['rep_player', 'dnf_skill_prism', 'dnf_avatar_studio']);
  assert.doesNotMatch(ui.html(), /阿拉德调色|data-id="dnf_palette_prism"/);
  assert.match(ui.html(), /<strong>01<\/strong> \/ 03/);
  assert.equal((ui.html().match(/class="pin-card/g) || []).length, 3);
  assert.equal((ui.html().match(/class="slide /g) || []).length, 3);
  assert.match(ui.html(), /class="pin-card[^>]*aria-pressed="true"[^>]*data-id="rep_player"/);
});

test('fixed list keeps pin order, omits stale and duplicate IDs, and leaves stored pins untouched', async () => {
  const ids = ['retired-tool', 'dnf_avatar_studio', 'rep_player', 'dnf_avatar_studio'];
  const ui = await app({pinnedToolIds: ids, lastSelectedToolId: 'retired-tool'});
  assert.deepEqual(Array.from(ui.evaluate('featureTools().map(tool => tool.id)')), ['dnf_avatar_studio', 'rep_player']);
  assert.equal(ui.evaluate('selectedTool().id'), 'dnf_avatar_studio');
  assert.deepEqual(ui.snapshot.preferences.pinnedToolIds, ids);
  assert.equal(ui.calls.filter(call => call.method.startsWith('preferences.')).length, 0);
});

test('selecting, unpinning the current tool, and repinning keep hero and pagination in sync', async () => {
  const ui = await app();
  await ui.evaluate('focusTool("dnf_skill_prism")');
  await ui.settle();
  assert.equal(ui.evaluate('selectedTool().id'), 'dnf_skill_prism');
  assert.match(ui.html(), /<strong>02<\/strong> \/ 03/);
  assert.equal(ui.calls.find(call => call.method === 'preferences.selectTool').params.toolId, 'dnf_skill_prism');
  await ui.evaluate('pinTool("dnf_skill_prism")');
  await ui.settle();
  assert.equal(ui.evaluate('selectedTool().id'), 'rep_player');
  assert.match(ui.html(), /<strong>01<\/strong> \/ 02/);
  await ui.evaluate('pinTool("dnf_skill_prism")');
  await ui.settle();
  assert.equal(ui.evaluate('selectedTool().id'), 'rep_player', 'repinning must not revive the removed selection');
});

test('unpinning the last tool displays an actionable empty home and preserves the full library and updates', async () => {
  const ui = await app({pinnedToolIds: ['rep_player'], lastSelectedToolId: 'rep_player'});
  await ui.evaluate('pinTool("rep_player")');
  await ui.settle();
  assert.equal(ui.evaluate('selectedTool()?.id'), undefined);
  assert.match(ui.html(), /还没有固定工具/);
  assert.match(ui.html(), /data-page="library"/);
  assert.doesNotMatch(ui.html(), /class="hero |class="slide |class="pin-card|data-act="primary"/);
  ui.evaluate('go("library")');
  assert.match(ui.html(), /阿拉德调色/);
  assert.equal((ui.html().match(/class="library-row"/g) || []).length, 4);
  ui.evaluate('go("updates")');
  assert.match(ui.html(), /阿拉德调色/);
});

test('an unpinned tool cannot be focused through a normal detail or more entry', async () => {
  const ui = await app();
  ui.evaluate('go("library"); openModal("more", {id: "dnf_palette_prism"})');
  assert.doesNotMatch(ui.node('dialog-body').innerHTML, /data-act="focus"/);
  assert.match(ui.node('dialog-body').innerHTML, />固定到首页<\/button>/);
  await ui.evaluate('focusTool("dnf_palette_prism")');
  assert.equal(ui.evaluate('ui.page'), 'library');
  assert.equal(ui.calls.filter(call => call.method === 'preferences.selectTool').length, 0);
  ui.evaluate('openModal("detail", {id: "rep_player"})');
  assert.match(ui.node('dialog-body').innerHTML, />在首页查看<\/button>/);
  assert.match(ui.node('dialog-body').innerHTML, />取消首页固定<\/button>/);
  assert.doesNotMatch(ui.node('dialog-body').innerHTML, /工作台/);
});

test('home responds to host pin changes and an empty or retired-only preference after restart', async () => {
  const ui = await app();
  ui.emit({v: 1, sessionId: 'home-session', seq: 1, event: 'preferences.changed', data: {revision: 2, pinnedToolIds: ['dnf_avatar_studio'], lastSelectedToolId: 'dnf_palette_prism'}});
  await ui.settle();
  assert.equal(ui.evaluate('selectedTool().id'), 'dnf_avatar_studio');
  assert.match(ui.html(), /<strong>01<\/strong> \/ 01/);
  assert.doesNotMatch(ui.html(), /data-id="rep_player"/);
  for (const pinnedToolIds of [[], ['retired-tool']]) {
    const restarted = await app({pinnedToolIds});
    assert.match(restarted.html(), /还没有固定工具/);
    assert.doesNotMatch(restarted.html(), /阿拉德调色|class="hero /);
  }
});

test('the two registered tools use their matching illustration in both the hero and fixed cards', async () => {
  const ui = await app();
  for (const [id, file] of [['dnf_avatar_studio', 'tailoring-atelier.webp'], ['dnf_skill_prism', 'skill-dye-workshop.webp']]) {
    await ui.evaluate(`focusTool(${JSON.stringify(id)})`);
    assert.match(ui.html(), new RegExp(`class="hero-img" src="assets/${file.replaceAll('.', '\\.')}"`));
    assert.match(ui.html(), new RegExp(`class="pin-art-img" src="assets/${file.replaceAll('.', '\\.')}"`));
  }
});

test('a full pinned catalog uses previous and next controls that reach every tool', async () => {
  const catalog = Array.from({length: 23}, (_value, index) => tool(`tool-${index + 1}`, `工具${index + 1}`));
  const ui = await app({catalog, pinnedToolIds: catalog.map(item => item.id), lastSelectedToolId: null});
  assert.match(ui.html(), /<strong>01<\/strong> \/ 23/);
  assert.equal((ui.html().match(/class="slide-arrow/g) || []).length, 2);
  assert.equal((ui.html().match(/class="slide /g) || []).length, 0);
  assert.match(ui.html(), /class="slide-arrow previous"[^>]*data-id="tool-23"/);
  for (const item of catalog) await ui.evaluate(`focusTool(${JSON.stringify(item.id)})`);
  assert.match(ui.html(), /<strong>23<\/strong> \/ 23/);
  assert.match(ui.html(), /class="slide-arrow next"[^>]*data-id="tool-1"/);
});

test('late pin responses and preference events cannot restore an older fixed list', async () => {
  const ui = await app({deferPreferences: true});
  const first = ui.evaluate('pinTool("dnf_palette_prism")');
  const second = ui.evaluate('pinTool("rep_player")');
  const [older, newer] = ui.responses;
  newer.resolve();
  await second;
  ui.emit({v: 1, sessionId: 'home-session', seq: 1, event: 'preferences.changed', data: older.result});
  older.resolve();
  await first;
  await ui.settle();
  assert.deepEqual(Array.from(ui.evaluate('pins()')), newer.result.pinnedToolIds);
  assert.equal(ui.evaluate('state.preferences.revision'), newer.result.revision);
  assert.equal(ui.evaluate('state.seq'), 1, 'ignoring old preference content must still advance event sequence');
});

test('a response from the previous host session cannot overwrite freshly loaded preferences', async () => {
  const ui = await app({deferPreferences: true});
  const pending = ui.evaluate('focusTool("dnf_skill_prism")');
  ui.snapshot.sessionId = 'restarted-host';
  ui.snapshot.preferences = {revision: 1, pinnedToolIds: ['dnf_avatar_studio'], lastSelectedToolId: 'dnf_avatar_studio'};
  await ui.evaluate('refreshSnapshot()');
  ui.responses[0].resolve();
  await pending;
  await ui.settle();
  assert.deepEqual(Array.from(ui.evaluate('pins()')), ['dnf_avatar_studio']);
  assert.equal(ui.evaluate('state.preferences.lastSelectedToolId'), 'dnf_avatar_studio');
});
