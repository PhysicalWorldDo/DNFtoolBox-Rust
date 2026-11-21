import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import * as model from '../frontend/assets/model.mjs';

const source = (await readFile(new URL('../frontend/assets/app.mjs', import.meta.url), 'utf8')).replace(/^import .*\r?\n/gm, '');
const tool = (id, name, hasUpdate = false) => ({id, name, category: '图像工具', description: `${name}介绍`, installed: true, installedVersion: '1.0', latestVersion: hasUpdate ? '2.0' : '1.0', hasUpdate, actions: ['launch', 'update'], availableVersions: [{version: hasUpdate ? '2.0' : '1.0', date: '2026-11-21', canInstall: true}]});

async function app({pinnedToolIds = ['rep_player', 'dnf_skill_prism', 'dnf_avatar_studio'], lastSelectedToolId = 'dnf_palette_prism', catalog, deferPreferences = false, deferLaunch = false, failSelectionOnce = false, operations = []} = {}) {
  const calls = [], nodes = new Map(), listeners = {}, responses = [], launches = [], scrolls = [], timers = [];
  let selectionFailed = false;
  const snapshot = {sessionId: 'home-session', seq: 0, catalog: {source: 'cached', revision: 1, tools: catalog || [tool('dnf_palette_prism', '阿拉德调色', true), tool('rep_player', 'REP 播放器', true), tool('dnf_skill_prism', '技能染色工坊'), tool('dnf_avatar_studio', '薛定谔的化裁间', true)]}, preferences: {schemaVersion: 1, revision: 1, pinnedToolIds, lastSelectedToolId}, settings: {revision: 1, githubProxy: {enabled: false, urls: []}}, operations, toolbox: {}, about: {}, directoryWarning: {}, capabilities: {}};
  const document = {getElementById: id => node(id), activeElement: null, addEventListener(name, listener) {(listeners[name] ||= []).push(listener);}, querySelectorAll() {return [];}};
  const node = id => nodes.get(id) || nodes.set(id, {id, dataset: {}, isConnected: true, classList: {add() {}, remove() {}}, setAttribute() {}, addEventListener() {}, focus() {document.activeElement = this;}, contains(element) {return element.onSurface === true;}, close() {this.open = false;}, showModal() {this.open = true;}, querySelectorAll() {return [];}}).get(id);
  const bridge = {available: true, listen: async callback => {listeners.host = callback; return () => {};}, async dispatch(method, params = {}) {
    calls.push({method, params});
    if (method === 'app.bootstrap') return structuredClone(snapshot);
    if (method === 'preferences.selectTool' && failSelectionOnce && !selectionFailed) {selectionFailed = true; throw new Error('选择保存失败，请重试。');}
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
    if (method === 'tool.launch' && deferLaunch) await new Promise(resolve => launches.push(resolve));
    return {accepted: true};
  }};
  const location = {hash: '#home'};
  const appWindow = {scrollY: 0, scrollTo(x, y) {scrolls.push([x, y]); this.scrollY = y;}, addEventListener() {}};
  const context = vm.createContext({...model, createBridge: () => bridge, location, history: {pushState(_state, _title, hash) {location.hash = hash;}}, document, window: appWindow, requestAnimationFrame(callback) {queueMicrotask(callback);}, setTimeout(_callback, delay) {timers.push(delay); return 1;}, clearTimeout() {}, console});
  vm.runInContext(source, context);
  const settle = async () => {for (let i = 0; i < 10; i++) await new Promise(resolve => setImmediate(resolve));};
  const evaluate = (script, bindings = {}) => {Object.assign(context, bindings); return vm.runInContext(script, context);};
  const card = (id, {onSurface = true, pin = true} = {}) => ({dataset: {act: pin ? 'focus' : 'detail', id}, onSurface, isConnected: true, classList: {contains: name => pin && name === 'pin-card'}, hasAttribute: name => name === 'data-act', getAttribute: name => name === 'data-act' ? pin ? 'focus' : 'detail' : null, closest(selector) {return selector === 'dialog' ? null : selector.includes('.pin-card') ? pin ? this : null : this;}, focus() {document.activeElement = this;}});
  const dispatch = (name, element, detail = name === 'dblclick' ? 2 : 1) => {
    if (name === 'click') document.activeElement = element;
    for (const listener of listeners[name] || []) listener({detail, button: 0, target: {closest: selector => element.closest(selector)}});
  };
  await settle();
  return {calls, node, snapshot, responses, launches, scrolls, timers, document, appWindow, card, dispatch, settle, evaluate, emit: listeners.host, html: () => node('surface').innerHTML};
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

test('a home fixed-card single click selects immediately without scrolling or moving keyboard focus', async () => {
  const ui = await app({deferPreferences: true});
  const card = ui.card('dnf_avatar_studio');
  ui.appWindow.scrollY = 920;
  ui.dispatch('click', card);
  assert.equal(ui.evaluate('selectedTool().id'), 'dnf_avatar_studio');
  assert.equal(ui.appWindow.scrollY, 920);
  assert.deepEqual(ui.scrolls, []);
  assert.equal(ui.document.activeElement, card);
  assert.deepEqual(ui.timers, [], 'selection must not wait for a custom double-click timeout');
  assert.equal(ui.calls.filter(call => call.method === 'tool.launch').length, 0);
  ui.responses[0].resolve();
  await ui.settle();
  assert.equal(ui.document.activeElement, card);
});

test('the native double-click event launches the clicked fixed tool once through tool.launch', async () => {
  const ui = await app({deferPreferences: true, deferLaunch: true});
  const card = ui.card('dnf_avatar_studio');
  ui.dispatch('click', card, 1);
  ui.emit({v: 1, sessionId: 'home-session', seq: 1, event: 'preferences.changed', data: ui.responses[0].result});
  await ui.settle();
  ui.dispatch('click', card, 2);
  assert.equal(ui.calls.filter(call => call.method === 'tool.launch').length, 0, 'click counts alone do not launch');
  assert.equal(ui.calls.filter(call => call.method === 'preferences.selectTool').length, 1, 'the second click does not write the same selection again');
  ui.dispatch('dblclick', card);
  ui.dispatch('dblclick', card);
  assert.deepEqual(ui.calls.filter(call => call.method === 'tool.launch').map(call => call.params.toolId), ['dnf_avatar_studio']);
  assert.equal(ui.calls.filter(call => call.method === 'operation.start').length, 0);
  for (const response of ui.responses) response.resolve();
  for (const resolve of ui.launches) resolve();
  await ui.settle();
});

test('rapid clicks across fixed cards select the last card without manufacturing a double-click', async () => {
  const ui = await app();
  ui.dispatch('click', ui.card('rep_player'), 1);
  ui.dispatch('click', ui.card('dnf_avatar_studio'), 2);
  await ui.settle();
  assert.equal(ui.evaluate('selectedTool().id'), 'dnf_avatar_studio');
  assert.equal(ui.calls.filter(call => call.method === 'tool.launch' || call.method === 'operation.start').length, 0);
  assert.deepEqual(ui.timers, []);
});

test('keyboard card activation only selects and an explicit home detail entry keeps its navigation behavior', async () => {
  const ui = await app();
  const card = ui.card('dnf_avatar_studio');
  ui.appWindow.scrollY = 920;
  ui.dispatch('click', card, 0);
  await ui.settle();
  assert.equal(ui.appWindow.scrollY, 920);
  assert.equal(ui.document.activeElement, card);
  assert.equal(ui.calls.filter(call => call.method === 'tool.launch').length, 0);
  ui.evaluate('go("library")');
  ui.appWindow.scrollY = 300;
  await ui.evaluate('focusTool("rep_player")');
  assert.equal(ui.evaluate('ui.page'), 'home');
  assert.equal(ui.appWindow.scrollY, 0);
  assert.equal(ui.document.activeElement, ui.node('surface'));
});

test('repeated home-card selection skips an acknowledged selection but can retry a failed preference save', async () => {
  const acknowledged = await app({lastSelectedToolId: 'rep_player'});
  acknowledged.dispatch('click', acknowledged.card('rep_player'));
  await acknowledged.settle();
  assert.equal(acknowledged.calls.filter(call => call.method === 'preferences.selectTool').length, 0);
  await acknowledged.evaluate('focusTool("rep_player")');
  assert.equal(acknowledged.calls.filter(call => call.method === 'preferences.selectTool').length, 1, 'non-card focus entries keep their existing behavior');

  const retry = await app({failSelectionOnce: true});
  const card = retry.card('dnf_avatar_studio');
  retry.dispatch('click', card);
  await retry.settle();
  assert.equal(retry.evaluate('selectedTool().id'), 'dnf_avatar_studio');
  assert.match(retry.node('toast').textContent, /选择保存失败/);
  retry.dispatch('click', card);
  await retry.settle();
  assert.equal(retry.calls.filter(call => call.method === 'preferences.selectTool').length, 2);
  assert.equal(retry.evaluate('state.preferences.lastSelectedToolId'), 'dnf_avatar_studio');
});

test('pending home selection is deduplicated only for the current card, including A-B-A clicks', async () => {
  const ui = await app({deferPreferences: true, lastSelectedToolId: 'rep_player'});
  const a = ui.card('dnf_avatar_studio'), b = ui.card('dnf_skill_prism');
  ui.dispatch('click', a, 1);
  ui.dispatch('click', a, 2);
  assert.equal(ui.calls.filter(call => call.method === 'preferences.selectTool').length, 1);
  ui.dispatch('click', b, 1);
  ui.dispatch('click', a, 2);
  assert.deepEqual(ui.calls.filter(call => call.method === 'preferences.selectTool').map(call => call.params.toolId), ['dnf_avatar_studio', 'dnf_skill_prism', 'dnf_avatar_studio']);
  for (const response of [...ui.responses].reverse()) response.resolve();
  await ui.settle();
  assert.equal(ui.evaluate('selectedTool().id'), 'dnf_avatar_studio');
  assert.equal(ui.evaluate('state.preferences.lastSelectedToolId'), 'dnf_avatar_studio');
  assert.equal(ui.calls.filter(call => call.method === 'tool.launch').length, 0);
});

test('fixed-card titles explain double-click launch only when the tool can currently launch', async () => {
  const catalog = [tool('ready', '可启动'), {...tool('installable', '待安装'), installed: false, actions: ['install']}, tool('busy', '任务中')];
  const ui = await app({catalog, pinnedToolIds: catalog.map(item => item.id), operations: [{id: 'busy-op', toolId: 'busy', state: 'running'}]});
  assert.match(ui.html(), /class="pin-card"[^>]*data-id="ready"[^>]*title="[^"]*单击切换，双击启动/);
  for (const id of ['installable', 'busy']) {
    const title = ui.html().match(new RegExp(`class="pin-card"[^>]*data-id="${id}"[^>]*title="([^"]*)"`))?.[1];
    assert.match(title, /单击切换/);
    assert.doesNotMatch(title, /双击启动/);
  }
});

test('double-click never installs, repairs, or launches a busy, unavailable, or unpinned tool', async () => {
  const catalog = [tool('launchable', '可启动'), {...tool('installable', '待安装'), installed: false, actions: ['install']}, {...tool('repairable', '待修复'), entryState: 'missing', actions: ['reinstall']}, {...tool('blocked', '已禁用'), actions: []}, tool('busy', '任务中'), tool('unpinned', '未固定')];
  const ui = await app({catalog, pinnedToolIds: catalog.filter(item => item.id !== 'unpinned').map(item => item.id), operations: [{id: 'busy-op', toolId: 'busy', state: 'running'}]});
  for (const id of ['installable', 'repairable', 'blocked', 'busy', 'unpinned', 'unknown']) ui.dispatch('dblclick', ui.card(id));
  await ui.settle();
  assert.equal(ui.calls.filter(call => call.method === 'tool.launch' || call.method === 'operation.start').length, 0);
  ui.dispatch('dblclick', ui.card('launchable'));
  await ui.settle();
  assert.deepEqual(ui.calls.filter(call => call.method === 'tool.launch').map(call => call.params.toolId), ['launchable']);
});

test('the tool library and cards outside the home surface have no double-click launch action', async () => {
  const ui = await app();
  ui.dispatch('dblclick', ui.card('rep_player', {onSurface: false}));
  ui.dispatch('dblclick', ui.card('rep_player', {pin: false}));
  ui.evaluate('go("library")');
  ui.dispatch('dblclick', ui.card('rep_player'));
  ui.dispatch('dblclick', ui.card('rep_player', {pin: false}));
  await ui.settle();
  assert.equal(ui.calls.filter(call => call.method === 'tool.launch' || call.method === 'operation.start').length, 0);
});

// A small DOM double makes node replacement and movement observable without a
// browser dependency. Browser QA separately verifies the native dblclick stream.
function cardGrid(ids, selectedId = ids[0]) {
  const grid = {children: [], moves: 0, get firstElementChild() {return this.children[0] || null;}, insertBefore(card, before) {
    this.moves++;
    card.remove();
    const index = before ? this.children.indexOf(before) : this.children.length;
    this.children.splice(index, 0, card);
    card.parentElement = this;
  }};
  for (const id of ids) {
    const attributes = new Map([['aria-pressed', String(id === selectedId)], ['title', `在首页查看${id}`]]);
    let html = `<span><img src="${id}.webp"><strong>${id}</strong></span>`;
    const card = {dataset: {id}, parentElement: grid, replacements: 0, get isConnected() {return this.parentElement !== null;}, get nextElementSibling() {return this.parentElement?.children[this.parentElement.children.indexOf(this) + 1] || null;}, get innerHTML() {return html;}, set innerHTML(value) {this.replacements++; html = value;}, getAttribute(name) {return attributes.get(name) ?? null;}, setAttribute(name, value) {attributes.set(name, String(value));}, remove() {if (!this.parentElement) return; const cards = this.parentElement.children; cards.splice(cards.indexOf(this), 1); this.parentElement = null;}};
    card.img = {card, get isConnected() {return card.isConnected && card.replacements === 0;}};
    card.strong = {card, get isConnected() {return card.isConnected && card.replacements === 0;}};
    grid.children.push(card);
  }
  return grid;
}

test('home selection and preference redraws keep the original card and actual image/text click targets connected', async () => {
  const ui = await app();
  const grid = cardGrid(['rep_player', 'dnf_avatar_studio']);
  const [first, second] = grid.children;
  for (const selectedId of ['dnf_avatar_studio', 'dnf_avatar_studio', 'rep_player']) {
    const next = cardGrid(['rep_player', 'dnf_avatar_studio'], selectedId);
    ui.evaluate('if (typeof syncHomeCards === "function") syncHomeCards(currentGrid, nextGrid)', {currentGrid: grid, nextGrid: next});
    assert.equal(second.getAttribute('aria-pressed'), String(selectedId === 'dnf_avatar_studio'));
    assert.equal(grid.children[0], first);
    assert.equal(grid.children[1], second);
    assert.equal(second.img.isConnected, true);
    assert.equal(second.strong.isConnected, true);
  }
  assert.equal(grid.moves, 0, 'ordinary redraws must not detach and reinsert click targets');
});

test('host pin changes reconcile cards by tool ID while retaining surviving click targets', async () => {
  const ui = await app();
  const grid = cardGrid(['a', 'b', 'retired']);
  const [a, b, retired] = grid.children;
  const next = cardGrid(['b', 'new', 'a'], 'b');
  ui.evaluate('if (typeof syncHomeCards === "function") syncHomeCards(currentGrid, nextGrid)', {currentGrid: grid, nextGrid: next});
  assert.deepEqual(grid.children.map(card => card.dataset.id), ['b', 'new', 'a']);
  assert.equal(grid.children[0], b);
  assert.equal(grid.children[2], a);
  assert.equal(b.img.isConnected, true);
  assert.equal(b.strong.isConnected, true);
  assert.equal(retired.isConnected, false);
});
