// Test-only local adapter. It is outside Tauri frontendDist and never packaged.
// All commands are simulated; no tool directory, process, clipboard, or network
// operation is performed. --snapshot accepts a real app.bootstrap JSON capture.
import http from 'node:http';
import fs from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../frontend');
const args = process.argv.slice(2);
const option = (name, fallback = '') => {const item = args.find(value => value.startsWith(`--${name}=`)); return item ? item.slice(name.length + 3) : fallback;};
const port = Number(option('port', '4173'));
const scenario = option('scenario', 'normal');
const source = option('snapshot');
const listeners = new Set();
const requests = [];
let nextOperation = 0;
let windowState = {maximized: false, active: true};

function fixtureTool(id, name, category, installedVersion, latestVersion, description) {
  return {id, name, category, description, projectUrl: 'https://example.invalid/test-only', iconUrl: null, needAdmin: false, installed: Boolean(installedVersion), installedVersion, latestVersion, hasUpdate: Boolean(installedVersion && installedVersion !== latestVersion), entryState: installedVersion ? 'ready' : 'notInstalled', disabledReason: null, actions: installedVersion ? ['launch', 'update', 'reinstall', 'switchVersion', 'uninstall', 'openFolder', 'copyProjectUrl'] : ['install', 'copyProjectUrl'], availableVersions: [...new Set([latestVersion, installedVersion, '0.9.0'].filter(Boolean))].map(version => ({version, date: '2026-11-21', channel: 'stable', sizeBytes: 16777216, compatible: true, blocked: false, canInstall: true, disabledReason: null, changelog: ['浏览器合同样本，仅用于界面验收。']}))};
}

let snapshot = {
  protocol: 1, sessionId: 'browser-fixture', seq: 0,
  catalog: {source: scenario === 'empty' ? 'empty' : 'cached', checkedAt: '2026-11-21T02:00:00Z', revision: 7, tools: scenario === 'empty' ? [] : [
    fixtureTool('dnf_palette_prism', '阿拉德调色', '图像工具', '1.0.0', '1.1.0', '预览 DNF 图像的调色盘，调整整组素材的配色。'),
    fixtureTool('rep_player', 'REP 播放器', '资源工具', '1.1.5', '1.2.3', '逐帧查看录像，预览声音并导出视频与图像。'),
    fixtureTool('dnf_skill_prism', '技能染色工坊', '资源工具', null, '1.0.0', '为技能特效调整颜色，预览技能配色。'),
    fixtureTool('dnf_avatar_studio', '薛定谔的化裁间', '图像工具', '1.0.0', '1.1.0', '浏览、搭配与预览角色时装。'),
    fixtureTool('dnf_music_detect', 'DNF 音乐识别', '音频工具', '1.0.0', '1.0.0', '识别与预览本地音乐资源。')
  ], categories: ['图像工具', '资源工具', '音频工具'], updateCount: 3, newInstallCount: 1, error: scenario === 'offline' ? '测试场景：网络不可用，继续显示上次缓存。' : null},
  preferences: {schemaVersion: 1, pinnedToolIds: ['empty', 'no-pins'].includes(scenario) ? [] : ['rep_player', 'dnf_skill_prism', 'dnf_avatar_studio'], lastSelectedToolId: 'dnf_palette_prism', revision: 1},
  settings: {name: '物理世界的工具箱', indexUrl: 'https://example.invalid/test-only', channel: 'stable', githubProxy: {enabled: true, urls: ['https://example.invalid/proxy/']}, suppressDirectoryWarning: false, revision: 1},
  operations: [], toolbox: {version: '0.4.0', latestVersion: '0.4.0', hasUpdate: false, canDownload: false},
  about: {title: '物理世界的工具箱', author: '物理世界的欺骗', tagline: 'DNF 工具集', qqChannelUrl: 'https://pd.qq.com/s/g907v9hdc', qqChannelText: 'QQ频道:点击链接加入腾讯频道【物理世界的工具箱】：https://pd.qq.com/s/g907v9hdc', githubUrl: 'https://example.invalid/project', feedbackUrl: 'https://example.invalid/feedback', bilibiliUrl: 'https://example.invalid/author', sponsorQrs: [{id: 'weixin', title: '微信赞助', imageUrl: 'assets/sponsor-weixin.jpg'}, {id: 'zhifubao', title: '支付宝赞助', imageUrl: 'assets/sponsor-zhifubao.jpg'}], logs: []},
  directoryWarning: {show: false, message: '', workspace: 'E:\\UI 合同验收\\工具箱'}, capabilities: {install: true, launch: true, selfUpdate: true, cancel: true, openWorkspace: true}
};
if (source) {const input = JSON.parse(await fs.readFile(path.resolve(source), 'utf8')); snapshot = input.result || input;}
if (scenario === 'blocked' && snapshot.catalog.tools[0]) {
  const tool = snapshot.catalog.tools[0];
  tool.actions = ['openFolder', 'copyProjectUrl']; tool.disabledReason = '测试场景：版本已禁用。';
  for (const release of tool.availableVersions) {release.blocked = true; release.canInstall = false; release.disabledReason = tool.disabledReason;}
}

function emit(event, data) {
  if (event === 'window.changed') windowState = {...windowState, ...data};
  const fields = {'catalog.changed': 'catalog', 'settings.changed': 'settings', 'preferences.changed': 'preferences', 'toolbox.changed': 'toolbox'};
  if (fields[event]) snapshot[fields[event]] = data;
  if (event === 'operation.changed') {const index = snapshot.operations.findIndex(item => item.id === data.id); if (index === -1) snapshot.operations.push(data); else snapshot.operations[index] = {...snapshot.operations[index], ...data};}
  const message = {v: 1, sessionId: snapshot.sessionId, seq: ++snapshot.seq, event, data};
  for (const response of listeners) response.write(`data: ${JSON.stringify(message)}\n\n`);
  return message;
}

async function dispatch(request) {
  requests.push(request);
  let result = {accepted: true};
  const params = request.params || {};
  switch (request.method) {
    case 'app.bootstrap': result = structuredClone(snapshot); break;
    case 'app.ready': break;
    case 'catalog.tool': result = snapshot.catalog.tools.find(tool => tool.id === params.toolId); break;
    case 'catalog.refresh': snapshot.catalog.checkedAt = new Date().toISOString(); emit('catalog.changed', snapshot.catalog); result = snapshot.catalog; break;
    case 'toolbox.checkUpdate':
      if (scenario === 'offline') return {v: 1, id: request.id, ok: false, error: {code: 'NETWORK', message: '测试场景：无法查询 GitHub Releases。', retryable: true}};
      result = {...snapshot.toolbox, checked: true};
      if (scenario === 'self-update') {
        Object.assign(result, {latestVersion: '0.4.3', hasUpdate: true, canDownload: true, sizeBytes: 6400000, changelog: ['测试样本，来自 GitHub 正式版。'], releaseUrl: 'https://github.com/PhysicalWorldDo/DNFtoolBox-Rust/releases/tag/v0.4.3'});
      }
      emit('toolbox.changed', result);
      break;
    case 'preferences.setPinned': {
      const ids = params.pinned ? [...snapshot.preferences.pinnedToolIds] : snapshot.preferences.pinnedToolIds.filter(id => id !== params.toolId);
      if (params.pinned && !ids.includes(params.toolId)) ids.push(params.toolId);
      result = {...snapshot.preferences, pinnedToolIds: ids, revision: snapshot.preferences.revision + 1}; emit('preferences.changed', result); break;
    }
    case 'preferences.selectTool': result = {...snapshot.preferences, lastSelectedToolId: params.toolId, revision: snapshot.preferences.revision + 1}; emit('preferences.changed', result); break;
    case 'settings.get': result = snapshot.settings; break;
    case 'settings.proxyDefaults': result = ['https://fixture.invalid/default/']; break;
    case 'settings.update':
      if (params.revision !== snapshot.settings.revision) return {v: 1, id: request.id, ok: false, error: {code: 'STALE_REVISION', message: '测试场景：设置已变化，请重新载入。', retryable: true}};
      result = {...snapshot.settings, ...params.patch, revision: snapshot.settings.revision + 1}; emit('settings.changed', result); break;
    case 'operation.start': {
      const operation = {id: `fixture-op-${++nextOperation}`, kind: params.kind, toolId: params.toolId || null, version: params.version || null, state: 'running', stage: 'download', doneBytes: 1048576, totalBytes: null, canCancel: true, error: null, children: [], result: null};
      emit('operation.changed', operation); result = {operationId: operation.id}; break;
    }
    case 'operation.list': result = snapshot.operations; break;
    case 'operation.cancel': {const operation = snapshot.operations.find(item => item.id === params.operationId); emit('operation.changed', {...operation, state: 'cancelled', canCancel: false}); break;}
    case 'operation.retry': {const operation = snapshot.operations.find(item => item.id === params.operationId); emit('operation.changed', {...operation, state: 'running', canCancel: true, error: null}); result = {operationId: operation.id}; break;}
    case 'window.state': result = {accepted: true, ...windowState}; break;
    case 'window.minimize':
      emit('window.changed', {...windowState, active: false}); result = {accepted: true, ...windowState}; break;
    case 'window.toggleMaximize':
      emit('window.changed', {...windowState, maximized: !windowState.maximized}); result = {accepted: true, ...windowState}; break;
    case 'window.requestClose': {
      const active = snapshot.operations.filter(item => ['queued', 'running'].includes(item.state));
      if (active.length && (!params.cancelTasks || active.some(item => !item.canCancel))) return {v: 1, id: request.id, ok: false, error: {code: 'BUSY', message: '测试场景：有任务正在进行。', retryable: true}};
      for (const operation of active) emit('operation.changed', {...operation, state: 'cancelled', canCancel: false});
      result = {accepted: true, ...windowState}; break;
    }
    case 'toolbox.downloadUpdate': {const operation = {id: `fixture-op-${++nextOperation}`, kind: 'toolboxUpdate', toolId: null, version: params.version, state: 'running', stage: 'download', doneBytes: 0, totalBytes: null, canCancel: true, error: null, children: []}; emit('operation.changed', operation); result = {operationId: operation.id}; break;}
  }
  return {v: 1, id: request.id, ok: true, result};
}

const bridgeSource = `(() => {const streams=[]; window.__TAURI__={core:{invoke:async(command,args)=>{if(command!=='toolbox_dispatch')throw new Error('Unknown fixture command');return fetch('/__fixture/dispatch',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(args.request)}).then(r=>r.json());}},event:{listen:async(name,callback,options)=>{if(name!=='toolbox://event'||options?.target!=='main')throw new Error('Fixture event contract mismatch');const stream=new EventSource('/__fixture/events');streams.push(stream);stream.onmessage=e=>callback({payload:JSON.parse(e.data)});return()=>stream.close();}}};window.addEventListener('beforeunload',()=>streams.forEach(stream=>stream.close()));})();`;
const mime = {'.html': 'text/html; charset=utf-8', '.css': 'text/css; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.webp': 'image/webp', '.jpg': 'image/jpeg'};
const server = http.createServer(async (request, response) => {
  const url = new URL(request.url, `http://127.0.0.1:${port}`);
  response.setHeader('cache-control', 'no-store');
  try {
    if (url.pathname === '/__fixture/events') {response.writeHead(200, {'content-type': 'text/event-stream', connection: 'keep-alive'}); response.write(': fixture event channel\n\n'); listeners.add(response); request.on('close', () => listeners.delete(response)); return;}
    if (url.pathname === '/__fixture/bridge.js') {response.writeHead(200, {'content-type': 'text/javascript'}); response.end(bridgeSource); return;}
    if (url.pathname === '/__fixture/state' || url.pathname === '/__fixture/requests') {response.writeHead(200, {'content-type': 'application/json'}); response.end(JSON.stringify(url.pathname.endsWith('requests') ? requests : snapshot)); return;}
    if (request.method === 'POST' && ['/__fixture/dispatch', '/__fixture/event'].includes(url.pathname)) {
      let body = ''; for await (const chunk of request) body += chunk;
      const input = JSON.parse(body);
      const result = url.pathname.endsWith('/event') ? emit(input.event, input.data) : await dispatch(input);
      response.writeHead(200, {'content-type': 'application/json'}); response.end(JSON.stringify(result)); return;
    }
    const relative = decodeURIComponent(url.pathname === '/' ? '/index.html' : url.pathname).replace(/^\/+/, '');
    const target = path.resolve(root, relative);
    if (target !== root && !target.startsWith(`${root}${path.sep}`)) {response.writeHead(403); response.end(); return;}
    let body = await fs.readFile(target);
    if (target.endsWith('.html')) body = Buffer.from(body.toString().replace('<head>', '<head><script src="/__fixture/bridge.js"></script>'));
    response.writeHead(200, {'content-type': mime[path.extname(target)] || 'application/octet-stream'}); response.end(body);
  } catch (error) {response.writeHead(404, {'content-type': 'text/plain; charset=utf-8'}); response.end(error.message);}
});
server.listen(port, '127.0.0.1', () => console.log(`Test-only toolbox fixture: http://127.0.0.1:${port} (${source ? 'captured bootstrap' : scenario})`));
