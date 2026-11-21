# 前端验收

生产入口是 `frontend/index.html`，Tauri `frontendDist` 只嵌入 `frontend/`。它不包含工具目录样本、安装记录、任务定时器、fixture 或远程 CDN；缺少 Tauri 桥时会显示宿主连接错误，不会创建演示状态。

```powershell
node --test tests/frontend-contract.test.mjs
node tests/browser-fixture.mjs --port=4173
```

在 Codex 内置浏览器打开 `http://127.0.0.1:4173`。本地服务只在响应 HTML 时插入测试桥，磁盘上的生产 HTML 不变。此桥记录请求，模拟回包和事件；不启动程序、不安装文件、不操作剪贴板、不打开外链，不证明原生后端行为。默认四个合同样本只存在于 `tests/browser-fixture.mjs`。

可选场景：`--scenario=empty`、`--scenario=offline`、`--scenario=blocked`。使用真实 Rust `app.bootstrap` 捕获数据时：`node tests/browser-fixture.mjs --snapshot=E:\完整路径\bootstrap.json --port=4173`。输入可以是 bootstrap 对象或带 `result` 的成功回包，后续操作仍由 fixture 模拟。

测试端点：

- `GET /__fixture/state` 查看当前快照。
- `GET /__fixture/requests` 查看发送过的完整请求（包括 app.ready）。
- `POST /__fixture/event`，JSON 为 `{event,data}`，自动分配同会话递增序号并应用目录、设置、偏好、本体、单任务事件。

独立 QA 应先用真实 Rust 捕获验证页面数据，必要时把上述测试桥替换为受控本机转发；文件事务、下载、进程、剪贴板、原生退出和自更新必须另测真实宿主。

可稳定定位的界面属性：`[data-page]`、`[data-act]`、`[data-tool-id]`、`[data-operation-id]`。主要检查工作台固定与切换、搜索/分类/状态、版本禁用和切换确认、卸载路径与数据提示、真实任务的不定进度/失败/取消/重试、设置草稿与 revision 冲突、本体仅发布页和完整更新包两条路径、`window.changed {closeRequested:true}` 的忙退出确认。所有窗口尺寸和交互由 Codex 内置浏览器验证，禁止 Computer Use。

## 已执行的快速冒烟

2026-11-21：Codex 内置浏览器 iab，测试桥 normal 场景。

- 工作台视觉与本地图片、工具库中文搜索、版本详情、版本切换确认已通过。
- 未知总大小的任务显示不定进度；提交 `operation.start` 后能显示任务，取消后显示终态与重试。
- 代理地址本地校验通过；设置草稿在切换至关于页再返回后保留。
- 390、768、1440 宽度的工作台无横向溢出，390 宽设置页无横向溢出；最终 1440 截图的图片均加载成功。
- 浏览器控制台无 error/warn。`GET /__fixture/requests` 核对了 app.bootstrap、app.ready、带 revision/version/confirmed 的切换请求与取消请求。
- 截图：`target/frontend-validation/home-390.png`、`home-768.png`、`home-1440.png`。这些是快速冒烟证据；不替代独立 QA 与真实 Rust 宿主验收。
