import {createBridge} from './bridge.mjs';
import {createState, applyBootstrap, applyPreferences, applyEvent, isActive, primaryAction, filterTools} from './model.mjs';

const bridge = createBridge();
const state = createState();
const pages = {home: ['首页', 'home', '首页'], library: ['工具库', 'grid', '工具库'], updates: ['更新', 'download', '安装与更新'], settings: ['设置', 'settings', '设置'], about: ['关于', 'info', '关于']};
const art = {dnf_palette_prism: {file: 'prism.webp', english: 'Palette Prism'}, rep_player: {file: 'reel.webp', english: 'Replay Player'}, dnf_avatar_studio: {file: 'tailoring-atelier.webp', english: 'Schrödinger’s Atelier'}, dnf_skill_prism: {file: 'skill-dye-workshop.webp', english: 'Skill Dye Workshop'}};
const paths = {
  home: 'M3 10.5 12 3l9 7.5V21h-6v-7H9v7H3z', grid: 'M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z',
  download: 'M12 3v12m-5-5 5 5 5-5M4 16v4h16v-4', settings: 'm9 3 1-1h4l1 3 2 1 3-.3 2 3-2 2v2l2 2-2 3-3-.3-2 1-1 3h-4l-1-3-2-1-3 .3-2-3 2-2v-2l-2-2 2-3 3 .3 2-1z M15.3 12a3.3 3.3 0 1 1-6.6 0 3.3 3.3 0 0 1 6.6 0z',
  info: 'M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0z M12 11v6m0-10v.3', search: 'M18 10.5a7.5 7.5 0 1 1-15 0 7.5 7.5 0 0 1 15 0z M16 16l5 5',
  right: 'm9 5 7 7-7 7', open: 'M7 17 18 6M8 6h10v10', close: 'm6 6 12 12M18 6 6 18', more: 'M5 12h.3M12 12h.3m6.7 0h.3', play: 'm8 4 12 8-12 8z',
  pin: 'm14 3 7 7-4 1.5-3 5.5-2-2-7 6 5-8-2-2 5-3z', box: 'm3 7 9-5 9 5v10l-9 5-9-5z M3 7l9 5 9-5m-9 5v10',
  minimize: 'M5 12h14', maximize: 'M5 5h14v14H5z', restore: 'M8 8h11v11H8z M5 15V5h10'
};
const ui = {page: pageFromHash(), tab: 'updates', query: '', category: '全部', filter: 'all', selectedId: null, loading: true, checking: false, error: null, modal: null, toastTimer: null, returnFocus: null, settingsDraft: null, settingsBaseRevision: null, settingsDirty: false, settingsMessage: '', dismissedWarning: false, pending: new Set()};
const $ = id => document.getElementById(id);
const esc = value => String(value ?? '').replace(/[&<>"']/g, character => ({'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'}[character]));
const tools = () => state.catalog.tools || [];
const findTool = id => tools().find(tool => tool.id === id);
const pins = () => state.preferences.pinnedToolIds || [];
const updates = () => tools().filter(tool => tool.hasUpdate);
const version = value => value ? `v${String(value).replace(/^v/i, '')}` : '未安装';
const bytes = value => value == null ? '大小未知' : Number(value) < 1048576 ? `${(Number(value) / 1024).toFixed(1)} KiB` : `${(Number(value) / 1048576).toFixed(1)} MiB`;
const release = (tool, selected = tool.latestVersion) => (tool.availableVersions || []).find(item => item.version === selected);
const activeTask = toolId => state.operations.find(item => isActive(item) && (!toolId || item.toolId === toolId));
const allows = (tool, action) => Boolean(tool?.actions?.includes(action));

function icon(name, size = 20) {
  return `<svg aria-hidden="true" width="${size}" height="${size}" viewBox="0 0 24 24" fill="${name === 'play' ? 'currentColor' : 'none'}" stroke="currentColor" stroke-width="${name === 'more' ? 3.5 : 1.7}" stroke-linecap="round" stroke-linejoin="round"><path d="${paths[name] || paths.box}"/></svg>`;
}

function imageUrl(value) {
  const source = String(value || '');
  return /^(assets\/[a-zA-Z0-9._/-]+|data:image\/(png|jpeg|webp|x-icon|vnd\.microsoft\.icon);base64,|asset:|https?:\/\/asset\.localhost\/)/.test(source) ? source : '';
}

function tile(tool, size = '') {
  const source = imageUrl(tool?.iconUrl);
  return `<span class="tool-tile ${size}" aria-hidden="true">${source ? `<img src="${esc(source)}" data-icon-fallback="${esc(Array.from(tool?.name || '工')[0])}" alt="">` : `<span class="tool-initial">${esc(Array.from(tool?.name || '工')[0])}</span>`}</span>`;
}

function button(label, action, {id = '', selectedVersion = '', kind = '', className = '', disabled = false, title = ''} = {}) {
  return `<button type="button" class="button pz-btn ${className}" data-act="${esc(action)}" data-id="${esc(id)}" data-version="${esc(selectedVersion)}" data-kind="${esc(kind)}" ${disabled ? 'disabled' : ''} ${title ? `title="${esc(title)}"` : ''}>${esc(label)}</button>`;
}

function heading(title, description, actions = '') {
  return `<header class="pz-head"><div><h1>${esc(title)}</h1><p>${esc(description)}</p></div>${actions ? `<div class="pz-actions">${actions}</div>` : ''}</header>`;
}

function empty(title, text = '', actions = '') {
  return `<div class="pz-empty"><strong>${esc(title)}</strong>${text ? `<p>${esc(text)}</p>` : ''}${actions ? `<div class="pz-actions">${actions}</div>` : ''}</div>`;
}

function toast(message) {
  clearTimeout(ui.toastTimer);
  $('toast').textContent = message;
  $('toast').classList.add('shown');
  ui.toastTimer = setTimeout(() => $('toast').classList.remove('shown'), 3300);
}

function freshness() {
  const labels = {fresh: '最新目录', cached: '本地缓存', local: '本地安装', empty: '尚无目录'};
  return labels[state.catalog.source] || '本地目录';
}

function checkedAt() {
  if (!state.catalog.checkedAt) return '';
  const date = new Date(state.catalog.checkedAt);
  return Number.isNaN(date.valueOf()) ? String(state.catalog.checkedAt) : date.toLocaleString('zh-CN', {month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false});
}

function syncChrome() {
  $('app-shell').dataset.view = ui.page;
  syncScrollChrome();
  syncWindowChrome();
  const nav = key => {
    const [label, glyph, title] = pages[key];
    return `<button class="nav-btn" data-page="${key}" title="${title}" aria-label="${title}" ${ui.page === key ? 'aria-current="page"' : ''}>${icon(glyph, 21)}${key === 'updates' && updates().length ? `<span class="nav-badge" aria-hidden="true">${updates().length}</span>` : ''}<span>${label}</span></button>`;
  };
  $('main-nav').innerHTML = ['home', 'library', 'updates'].map(nav).join('');
  $('utility-nav').innerHTML = ['settings', 'about'].map(nav).join('');
  $('app-version').textContent = state.toolbox.version ? version(state.toolbox.version) : '正在载入';
  $('catalog-source').dataset.source = state.catalog.source;
  $('catalog-source').innerHTML = `<i></i>${esc(ui.checking ? '正在检查' : freshness())}`;
  $('summary').textContent = ui.loading ? '正在读取工作区' : `目录内 ${tools().length} 款工具　·　已安装 ${tools().filter(tool => tool.installed).length} 款${activeTask() ? '　·　有任务正在处理' : ''}`;
  $('surface').setAttribute('aria-label', pages[ui.page][2]);
  document.title = `物理世界的工具箱 · ${pages[ui.page][2]}`;
  renderNotice();
}

function syncScrollChrome() {
  const scrolled = String(window.scrollY > 0);
  if ($('app-shell').dataset.scrolled !== scrolled) $('app-shell').dataset.scrolled = scrolled;
}

function syncWindowChrome() {
  const maximized = state.window.maximized === true;
  const label = maximized ? '还原窗口' : '最大化窗口';
  $('window-maximize').setAttribute('aria-label', label);
  $('window-maximize').setAttribute('title', label);
  $('window-maximize').innerHTML = icon(maximized ? 'restore' : 'maximize', 14);
  $('app-shell').dataset.windowActive = String(state.window.active !== false);
  $('app-shell').dataset.maximized = String(maximized);
}

function renderNotice() {
  const notices = [];
  if (state.directoryWarning?.show && !ui.dismissedWarning) notices.push(`<div class="notice"><p>${esc(state.directoryWarning.message)}${state.directoryWarning.workspace ? `<br><span>${esc(state.directoryWarning.workspace)}</span>` : ''}</p><div class="pz-actions">${button('打开工作目录', 'workspace')}${button('知道了', 'dismiss-warning')}</div></div>`);
  if (state.catalog.error) notices.push(`<div class="notice error-notice"><p>${esc(state.catalog.error)}${checkedAt() ? `　上次检查 ${esc(checkedAt())}` : ''}</p>${button(ui.checking ? '检查中…' : '重新检查', 'refresh', {disabled: ui.checking})}</div>`);
  if (ui.error && !ui.loading && state.sessionId) notices.push(`<div class="notice error-notice"><p>${esc(ui.error.message)}</p>${button('重新载入状态', 'reconnect')}</div>`);
  $('notice').innerHTML = notices.join('');
}

function featureTools() {
  return [...new Set(pins())].map(findTool).filter(Boolean);
}

function selectedTool(fixed = featureTools()) {
  const selected = fixed.find(tool => tool.id === ui.selectedId) || fixed.find(tool => tool.id === state.preferences.lastSelectedToolId) || fixed[0];
  ui.selectedId = selected?.id || null;
  return selected;
}

function primaryButton(tool, hero = false) {
  const action = primaryAction(tool, state.operations);
  const pending = ui.pending.has(`tool:${tool.id}`);
  const label = pending ? '正在处理…' : action.label;
  if (hero) return `<button class="hero-action" data-act="primary" data-id="${esc(tool.id)}" ${action.disabled || pending ? 'disabled' : ''} ${action.reason ? `title="${esc(action.reason)}"` : ''}><span class="play-disc">${icon(action.action === 'launch' ? 'play' : 'download', 13)}</span>${esc(label)}</button>`;
  return button(label, 'primary', {id: tool.id, disabled: action.disabled || pending, title: action.reason});
}

function pinButton(tool, {compact = false, menu = false} = {}) {
  const pinned = pins().includes(tool.id);
  const label = pinned ? '取消首页固定' : '固定到首页';
  const pending = ui.pending.has(`pin:${tool.id}`);
  return `<button type="button" class="${compact ? 'icon-button' : 'button pz-btn'} pin-button ${menu ? 'pz-menuitem' : ''}" data-act="pin" data-id="${esc(tool.id)}" aria-pressed="${pinned}" aria-label="${esc(label)}${esc(tool.name)}" title="${esc(label)}" ${pending ? 'disabled aria-busy="true"' : ''}>${icon('pin', 17)}${compact ? '' : esc(label)}</button>`;
}

function homePagination(fixed, selectedIndex) {
  const counter = `<span class="counter"><strong>${String(selectedIndex + 1).padStart(2, '0')}</strong> / ${String(fixed.length).padStart(2, '0')}</span>`;
  if (fixed.length > 5) {
    const previous = fixed[(selectedIndex + fixed.length - 1) % fixed.length];
    const next = fixed[(selectedIndex + 1) % fixed.length];
    return `<div class="slide-controls compact-pages" role="group" aria-label="切换固定工具"><button class="slide-arrow previous" data-act="focus" data-id="${esc(previous.id)}" aria-label="上一个固定工具：${esc(previous.name)}" title="上一个固定工具">${icon('right', 15)}</button>${counter}<button class="slide-arrow next" data-act="focus" data-id="${esc(next.id)}" aria-label="下一个固定工具：${esc(next.name)}" title="下一个固定工具">${icon('right', 15)}</button></div>`;
  }
  return `<div class="slide-controls" role="group" aria-label="切换固定工具">${counter}${fixed.map((item, index) => `<button class="slide ${index === selectedIndex ? 'active' : ''}" data-act="focus" data-id="${esc(item.id)}" aria-pressed="${index === selectedIndex}" aria-label="聚焦${esc(item.name)}" title="${esc(item.name)}"></button>`).join('')}</div>`;
}

function renderHome() {
  const focus = featureTools();
  const tool = selectedTool(focus);
  if (!tools().length) return `<section class="home-empty"><h1>首页</h1><p class="page-sub">本地没有可显示的工具目录。</p>${empty('从工具目录开始', '联网检查目录后，可以安装新工具；已安装工具会从本地安装记录读取。', button(ui.checking ? '检查中…' : '检查工具目录', 'refresh', {className: 'primary', disabled: ui.checking}) + button('下载设置', 'settings-page'))}</section>`;
  if (!tool) return `<section class="home-empty"><h1>首页</h1>${empty('还没有固定工具', '在工具库选择常用工具并固定到首页，即可在这里查看和启动。', '<button type="button" class="button primary pz-btn" data-page="library">去工具库固定</button>')}</section>`;
  const visual = art[tool.id];
  const index = focus.findIndex(item => item.id === tool.id);
  const fixed = focus;
  const news = fixed.filter(item => item.hasUpdate).sort((a, b) => String(release(b)?.date || '').localeCompare(String(release(a)?.date || ''))).slice(0, 3);
  const primary = primaryAction(tool, state.operations);
  const pinned = fixed.map(item => {
    const action = primaryAction(item, state.operations);
    const hint = action.action === 'launch' && !action.disabled ? '单击切换，双击启动' : '单击切换';
    return `<button class="pin-card" data-act="focus" aria-pressed="${item.id === tool.id}" data-id="${esc(item.id)}" title="${esc(item.name)}：${hint}"><span class="pin-art">${art[item.id] ? `<img class="pin-art-img" src="assets/${art[item.id].file}" alt="">` : `<span aria-hidden="true">${esc(Array.from(item.name)[0])}</span>`}<span class="pin-overlay">${icon('open', 12)}</span></span><span class="pin-info"><strong>${esc(item.name)}</strong><small>${esc(version(item.installedVersion))}</small></span></button>`;
  }).join('');
  return `<section class="hero ${visual ? '' : 'no-art'}" aria-label="聚焦工具">${visual ? `<img class="hero-img" src="assets/${visual.file}" alt="">` : `<div class="hero-category-art" aria-hidden="true"><span class="initial">${esc(Array.from(tool.name)[0])}</span></div>`}<div class="hero-shade"></div><div class="hero-copy"><h1>${esc(tool.name)}</h1>${visual ? `<p class="hero-en" lang="en">${visual.english}</p>` : ''}<div class="hero-meta"><span class="category">${esc(tool.category || '工具')}</span><span class="build-tag">${esc(version(tool.installedVersion || tool.latestVersion))}</span><span class="installed-label">${tool.installed ? '<i class="state-dot"></i>已安装' : '未安装'}</span></div><p class="hero-description">${esc(tool.description || '此工具暂未提供简介。')}</p><button class="hero-link" data-act="detail" data-id="${esc(tool.id)}">${tool.hasUpdate ? `查看新版 ${esc(version(tool.latestVersion))}` : '查看版本详情'}${icon('right', 13)}</button></div>${primary.disabled && primary.reason ? `<p class="hero-disabled">${esc(primary.reason)}</p>` : ''}<div class="hero-foot">${homePagination(focus, index)}<div class="hero-actions">${primaryButton(tool, true)}<button class="hero-more" data-act="more" data-id="${esc(tool.id)}" aria-label="${esc(tool.name)}更多操作" title="更多工具操作">${icon('more', 23)}</button></div></div></section><div class="home-lower"><section><div class="section-heading"><div><h2>固定工具</h2><span class="count">${String(fixed.length).padStart(2, '0')}</span></div><button class="link-button" data-page="library">工具库 ${icon('right', 13)}</button></div><div class="pin-grid">${pinned}</div></section><section><div class="section-heading"><div><h2>版本动态</h2></div><button class="link-button" data-page="updates">全部更新信息 ${icon('right', 13)}</button></div>${news.length ? `<div class="news-list">${news.map(item => `<button class="news-row" data-act="detail" data-id="${esc(item.id)}"><i class="news-dot"></i><strong>${esc(item.name)}</strong><time>${esc(release(item)?.date || '')}</time><span class="news-sub">${esc(version(item.installedVersion))} → ${esc(version(item.latestVersion))}</span></button>`).join('')}</div>` : '<div class="empty">已固定工具暂无可更新版本。</div>'}</section></div>`;
}

function libraryRows() {
  const rows = filterTools(tools(), ui, pins());
  const html = rows.map(tool => `<article class="library-row" data-tool-id="${esc(tool.id)}"><button class="row-main" data-act="detail" data-id="${esc(tool.id)}" title="查看工具与版本详情">${tile(tool)}<span><strong>${esc(tool.name)}</strong><small>${esc(tool.category || '工具')} · ${esc(version(tool.installedVersion || tool.latestVersion))}${tool.entryState === 'missing' ? ' · 入口缺失' : tool.entryState === 'recoveryRequired' ? ' · 需要恢复' : ''}</small></span></button><span class="status-text ${tool.hasUpdate ? 'update' : ''}">${esc(tool.hasUpdate ? `可更新至 ${version(tool.latestVersion)}` : tool.disabledReason || (tool.installed ? '已安装' : '未安装'))}</span>${pinButton(tool, {compact: true})}${primaryButton(tool)}<button class="icon-button" data-act="more" data-id="${esc(tool.id)}" aria-label="${esc(tool.name)}更多操作" title="更多操作">${icon('more', 17)}</button></article>`).join('');
  return {count: rows.length, html: html || empty('没有找到匹配工具', '试试其他名称或清除当前筛选。', button('清除筛选', 'reset'))};
}

function renderLibrary() {
  const categories = ['全部', ...new Set(tools().map(tool => tool.category).filter(Boolean))];
  if (!categories.includes(ui.category)) ui.category = '全部';
  const result = libraryRows();
  return `<section class="page-wrap"><h1 class="page-heading">工具库</h1><p class="page-sub">按名称或分类找到工具，查看版本并管理安装。</p><div class="library-tools"><label class="library-search">${icon('search', 18)}<input type="search" id="tool-search" placeholder="搜索工具名称、描述或 ID" aria-label="搜索工具名称、描述或 ID" value="${esc(ui.query)}"></label><select id="install-filter" aria-label="筛选工具状态">${[['all', '全部状态'], ['installed', '已安装'], ['uninstalled', '未安装'], ['updates', '可更新'], ['pinned', '已固定']].map(([value, label]) => `<option value="${value}" ${ui.filter === value ? 'selected' : ''}>${label}</option>`).join('')}</select></div><div class="filter-row" role="group" aria-label="工具分类">${categories.map(category => `<button class="filter" data-category="${esc(category)}" aria-pressed="${ui.category === category}">${esc(category)}</button>`).join('')}</div><p class="result-summary"><span id="result-count" role="status">显示 ${result.count} 款工具</span><span>${esc(freshness())}${checkedAt() ? ` · ${esc(checkedAt())}` : ''}</span></p><div class="library-list" id="library-list">${result.html}</div></section>`;
}

function updateLibraryResults() {
  const result = libraryRows();
  $('result-count').textContent = `显示 ${result.count} 款工具`;
  $('library-list').innerHTML = result.html;
}

const stageNames = {queued: '排队中', prepare: '准备安装', preflight: '检查安装条件', download: '下载', verify: '校验安装包', extract: '解压安装包', buildCandidate: '准备文件', candidate: '准备文件', commit: '提交文件', write: '写入安装目录', record: '保存安装记录', cleanup: '清理临时文件', rollback: '恢复旧文件', recover: '恢复安装', uninstall: '卸载', complete: '已完成', done: '已完成', selfUpdate: '本体更新'};
const kindNames = {install: '安装', update: '更新', reinstall: '重装', switchVersion: '切换版本', uninstall: '卸载', updateAll: '批量更新', toolboxUpdate: '工具箱更新', selfUpdate: '工具箱更新', downloadUpdate: '下载工具箱更新'};
const stateNames = {queued: '排队中', running: '处理中', succeeded: '已完成', failed: '失败', cancelled: '已取消', recoveryRequired: '需要恢复'};

function operationRow(operation) {
  const tool = findTool(operation.toolId);
  const batch = operation.kind === 'updateAll';
  const partial = batch && operation.result?.status === 'partial';
  const name = batch ? '批量更新' : tool?.name || (operation.toolId ? operation.toolId : '工具箱本体');
  const active = isActive(operation);
  const status = partial ? '部分完成' : stateNames[operation.state] || operation.state;
  const total = Number(operation.totalBytes);
  const done = Number(operation.doneBytes || 0);
  const percent = total > 0 ? Math.max(0, Math.min(100, Math.floor(done / total * 100))) : null;
  const progress = active ? `<progress ${percent == null ? '' : `value="${percent}" max="100"`} aria-label="${esc(name)}任务进度"></progress>` : '';
  let controls = '';
  if (active) controls += button(operation.canCancel ? '取消' : '正在提交', 'cancel-task', {id: operation.id, disabled: !operation.canCancel || ui.pending.has(`task:${operation.id}`)});
  const failedItems = batch && !active && operation.children?.some(child => ['failed', 'cancelled'].includes(child.state));
  if (failedItems) controls += button('重试失败项', 'retry-task', {id: operation.id, disabled: ui.pending.has(`task:${operation.id}`)});
  else if (['failed', 'cancelled', 'recoveryRequired'].includes(operation.state) && operation.error?.retryable !== false) controls += button('重试', 'retry-task', {id: operation.id, disabled: ui.pending.has(`task:${operation.id}`)});
  controls += button('日志', 'open-log', {id: operation.id});
  if (operation.state === 'succeeded' && !operation.toolId && state.toolbox.hasUpdate && ['toolboxUpdate', 'selfUpdate', 'downloadUpdate'].includes(operation.kind)) controls = button('应用更新并重启', 'apply-self', {id: operation.id}) + controls;
  const stage = active ? stageNames[operation.stage] || operation.stage || stateNames[operation.state] : status;
  const note = operation.error?.message || (partial ? operation.result?.message || '部分工具未完成，请查看各项结果。' : active && !operation.canCancel ? '正在提交或恢复文件，请等待任务完成。' : operation.state === 'succeeded' ? operation.result?.message || '' : '');
  return `<article class="pz-job" data-operation-id="${esc(operation.id)}"><div class="pz-jobline">${tool ? tile(tool, 'small') : icon('download', 28)}<div class="pz-grow"><strong>${esc(name)}</strong><div class="pz-meta"><span>${esc(kindNames[operation.kind] || operation.kind)}</span>${operation.version ? `<span>${esc(version(operation.version))}</span>` : ''}<span>${esc(stage)}</span></div></div><small>${active ? `${percent == null ? bytes(done) : `${percent}%`}${total > 0 ? ` · ${bytes(total)}` : ''}` : esc(status)}</small><div class="pz-actions">${controls}</div></div>${progress}${note ? `<p class="pz-jobnote ${operation.error ? 'error' : ''}">${esc(note)}</p>` : ''}${operation.children?.length ? `<div class="pz-jobnote">${operation.children.map(child => `<div>${esc(child.toolId || child.name || child.id || '')} · ${esc(stateNames[child.state] || child.state || '')}${child.error?.message ? ` · ${esc(child.error.message)}` : ''}</div>`).join('')}</div>` : ''}</article>`;
}

function catalogRow(tool) {
  const target = release(tool);
  const action = tool.installed ? 'update' : 'install';
  return `<article class="pz-row">${tile(tool)}<div class="pz-grow"><button type="button" class="pz-name" data-act="detail" data-id="${esc(tool.id)}">${esc(tool.name)}</button><div class="pz-meta"><span>${esc(tool.category)}</span><span>${esc(tool.installed ? `${version(tool.installedVersion)} → ${version(tool.latestVersion)}` : version(tool.latestVersion))}</span>${!allows(tool, action) && (target?.disabledReason || tool.disabledReason) ? `<span>${esc(target?.disabledReason || tool.disabledReason)}</span>` : ''}</div></div><div class="pz-release"><span>${esc(bytes(target?.sizeBytes))}</span><small>${esc(target?.date || '日期未提供')}</small></div>${button(allows(tool, action) ? tool.installed ? '更新' : '安装' : '暂不可用', 'request-operation', {id: tool.id, kind: action, selectedVersion: tool.latestVersion, disabled: !allows(tool, action) || ui.pending.has(`tool:${tool.id}`), title: target?.disabledReason || tool.disabledReason})}</article>`;
}

function renderUpdates() {
  const fresh = updates();
  const uninstalled = tools().filter(tool => !tool.installed);
  const eligible = fresh.filter(tool => allows(tool, 'update'));
  const tabs = [['updates', '可更新', fresh.length], ['available', '待安装', uninstalled.length], ['tasks', '任务记录', state.operations.length]];
  let body;
  if (ui.tab === 'tasks') {
    const list = [...state.operations.filter(isActive), ...state.operations.filter(item => !isActive(item)).reverse()];
    body = list.length ? `<div class="pz-list" id="operation-list">${list.map(operationRow).join('')}</div>` : empty('还没有安装任务', '从工具详情、可更新或待安装列表开始安装。');
  } else {
    const list = ui.tab === 'updates' ? fresh : uninstalled;
    body = list.length ? `<div class="pz-list">${list.map(catalogRow).join('')}</div>` : empty(ui.tab === 'updates' ? '当前没有可更新的工具' : '当前目录中的工具均已安装', state.catalog.source === 'empty' ? '检查工具目录后可查看新版本。' : '检查更新可以核对最新发布信息。');
  }
  return `<section class="page-wrap pz-page">${heading('安装与更新', '检查版本、管理安装包，跟踪每个工具的处理进度。', button(ui.checking ? '检查中…' : '检查更新', 'refresh', {disabled: ui.checking}) + button('全部更新', 'batch', {className: 'primary', disabled: !eligible.length || Boolean(activeTask()) || ui.checking}))}<nav class="pz-tabs" aria-label="安装状态">${tabs.map(([key, name, count]) => `<button type="button" data-tab="${key}" aria-pressed="${ui.tab === key}">${name} <span>${count}</span></button>`).join('')}</nav><div class="pz-source"><span class="pz-dot"></span>${esc(ui.checking ? '正在检查工具目录…' : freshness())}${checkedAt() ? ` · 上次检查 ${esc(checkedAt())}` : ''}</div>${body}</section>`;
}

function resetSettingsDraft() {
  if (!state.settings) return;
  ui.settingsDraft = {enabled: Boolean(state.settings.githubProxy?.enabled), nodes: (state.settings.githubProxy?.urls || []).join('\n'), directoryWarning: !state.settings.suppressDirectoryWarning, channel: state.settings.channel || 'stable'};
  ui.settingsBaseRevision = state.settings.revision;
  ui.settingsDirty = false;
}

function renderSettings() {
  if (!state.settings) return `<section class="page-wrap pz-page">${heading('设置', '读取下载连接与工作目录设置。')}${empty('设置尚未载入', '', button('重新载入', 'reconnect'))}</section>`;
  if (!ui.settingsDraft) resetSettingsDraft();
  const draft = ui.settingsDraft;
  const conflict = ui.settingsDirty && ui.settingsBaseRevision !== state.settings.revision;
  return `<section class="page-wrap pz-page">${heading('设置', '管理下载连接、工作目录提示与工具箱本体。')}<section class="pz-section"><h2>下载与目录</h2>${conflict ? `<div class="settings-conflict" role="status">设置已在其他位置发生变化。保留当前草稿，或重新载入最新设置。 ${button('重新载入设置', 'reload-settings')}</div>` : ''}<form id="toolbox-settings" class="pz-form"><label class="pz-check"><span><b>启用 GitHub 下载代理</b><small>用于目录和安装包下载。节点按填写顺序尝试。</small></span><input type="checkbox" name="enabled" role="switch" ${draft.enabled ? 'checked' : ''}></label><label class="pz-field">代理节点 <small>每行一个 HTTP 或 HTTPS 地址</small><textarea name="nodes" rows="4" spellcheck="false" autocapitalize="off" ${draft.enabled ? '' : 'disabled'}>${esc(draft.nodes)}</textarea></label><label class="pz-field">发布通道<select name="channel"><option value="stable" ${draft.channel === 'stable' ? 'selected' : ''}>稳定版</option><option value="beta" ${draft.channel === 'beta' ? 'selected' : ''}>测试版</option></select></label><label class="pz-check"><span><b>打开时检查工作目录</b><small>目录与桌面、下载或游戏文件混用时显示提醒。</small></span><input type="checkbox" name="directoryWarning" role="switch" ${draft.directoryWarning ? 'checked' : ''}></label><p id="settings-message" class="settings-status" role="status">${esc(ui.settingsMessage)}</p><div class="pz-actions"><button class="button primary pz-btn" type="submit" ${ui.pending.has('settings') ? 'disabled' : ''}>${ui.pending.has('settings') ? '正在保存…' : '保存设置'}</button>${button('恢复默认值', 'proxy-defaults', {disabled: ui.pending.has('settings')})}<span class="settings-stamp">${ui.settingsDirty ? '有未保存的更改' : '已与本地设置同步'}</span></div></form></section><section class="pz-section"><h2>工具箱版本</h2>${selfUpdateBody()}</section><section class="pz-section"><h2>工作目录</h2><div class="pz-split"><div class="pz-grow"><b>便携工具箱工作区</b><p>${esc(state.directoryWarning?.workspace || '与工具箱主程序位于同一目录')}</p></div>${button('打开目录', 'workspace')}</div></section></section>`;
}

function selfUpdateBody() {
  const toolbox = state.toolbox;
  const available = Boolean(toolbox.hasUpdate);
  const pending = state.operations.find(operation => !operation.toolId && ['toolboxUpdate', 'selfUpdate', 'downloadUpdate'].includes(operation.kind) && (isActive(operation) || operation.state === 'succeeded'));
  let action = button(ui.checking ? '检查中…' : '检查本体更新', 'self-check', {disabled: ui.checking});
  if (available && toolbox.canDownload) action += pending?.state === 'succeeded' ? button('应用更新并重启', 'apply-self', {id: pending.id, className: 'primary'}) : pending ? button('查看下载任务', 'show-tasks') : button(`下载 ${version(toolbox.latestVersion)}`, 'download-self', {className: 'primary', disabled: ui.pending.has('self-download')});
  if (available && toolbox.releaseUrl) action += button('查看发布页', 'link', {id: 'release'});
  return `<div class="pz-split self-update"><div class="pz-grow"><b>物理世界的工具箱 ${esc(version(toolbox.version))}</b><p>${available ? `发现新版 ${esc(version(toolbox.latestVersion))}${toolbox.sizeBytes != null ? ` · ${esc(bytes(toolbox.sizeBytes))}` : ''}` : '当前目录未提供更高的本体版本。'}</p>${available && !toolbox.canDownload ? '<p>本次更新从发布页获取。</p>' : ''}</div><div class="pz-actions">${action}</div></div>`;
}

function renderAbout() {
  const about = state.about;
  const links = [[about.githubUrl, 'GitHub 项目', 'project'], [about.feedbackUrl, '问题反馈', 'feedback']].filter(([url]) => url).map(([, label, target]) => button(label, 'link', {id: target})).join('');
  const communityLinks = `${about.bilibiliUrl ? button('作者 B 站', 'link', {id: 'author'}) : ''}${button('QQ频道', 'link', {id: 'qq-channel'})}`;
  const logs = (about.logs || []).map((log, index) => `<details class="pz-log" ${index === 0 ? 'open' : ''}><summary>${esc(version(log.version))} <span>${esc(log.date || '')}</span></summary>${notes(log.items)}</details>`).join('');
  const sponsors = (about.sponsorQrs || []).map(item => button(item.title, 'sponsor', {id: item.id})).join('');
  return `<section class="page-wrap pz-page">${heading('关于工具箱', about.tagline || 'DNF 工具集')}<section class="pz-section"><h2>${esc(about.title || '物理世界的工具箱')}</h2><p>在一个桌面入口安装、更新、启动和管理 DNF 工具。当前版本 ${esc(version(state.toolbox.version))}。</p>${about.author ? `<p>作者 · ${esc(about.author)}</p>` : ''}<div class="pz-actions">${links}<span class="pz-actions">${communityLinks}</span></div></section><section class="pz-section"><h2>工具箱更新记录</h2>${logs || '<p class="pz-fine">当前工作区没有更新记录。</p>'}</section>${sponsors ? `<section class="pz-section"><h2>赞助支持</h2><p>如果工具箱帮到了你，可以请作者喝杯咖啡。</p><div class="pz-actions">${sponsors}</div></section>` : ''}<section class="pz-section"><h2>本地运行</h2><p>界面与静态资源随工具箱程序提供。目录缓存和已安装工具可离线读取，安装与更新由本地宿主管理。</p>${selfUpdateBody()}</section></section>`;
}

function notes(lines) {
  const entries = (Array.isArray(lines) ? lines : [lines]).filter(Boolean);
  return entries.length ? `<ul class="pz-notes">${entries.map(line => `<li>${esc(line)}</li>`).join('')}</ul>` : '<p class="pz-fine">此版本未附更新说明。</p>';
}

function connectionState() {
  const error = ui.error;
  return `<section class="connection-state" role="${error ? 'alert' : 'status'}"><h1>${error ? '无法读取工具箱状态' : '正在打开工具箱'}</h1><p>${esc(error?.message || '读取本地目录、安装记录和任务状态…')}</p>${error ? `${button('重新载入状态', 'reconnect')}<p class="error-code">${esc(error.code || 'CONNECTION')}</p>` : ''}</section>`;
}

function rememberFocus() {
  const active = document.activeElement;
  if (!active || active === document.body) return null;
  if (active.id) return {id: active.id, start: active.selectionStart, end: active.selectionEnd};
  const field = active.name ? {name: active.name, form: active.form?.id, start: active.selectionStart, end: active.selectionEnd} : null;
  if (field) return field;
  for (const attribute of ['data-act', 'data-page', 'data-category', 'data-tab']) {
    if (active.hasAttribute(attribute)) return {attribute, value: active.getAttribute(attribute), toolId: active.dataset.id, dialog: Boolean(active.closest('dialog'))};
  }
  return null;
}

function restoreFocus(focus) {
  if (!focus) return;
  let element;
  if (focus.id) element = $(focus.id);
  else if (focus.name) element = $(focus.form)?.elements.namedItem(focus.name);
  else {
    const root = focus.dialog ? $('dialog') : document;
    element = [...root.querySelectorAll(`[${focus.attribute}]`)].find(item => item.getAttribute(focus.attribute) === focus.value && item.dataset.id === focus.toolId);
  }
  if (element && !element.disabled) {
    element.focus({preventScroll: true});
    if (Number.isInteger(focus.start) && typeof element.setSelectionRange === 'function' && element.type !== 'checkbox') {
      try { element.setSelectionRange(focus.start, focus.end); } catch {}
    }
  }
}

function syncHomeCards(grid, nextGrid) {
  const cards = new Map([...grid.children].map(card => [card.dataset.id, card]));
  let cursor = grid.firstElementChild;
  for (const next of [...nextGrid.children]) {
    const card = cards.get(next.dataset.id) || next;
    for (const name of ['aria-pressed', 'title']) {
      if (card.getAttribute(name) !== next.getAttribute(name)) card.setAttribute(name, next.getAttribute(name));
    }
    // Selection and preference acknowledgements leave these descendants intact,
    // including the img/strong/svg that the browser uses as a dblclick target.
    if (card.innerHTML !== next.innerHTML) card.innerHTML = next.innerHTML;
    if (card !== cursor) grid.insertBefore(card, cursor);
    cursor = card.nextElementSibling;
  }
  while (cursor) {const next = cursor.nextElementSibling; cursor.remove(); cursor = next;}
}

function syncHomeSurface(html) {
  const surface = $('surface');
  const next = document.createElement('template');
  next.innerHTML = html;
  const grid = surface.querySelector('.pin-grid');
  const nextGrid = next.content.querySelector('.pin-grid');
  if (!grid || !nextGrid) {surface.innerHTML = html; return;}
  const hero = surface.querySelector('.hero');
  const nextHero = next.content.querySelector('.hero');
  if (hero.outerHTML !== nextHero.outerHTML) hero.replaceWith(nextHero);
  syncHomeCards(grid, nextGrid);
  grid.parentElement.querySelector('.count').textContent = nextGrid.parentElement.querySelector('.count').textContent;
  const news = grid.parentElement.nextElementSibling;
  const nextNews = nextGrid.parentElement.nextElementSibling;
  if (news.innerHTML !== nextNews.innerHTML) news.innerHTML = nextNews.innerHTML;
}

function render() {
  const active = document.activeElement;
  const focus = rememberFocus();
  selectedTool();
  syncChrome();
  const views = {home: renderHome, library: renderLibrary, updates: renderUpdates, settings: renderSettings, about: renderAbout};
  const html = ui.loading || !state.sessionId ? connectionState() : views[ui.page]();
  if (ui.page === 'home' && $('surface').firstElementChild?.classList.contains('hero')) syncHomeSurface(html);
  else $('surface').innerHTML = html;
  if (ui.modal) renderModal();
  if (!active?.isConnected) restoreFocus(focus);
}

let renderQueued = false;
function queueRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => {renderQueued = false; render();});
}

function pageFromHash() {
  const page = location.hash.slice(1);
  return Object.hasOwn(pages, page) ? page : 'home';
}

function go(page) {
  if (!Object.hasOwn(pages, page)) return;
  closeModal();
  if (location.hash !== `#${page}`) history.pushState(null, '', `#${page}`);
  ui.page = page;
  render();
  checkUpdatesOnEntry();
  window.scrollTo(0, 0);
  $('surface').focus({preventScroll: true});
}

function openModal(type, data = {}) {
  if (!$('dialog').open) ui.returnFocus = document.activeElement;
  ui.modal = {type, ...data};
  renderModal();
  if (!$('dialog').open) $('dialog').showModal();
  $('dialog').scrollTop = 0;
  $('dialog-close').focus({preventScroll: true});
}

function closeModal() {
  ui.modal = null;
  if ($('dialog').open) $('dialog').close();
}

function versionBody(tool, chosen) {
  const entry = release(tool, chosen);
  const current = chosen === tool.installedVersion;
  const kind = !tool.installed ? 'install' : current ? 'reinstall' : chosen === tool.latestVersion && tool.hasUpdate ? 'update' : 'switchVersion';
  const canInstall = Boolean(entry?.canInstall && allows(tool, kind));
  const label = !tool.installed ? '安装此版本' : current ? '重新安装此版本' : kind === 'update' ? '更新至此版本' : '切换至此版本';
  return `<div class="pz-versionmeta"><span>${esc(entry?.date || '日期未提供')}</span><span>${esc(bytes(entry?.sizeBytes))}</span>${entry?.channel ? `<span>${esc(entry.channel === 'stable' ? '稳定版' : entry.channel === 'beta' ? '测试版' : entry.channel)}</span>` : ''}${tool.needAdmin ? '<span>工具运行可能需要管理员权限</span>' : ''}</div>${notes(entry?.changelog)}${!canInstall ? `<p class="pz-error">${esc(entry?.disabledReason || tool.disabledReason || (entry?.blocked ? '发布方已停用此版本。' : !entry?.compatible ? '当前工具箱不满足此版本要求。' : '此版本当前不可安装。'))}</p>` : ''}<div class="pz-actions">${button(label, 'request-operation', {id: tool.id, selectedVersion: chosen, kind, className: 'primary', disabled: !canInstall || ui.pending.has(`tool:${tool.id}`)})}${tool.installed ? button('启动已安装版本', 'launch', {id: tool.id, disabled: !allows(tool, 'launch') || ui.pending.has(`tool:${tool.id}`)}) : ''}${pins().includes(tool.id) ? button('在首页查看', 'focus', {id: tool.id}) : ''}</div>`;
}

function renderModal() {
  if (!ui.modal) return;
  const modal = ui.modal;
  const tool = findTool(modal.id);
  let title = '';
  let body = '';
  if (modal.type === 'detail' && tool) {
    title = tool.name;
    const chosen = modal.selectedVersion || tool.latestVersion || tool.installedVersion;
    body = `<div class="pz-intro">${tile(tool, 'large')}<div><b>${esc(tool.category)}</b><p>${tool.installed ? `已安装 ${esc(version(tool.installedVersion))}` : '尚未安装'}</p></div></div><p>${esc(tool.description || '此工具暂未提供简介。')}</p>${tool.disabledReason ? `<p class="pz-error">${esc(tool.disabledReason)}</p>` : ''}${tool.availableVersions?.length ? `<label class="pz-field">版本详情<select id="tool-versions" data-id="${esc(tool.id)}">${tool.availableVersions.map(entry => `<option value="${esc(entry.version)}" ${entry.version === chosen ? 'selected' : ''}>${esc(version(entry.version))}${entry.version === tool.latestVersion ? ' · 最新版' : ''}${entry.version === tool.installedVersion ? ' · 已安装' : ''}${entry.blocked ? ' · 已禁用' : ''}</option>`).join('')}</select></label><div class="pz-versionbody">${versionBody(tool, chosen)}</div>` : '<p class="pz-fine">本地尚无可选版本。检查更新可获取版本清单。</p>'}<div class="pz-actions">${pinButton(tool)}${button('更多操作', 'more', {id: tool.id})}</div>`;
  } else if (modal.type === 'more' && tool) {
    title = `${tool.name} · 更多操作`;
    const menu = (label, action, actionName = '', className = '') => button(label, action, {id: tool.id, className: `pz-menuitem ${className}`, disabled: actionName ? !allows(tool, actionName) : false, title: actionName && !allows(tool, actionName) ? tool.disabledReason || '当前状态不可执行此操作。' : ''});
    body = `<div class="pz-menu">${pins().includes(tool.id) ? menu('在首页查看', 'focus') : ''}${menu('版本详情与切换', 'detail')}${tool.hasUpdate ? button(`更新至 ${version(tool.latestVersion)}`, 'request-operation', {id: tool.id, kind: 'update', selectedVersion: tool.latestVersion, className: 'pz-menuitem', disabled: !allows(tool, 'update')}) : ''}${pinButton(tool, {menu: true})}${menu('打开安装目录', 'open-folder', 'openFolder')}${menu('复制项目地址', 'copy-project', 'copyProjectUrl')}${menu('卸载工具', 'uninstall', 'uninstall', 'pz-danger')}</div>`;
  } else if (modal.type === 'confirm' && tool) {
    const labels = {uninstall: '卸载工具', reinstall: '重新安装此版本', switchVersion: '切换工具版本'};
    title = labels[modal.kind] || '确认工具操作';
    const workspace = state.directoryWarning?.workspace || '';
    body = modal.kind === 'uninstall' ? `<p>卸载 ${esc(tool.name)} ${esc(version(tool.installedVersion))}，移除下列工具目录及安装记录。</p><span class="pz-path">${esc(tool.installDirectory || (workspace ? `${workspace.replace(/[\\/]+$/, '')}\\tools\\${tool.id}` : `tools\\${tool.id}`))}</span><p>目录内的 config、data 和其他个人文件会一并移除。请先备份需要保留的文件。</p>` : `<p>${modal.kind === 'reinstall' ? '重新安装' : '将'} ${esc(tool.name)} ${modal.kind === 'switchVersion' ? `从 ${esc(version(tool.installedVersion))} 切换为` : ''} ${esc(version(modal.selectedVersion))}。</p><p class="pz-fine">保留工具的配置与数据，其他程序文件会按安装包更新。</p>`;
    body += `<div class="pz-actions">${button('取消', 'close')}${button(modal.kind === 'uninstall' ? '确认卸载' : modal.kind === 'reinstall' ? '确认重装' : '确认切换', 'confirm-operation', {id: tool.id, kind: modal.kind, selectedVersion: modal.selectedVersion, className: modal.kind === 'uninstall' ? 'pz-danger' : 'primary', disabled: ui.pending.has(`tool:${tool.id}`)})}</div>`;
  } else if (modal.type === 'batch') {
    title = '更新已安装工具';
    const eligible = updates().filter(item => allows(item, 'update'));
    const totalKnown = eligible.every(item => release(item)?.sizeBytes != null);
    body = `<p>依次更新 ${eligible.length} 款工具${totalKnown ? `，合计 ${esc(bytes(eligible.reduce((sum, item) => sum + Number(release(item).sizeBytes), 0)))}` : ''}。</p><ul class="pz-notes">${eligible.map(item => `<li>${esc(item.name)} · ${esc(version(item.installedVersion))} → ${esc(version(item.latestVersion))}</li>`).join('')}</ul><div class="pz-actions">${button('取消', 'close')}${button('开始更新', 'confirm-batch', {className: 'primary', disabled: !eligible.length || ui.pending.has('batch')})}</div>`;
  } else if (modal.type === 'self') {
    title = '工具箱本体更新';
    body = `${selfUpdateBody()}${state.toolbox.hasUpdate ? notes(state.toolbox.changelog) : ''}`;
  } else if (modal.type === 'apply-self') {
    title = '应用工具箱更新';
    body = `<p>更新到 ${esc(version(state.toolbox.latestVersion))}。工具箱将关闭并重新打开，已安装工具及配置会保留。</p>${activeTask() ? '<p class="native-close-note">请等待当前安装任务完成后再应用更新。</p>' : ''}<div class="pz-actions">${button('稍后', 'close')}${button('应用更新并重启', 'confirm-apply-self', {id: modal.id, className: 'primary', disabled: Boolean(activeTask()) || ui.pending.has('self-apply')})}</div>`;
  } else if (modal.type === 'sponsor') {
    const sponsor = state.about.sponsorQrs?.find(item => item.id === modal.id);
    const source = imageUrl(sponsor?.imageUrl);
    title = sponsor?.title || '赞助支持';
    body = source ? `<img class="pz-qr" src="${esc(source)}" alt="${esc(title)}二维码">` : '<p>当前版本没有提供此二维码。</p>';
  } else if (modal.type === 'busy-close') {
    title = '任务尚未完成';
    const cancellable = state.operations.filter(isActive).every(operation => operation.canCancel);
    body = `<p>${cancellable ? '有安装或下载任务正在进行。取消任务后，工具箱会等待文件状态明确再退出。' : '正在提交或恢复文件，完成后才能安全退出工具箱。'}</p><div class="pz-actions">${button('继续等待', 'close')}${button('查看任务', 'show-tasks')}${cancellable ? button('取消任务并退出', 'cancel-close', {className: 'pz-danger', disabled: ui.pending.has('window-close')}) : ''}</div>`;
  } else if (modal.type === 'error') {
    title = '操作未完成';
    body = `<p>${esc(modal.error?.message || '请重试。')}</p><p class="error-code">${esc(modal.error?.code || '')}</p><div class="pz-actions">${modal.error?.operationId ? button('查看任务', 'show-tasks') + button('打开日志', 'open-log', {id: modal.error.operationId}) : ''}${modal.error?.outcomeUnknown ? button('重新载入状态', 'reconnect') : ''}${button('关闭', 'close')}</div>`;
  } else {
    title = '工具信息已变化';
    body = '<p>这个工具已不在当前目录中，请返回工具库查看。</p>';
  }
  $('dialog-title').textContent = title;
  $('dialog-body').innerHTML = `<div class="pz-dialog">${body}<p id="dialog-error" class="dialog-error" role="alert">${esc(modal.errorMessage || '')}</p></div>`;
}

function showError(error) {
  if (ui.modal && ui.modal.type !== 'error') {
    ui.modal.errorMessage = error.message;
    renderModal();
  } else openModal('error', {error});
}

async function run(key, action) {
  if (ui.pending.has(key)) return;
  ui.pending.add(key);
  queueRender();
  try { return await action(); }
  catch (error) {
    if (error.code === 'STALE_REVISION') await refreshSnapshot();
    if (error.outcomeUnknown) await refreshSnapshot().catch(() => {});
    showError(error);
  } finally {ui.pending.delete(key); queueRender();}
}

let refreshPromise = null;
let eventBuffer = [];
let unsubscribe = null;
let windowNotification = 0;
let readySession = null;
let catalogCheckSession = null;
function checkUpdatesOnEntry() {
  if (ui.loading || !state.sessionId || ui.page !== 'updates' || catalogCheckSession === state.sessionId) return;
  catalogCheckSession = state.sessionId;
  const sessionCheck = state.operations.some(operation => ['refresh', 'catalogRefresh', 'catalog.refresh'].includes(operation.kind) && operation.request?.sessionId === state.sessionId);
  if (!ui.checking && !sessionCheck) refreshCatalog(false, 'updatesPage');
}
async function refreshSnapshot() {
  if (refreshPromise) return refreshPromise;
  refreshPromise = (async () => {
    const snapshot = await bridge.dispatch('app.bootstrap');
    applyBootstrap(state, snapshot);
    ui.loading = false;
    ui.error = null;
    if (!ui.settingsDirty) resetSettingsDraft();
    const queued = eventBuffer;
    eventBuffer = [];
    for (const message of queued) {
      if (message.sessionId === state.sessionId && message.seq <= state.seq) continue;
      const result = applyEvent(state, message);
      if (result.resync) {ui.error = {message: '状态已变化，正在重新同步。'}; break;}
    }
    render();
    if (readySession !== state.sessionId) {
      await new Promise(resolve => requestAnimationFrame(resolve));
      await bridge.dispatch('app.ready', {sessionId: state.sessionId});
      readySession = state.sessionId;
    }
  })();
  try {await refreshPromise;} finally {refreshPromise = null;}
  const arrivedDuringReady = eventBuffer;
  eventBuffer = [];
  for (const message of arrivedDuringReady) handleEvent(message);
  if (ui.error?.message === '状态已变化，正在重新同步。') return refreshSnapshot();
  checkUpdatesOnEntry();
}

function handleEvent(message) {
  if (refreshPromise || !state.sessionId) {eventBuffer.push(message); return;}
  const result = applyEvent(state, message);
  if (result.resync) {refreshSnapshot().catch(connectionError); return;}
  if (result.ignored) return;
  if (result.changed === 'window.changed') {
    windowNotification++;
    syncWindowChrome();
    if (message.data.closeRequested) openModal('busy-close');
    return;
  }
  if (result.changed === 'settings.changed' && !ui.settingsDirty) resetSettingsDraft();
  queueRender();
  if (result.terminal) refreshSnapshot().catch(connectionError);
}

function connectionError(error) {
  ui.loading = false;
  ui.error = error;
  render();
}

async function connect() {
  ui.loading = !state.sessionId;
  ui.error = null;
  render();
  try {
    if (!unsubscribe) unsubscribe = await bridge.listen(handleEvent);
    await refreshSnapshot();
    const notification = windowNotification;
    const windowState = await bridge.dispatch('window.state');
    if (notification === windowNotification) Object.assign(state.window, windowState);
    syncWindowChrome();
  } catch (error) {connectionError(error);}
}

async function refreshCatalog(showSelf = false, reason = 'manual') {
  if (state.sessionId) catalogCheckSession = state.sessionId;
  if (ui.checking) return;
  ui.checking = true;
  render();
  try {
    await bridge.dispatch('catalog.refresh', {reason});
    await refreshSnapshot();
    if (showSelf) openModal('self');
    else if (reason !== 'updatesPage') toast('已检查工具目录。');
  } catch (error) {showError(error);}
  finally {ui.checking = false; render();}
}

let selectionRequest = null;
async function focusTool(id, {preservePosition = false} = {}) {
  if (!findTool(id) || !pins().includes(id)) return;
  const sessionId = state.sessionId;
  const homeCard = preservePosition && ui.page === 'home';
  const pendingHere = selectionRequest?.sessionId === sessionId;
  if (homeCard && ui.selectedId === id && (pendingHere ? selectionRequest.id === id : state.preferences.lastSelectedToolId === id)) return;
  const request = {id, sessionId};
  selectionRequest = request;
  ui.selectedId = id;
  if (homeCard) render();
  else go('home');
  try {
    const preferences = await bridge.dispatch('preferences.selectTool', {toolId: id});
    if (sessionId === state.sessionId) applyPreferences(state, preferences);
  }
  catch (error) {toast(error.message);}
  finally {if (selectionRequest === request) selectionRequest = null;}
}

async function pinTool(id) {
  if (!findTool(id)) return;
  await run(`pin:${id}`, async () => {
    const sessionId = state.sessionId;
    const pinned = !pins().includes(id);
    const preferences = await bridge.dispatch('preferences.setPinned', {toolId: id, pinned});
    if (sessionId !== state.sessionId) return;
    applyPreferences(state, preferences);
    selectedTool();
    toast(pinned ? '已固定到首页。' : '已取消首页固定。');
  });
}

async function launchTool(id) {
  if (!allows(findTool(id), 'launch')) return;
  await run(`tool:${id}`, async () => {
    await bridge.dispatch('tool.launch', {toolId: id});
    toast(`已发起启动：${findTool(id)?.name || id}`);
  });
}

function requestOperation(kind, id, selectedVersion) {
  const tool = findTool(id);
  if (!allows(tool, kind)) return;
  if (['uninstall', 'reinstall', 'switchVersion'].includes(kind)) openModal('confirm', {id, kind, selectedVersion});
  else startOperation(kind, id, selectedVersion);
}

async function startOperation(kind, id, selectedVersion) {
  return run(id ? `tool:${id}` : 'batch', async () => {
    const params = {kind, revision: state.catalog.revision};
    if (id) params.toolId = id;
    if (selectedVersion) params.version = selectedVersion;
    if (['uninstall', 'reinstall', 'switchVersion', 'updateAll'].includes(kind)) params.confirmed = true;
    await bridge.dispatch('operation.start', params);
    ui.tab = 'tasks';
    closeModal();
    go('updates');
    await refreshSnapshot();
  });
}

async function taskAction(id, action) {
  return run(`task:${id}`, async () => {
    await bridge.dispatch(`operation.${action}`, {operationId: id});
    await refreshSnapshot();
    toast(action === 'cancel' ? '已请求取消任务。' : '已重新提交任务。');
  });
}

function readSettingsDraft(form) {
  ui.settingsDraft = {enabled: form.elements.enabled.checked, nodes: form.elements.nodes.value, directoryWarning: form.elements.directoryWarning.checked, channel: form.elements.channel.value};
  ui.settingsDirty = true;
  ui.settingsMessage = '';
  const stamp = form.querySelector('.settings-stamp');
  if (stamp) stamp.textContent = '有未保存的更改';
}

async function saveSettings(form) {
  readSettingsDraft(form);
  const draft = ui.settingsDraft;
  const urls = [...new Set(draft.nodes.split(/\r?\n/).map(value => value.trim()).filter(Boolean))];
  if (draft.enabled && (!urls.length || urls.some(value => {try {return !['http:', 'https:'].includes(new URL(value).protocol);} catch {return true;}}))) {
    ui.settingsMessage = '请输入至少一个有效的 HTTP 或 HTTPS 代理地址。';
    $('settings-message').textContent = ui.settingsMessage;
    form.elements.nodes.focus();
    return;
  }
  await run('settings', async () => {
    state.settings = await bridge.dispatch('settings.update', {patch: {githubProxy: {enabled: draft.enabled, urls}, suppressDirectoryWarning: !draft.directoryWarning, channel: draft.channel}, revision: ui.settingsBaseRevision});
    resetSettingsDraft();
    ui.settingsMessage = '设置已保存。';
    toast('设置已保存。');
  });
}

async function nativeWindow(action, cancelTasks = false) {
  try {
    const result = await bridge.dispatch(`window.${action}`, cancelTasks ? {cancelTasks: true} : {});
    if (typeof result?.maximized === 'boolean') state.window.maximized = result.maximized;
    if (typeof result?.active === 'boolean') state.window.active = result.active;
    syncWindowChrome();
  }
  catch (error) {
    if (action === 'requestClose' && error.code === 'BUSY') openModal('busy-close');
    else showError(error);
  }
}

async function handleAction(element) {
  const action = element.dataset.act;
  const id = element.dataset.id;
  switch (action) {
    case 'primary': {
      const tool = findTool(id);
      if (!tool) return;
      const primary = primaryAction(tool, state.operations);
      if (primary.disabled) return;
      return primary.action === 'launch' ? launchTool(id) : requestOperation(primary.action, id, primary.version || tool.latestVersion || tool.installedVersion);
    }
    case 'launch': return launchTool(id);
    case 'focus': return focusTool(id, {preservePosition: element.classList?.contains('pin-card')});
    case 'pin': return pinTool(id);
    case 'detail': return openModal('detail', {id});
    case 'more': return openModal('more', {id});
    case 'request-operation': return requestOperation(element.dataset.kind, id, element.dataset.version);
    case 'confirm-operation': return startOperation(element.dataset.kind, id, element.dataset.version);
    case 'uninstall': return requestOperation('uninstall', id);
    case 'batch': return openModal('batch');
    case 'confirm-batch': return startOperation('updateAll');
    case 'cancel-task': return taskAction(id, 'cancel');
    case 'retry-task': return taskAction(id, 'retry');
    case 'open-log': return run(`log:${id}`, () => bridge.dispatch('system.openLog', {operationId: id}));
    case 'open-folder': return run(`folder:${id}`, () => bridge.dispatch('tool.openFolder', {toolId: id}));
    case 'copy-project': return run(`copy:${id}`, async () => {await bridge.dispatch('tool.copyProjectUrl', {toolId: id}); toast('项目地址已复制。');});
    case 'workspace': return run('workspace', () => bridge.dispatch('system.openWorkspace'));
    case 'link': return run(`link:${id}`, () => bridge.dispatch('system.openLink', {target: id}));
    case 'sponsor': return openModal('sponsor', {id});
    case 'refresh': return refreshCatalog();
    case 'self-check': return refreshCatalog(true);
    case 'download-self': return run('self-download', async () => {await bridge.dispatch('toolbox.downloadUpdate', {version: state.toolbox.latestVersion}); closeModal(); ui.tab = 'tasks'; go('updates'); await refreshSnapshot();});
    case 'apply-self': return openModal('apply-self', {id});
    case 'confirm-apply-self': return run('self-apply', () => bridge.dispatch('toolbox.applyUpdate', {operationId: id, confirmed: true}));
    case 'show-tasks': ui.tab = 'tasks'; return go('updates');
    case 'settings-page': return go('settings');
    case 'reload-settings': resetSettingsDraft(); ui.settingsMessage = ''; return render();
    case 'proxy-defaults': return run('defaults', async () => {
      const defaults = await bridge.dispatch('settings.proxyDefaults');
      ui.settingsDraft.nodes = (Array.isArray(defaults) ? defaults : defaults.urls || defaults.githubProxy?.urls || []).join('\n');
      ui.settingsDraft.enabled = true;
      ui.settingsDirty = true;
      ui.settingsMessage = '已填入默认节点，保存后生效。';
    });
    case 'dismiss-warning': ui.dismissedWarning = true; return renderNotice();
    case 'reconnect': closeModal(); return connect();
    case 'reset': ui.query = ''; ui.category = '全部'; ui.filter = 'all'; render(); return $('tool-search')?.focus();
    case 'close': return closeModal();
    case 'cancel-close': return run('window-close', () => nativeWindow('requestClose', true));
  }
}

$('global-search').innerHTML = `${icon('search', 16)}<span>搜索工具</span><kbd>Ctrl K</kbd>`;
$('window-minimize').innerHTML = icon('minimize', 14);
$('window-close').innerHTML = icon('close', 17);
$('dialog-close').innerHTML = icon('close', 18);
$('dialog-close').addEventListener('click', closeModal);
$('dialog').addEventListener('cancel', () => {ui.modal = null;});
$('dialog').addEventListener('close', () => {
  ui.modal = null;
  if (ui.returnFocus?.isConnected) ui.returnFocus.focus({preventScroll: true});
  else $('surface').focus({preventScroll: true});
});
$('dialog').addEventListener('click', event => {
  if (event.target !== $('dialog')) return;
  const rect = $('dialog').getBoundingClientRect();
  if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) closeModal();
});
document.addEventListener('click', event => {
  const element = event.target.closest('[data-page],[data-act],[data-category],[data-tab],[data-window]');
  if (!element || element.disabled) return;
  if (element.hasAttribute('data-window')) {nativeWindow(element.dataset.window); return;}
  if (element.hasAttribute('data-page')) {go(element.dataset.page); return;}
  if (element.hasAttribute('data-category')) {ui.category = element.dataset.category; render(); return;}
  if (element.hasAttribute('data-tab')) {ui.tab = element.dataset.tab; render(); return;}
  handleAction(element).catch(error => showError(error));
});
document.addEventListener('dblclick', event => {
  const card = event.target.closest('.pin-card[data-act="focus"]');
  if (ui.page !== 'home' || !card || card.disabled || !$('surface').contains(card) || !pins().includes(card.dataset.id)) return;
  const tool = findTool(card.dataset.id);
  if (!tool) return;
  const primary = primaryAction(tool, state.operations);
  if (primary.action === 'launch' && !primary.disabled) launchTool(tool.id).catch(showError);
});
document.addEventListener('input', event => {
  if (event.target.id === 'tool-search') {ui.query = event.target.value; updateLibraryResults();}
  if (event.target.form?.id === 'toolbox-settings') readSettingsDraft(event.target.form);
});
document.addEventListener('change', event => {
  if (event.target.id === 'install-filter') {ui.filter = event.target.value; updateLibraryResults();}
  if (event.target.id === 'tool-versions' && ui.modal) {ui.modal.selectedVersion = event.target.value; renderModal(); $('tool-versions').focus({preventScroll: true});}
  if (event.target.form?.id === 'toolbox-settings') {
    readSettingsDraft(event.target.form);
    if (event.target.name === 'enabled') event.target.form.elements.nodes.disabled = !event.target.checked;
  }
});
document.addEventListener('submit', event => {
  if (event.target.id !== 'toolbox-settings') return;
  event.preventDefault();
  saveSettings(event.target).catch(showError);
});
function search() {go('library'); $('tool-search')?.focus(); $('tool-search')?.select();}
$('global-search').addEventListener('click', search);
document.addEventListener('keydown', event => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {event.preventDefault(); search();}
  if (event.key === 'Escape' && $('dialog').open) closeModal();
});
window.addEventListener('popstate', () => {closeModal(); ui.page = pageFromHash(); render(); checkUpdatesOnEntry(); window.scrollTo(0, 0);});
window.addEventListener('scroll', syncScrollChrome, {passive: true});
window.addEventListener('beforeunload', () => {if (unsubscribe) unsubscribe();});
document.addEventListener('error', event => {
  const image = event.target;
  if (image?.tagName !== 'IMG' || !image.hasAttribute('data-icon-fallback')) return;
  const fallback = document.createElement('span');
  fallback.className = 'tool-initial';
  fallback.textContent = image.getAttribute('data-icon-fallback');
  image.replaceWith(fallback);
}, true);

connect();
