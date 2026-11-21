import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import * as model from '../frontend/assets/model.mjs';

const source = (await readFile(new URL('../frontend/assets/app.mjs', import.meta.url), 'utf8')).replace(/^import .*\n/gm, '');
const channelUrl = 'https://pd.qq.com/s/g907v9hdc';
const channelText = `QQ频道:点击链接加入腾讯频道【物理世界的工具箱】：${channelUrl}`;
const legacyAbout = {author: '物理世界的欺骗', qqNumber: '1548220577', feedbackUrl: 'https://github.com/PhysicalWorldDo/DNFTOOLBOX/issues', bilibiliUrl: 'https://space.bilibili.com/492488982', sponsorQrs: [{id: 'weixin', title: '微信赞助', imageUrl: 'assets/sponsor-weixin.jpg'}]};

async function app(about = legacyAbout) {
  const calls = [], listeners = {}, nodes = new Map();
  const snapshot = {protocol: 1, sessionId: 'about-session', seq: 0, catalog: {tools: [], revision: 1, source: 'cached'}, preferences: {pinnedToolIds: []}, settings: {revision: 1}, operations: [], toolbox: {version: '0.4.0'}, about, directoryWarning: {}, capabilities: {}};
  const node = id => nodes.get(id) || nodes.set(id, {id, dataset: {}, classList: {add() {}, remove() {}}, setAttribute() {}, addEventListener() {}, focus() {}, close() {this.open = false;}, showModal() {this.open = true;}, querySelectorAll() {return [];}}).get(id);
  const bridge = {available: true, listen: async () => () => {}, async dispatch(method, params = {}) {
    calls.push({method, params});
    return method === 'app.bootstrap' ? structuredClone(snapshot) : {accepted: true};
  }};
  const context = vm.createContext({...model, createBridge: () => bridge, location: {hash: '#about'}, history: {pushState() {}}, document: {getElementById: node, activeElement: null, addEventListener(name, listener) {(listeners[name] ||= []).push(listener);}}, window: {scrollY: 0, scrollTo() {}, addEventListener() {}}, requestAnimationFrame(callback) {queueMicrotask(callback);}, setTimeout: () => 1, clearTimeout() {}, console});
  vm.runInContext(source, context);
  const settle = async () => {for (let i = 0; i < 10; i++) await new Promise(resolve => setImmediate(resolve));};
  await settle();
  return {calls, node, settle, evaluate: script => vm.runInContext(script, context), clickLink() {
    let prevented = false;
    const element = {tagName: 'A', dataset: {act: 'link', id: 'qq-channel'}, hasAttribute: name => name === 'data-act'};
    for (const listener of listeners.click || []) listener({target: {closest: () => element}, preventDefault() {prevented = true;}});
    return prevented;
  }};
}

test('normal about entry shows the complete channel invitation with a real link for new and legacy snapshots', async () => {
  for (const about of [legacyAbout, {}, {...legacyAbout, qqChannelUrl: channelUrl, qqChannelText: channelText}]) {
    const ui = await app(about);
    const html = ui.node('surface').innerHTML;
    assert.match(html, /<a\b[^>]*href="https:\/\/pd\.qq\.com\/s\/g907v9hdc"[^>]*data-act="link"[^>]*data-id="qq-channel"[^>]*>QQ频道:点击链接加入腾讯频道【物理世界的工具箱】：https:\/\/pd\.qq\.com\/s\/g907v9hdc<\/a>/);
    assert.doesNotMatch(html, /联系作者|1548220577|data-act="contact"/);
  }
});

test('channel link activation uses the controlled host target and prevents WebView navigation', async () => {
  const ui = await app();
  assert.equal(ui.clickLink(), true);
  await ui.settle();
  const calls = ui.calls.filter(call => call.method === 'system.openLink');
  assert.equal(calls.length, 1);
  assert.equal(calls[0].params.target, 'qq-channel');
  assert.equal(ui.node('dialog').open, undefined);
});

test('feedback, author video profile and sponsorship retain their existing actions', async () => {
  const ui = await app();
  assert.match(ui.node('surface').innerHTML, /data-act="link" data-id="feedback"/);
  assert.match(ui.node('surface').innerHTML, /data-act="link" data-id="author"/);
  assert.match(ui.node('surface').innerHTML, /data-act="sponsor" data-id="weixin"/);
  await ui.evaluate('handleAction({dataset: {act: "link", id: "feedback"}})');
  assert.equal(ui.calls.find(call => call.method === 'system.openLink').params.target, 'feedback');
  ui.evaluate('openModal("sponsor", {id: "weixin"})');
  assert.equal(ui.node('dialog-title').textContent, '微信赞助');
  assert.match(ui.node('dialog-body').innerHTML, /src="assets\/sponsor-weixin.jpg"/);
});
