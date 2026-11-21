# 前端验收

生产入口是 `frontend/index.html`，Tauri `frontendDist` 只嵌入 `frontend/`。它不包含工具目录样本、安装记录、任务定时器、fixture 或远程 CDN；缺少 Tauri 桥时会显示宿主连接错误，不会创建演示状态。

```powershell
node --test tests/*.test.mjs
node tests/browser-fixture.mjs --port=4173
```

在 Codex 内置浏览器打开 `http://127.0.0.1:4173`。本地服务只在响应 HTML 时插入测试桥，磁盘上的生产 HTML 不变。此桥记录请求，模拟回包和事件；不启动程序、不安装文件、不操作剪贴板、不打开外链，不证明原生后端行为。默认五个合同样本只存在于 `tests/browser-fixture.mjs`；未固定的阿拉德调色保留为旧选中偏好，用于确认它不会进入首页。

可选场景：`--scenario=empty`、`--scenario=no-pins`、`--scenario=offline`、`--scenario=blocked`。`no-pins` 保留完整目录，首页显示前往工具库固定的空状态。使用真实 Rust `app.bootstrap` 捕获数据时：`node tests/browser-fixture.mjs --snapshot=E:\完整路径\bootstrap.json --port=4173`。输入可以是 bootstrap 对象或带 `result` 的成功回包，后续操作仍由 fixture 模拟。

测试端点：

- `GET /__fixture/state` 查看当前快照。
- `GET /__fixture/requests` 查看发送过的完整请求（包括 app.ready）。
- `POST /__fixture/event`，JSON 为 `{event,data}`，自动分配同会话递增序号并应用目录、设置、偏好、本体、单任务事件。

独立 QA 应先用真实 Rust 捕获验证页面数据，必要时把上述测试桥替换为受控本机转发；文件事务、下载、进程、剪贴板、原生退出和自更新必须另测真实宿主。

可稳定定位的界面属性：`[data-page]`、`[data-act]`、`[data-tool-id]`、`[data-operation-id]`。主要检查首页固定与切换、搜索/分类/状态、版本禁用和切换确认、卸载路径与数据提示、真实任务的不定进度/失败/取消/重试、设置草稿与 revision 冲突、本体仅发布页和完整更新包两条路径、`window.changed {closeRequested:true}` 的忙退出确认。所有窗口尺寸和交互由 Codex 内置浏览器验证，禁止 Computer Use。

首页轮播、固定卡与版本动态共用可用的固定工具列表，并保持固定顺序；旧偏好、已下架 ID 不会引入其他工具。取消当前项回到剩余固定项，取消最后一项显示空状态，工具库与更新页保留完整目录。超过五个固定项时，轮播使用前后按钮与当前/总数，避免分页点挤占启动按钮。图钉在各宽度保留可操作性，固定时显示实心黄色状态。`home-pins.test.mjs` 还验证并行偏好响应和事件不会回退到旧 revision，旧会话回包不会覆盖重连后的偏好。

首页固定卡单击立即切换当前工具，并保留长列表中的滚动位置和卡片焦点；普通选择及偏好事件重绘会保留原卡片及其图片、文字等点击节点。双击遵循浏览器原生 `dblclick`，仅对当前可启动且没有活动任务的固定工具发送一次 `tool.launch`；不会触发安装或修复。工具库维持原有详情和按钮交互。`home-pins.test.mjs` 覆盖单击与键盘激活、偏好事件、跨卡快速点击、启动去重、不可启动状态和卡片节点复用。浏览器验收需在长固定列表下滚后真实单击、双击图片和名称，并核对滚动、焦点及测试桥请求。

## 已执行的快速冒烟

2026-11-21：Codex 内置浏览器 iab，测试桥 normal 场景。

- 工作台视觉与本地图片、工具库中文搜索、版本详情、版本切换确认已通过。
- 未知总大小的任务显示不定进度；提交 `operation.start` 后能显示任务，取消后显示终态与重试。
- 代理地址本地校验通过；设置草稿在切换至关于页再返回后保留。
- 390、768、1440 宽度的工作台无横向溢出，390 宽设置页无横向溢出；最终 1440 截图的图片均加载成功。
- 浏览器控制台无 error/warn。`GET /__fixture/requests` 核对了 app.bootstrap、app.ready、带 revision/version/confirmed 的切换请求与取消请求。
- 截图：`target/frontend-validation/home-390.png`、`home-768.png`、`home-1440.png`。这些是快速冒烟证据；不替代独立 QA 与真实 Rust 宿主验收。
