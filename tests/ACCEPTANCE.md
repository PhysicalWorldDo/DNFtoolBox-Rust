# Windows x64 本地发行验收

## 0.4.0 内嵌标题栏构建

2026-11-21，源码提交 `3b59e79`。以下文件由 `scripts/package.ps1` 生成。

| 文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `dist/PhysicalWorldToolbox.exe` | 14,705,664 | `0b0f1172c941770f336a57e210073984a33555387b73026b80cb641d22353c84` |
| `dist/PhysicalWorldToolbox-0.4.0-win-x64.zip` | 5,647,525 | `bf2f89a7eaed242f4f052f312ba946ad4236f7d11c89873577d539d60e44d6d7` |

ZIP 内唯一的条目是根目录的 `PhysicalWorldToolbox.exe`，其解压内容的 SHA-256 与独立 EXE 一致。45 项 Rust 测试、21 项前端测试及 PE 导入检查通过。

正式 EXE 的隔离副本在 120 DPI 下用 451 毫秒完成 WebView2 首屏就绪握手。Win32 读数显示客户区与窗口顶边的距离为 0，四边、四角的缩放命中均正确。宿主响应最大化、还原、最小化和 Alt+F4，正常退出码为 0。最大化客户区与本机显示器工作区相同。原生宿主记录位于 `.work/embedded-window/native-a3642681d6314da7ac60c0b7fb2e917c/evidence.json`。

独立验收在 Codex 内置浏览器中加载生产前端，并以测试桥提供工具数据和窗口事件。1420 × 900、640 × 440、390 × 844 三种视口下，主视觉均延伸至顶边，搜索按钮和三个窗口按钮各自独立命中。640 × 440 首页下滚后标题栏使用实色背景，回顶恢复透明渐变。单击搜索或按 Ctrl+K 可聚焦搜索框；切换窗口焦点及最大化状态时，已输入的文本与光标位置保留。可取消任务的关闭请求先显示确认，提交或恢复文件时只显示等待与查看任务。[桌面界面截图](titlebar-home.jpg)

拖动与双击根据已锁定 Tauri 版本的命中规则核对标题区、搜索、窗口按钮和首页主操作。独立测试另起最终 EXE，读取 Win32 与 DWM 窗口边界，并记录了 486 毫秒的就绪握手。原生生命周期由上一段的宿主测试执行。独立复核记录位于 `.work/titlebar-qa/report.json`。

## 0.4.0 初版构建记录

2026-11-21，版本 0.4.0。验收对象为源码提交 `e6d56a6` 经 `scripts/package.ps1` 生成的发行物。

| 文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `dist/PhysicalWorldToolbox.exe` | 14,742,016 | `0c842505dc2fac617c81ed87daac7b7e125b7258d867f2dd67b118e1312ef072` |
| `dist/PhysicalWorldToolbox-0.4.0-win-x64.zip` | 5,666,334 | `28b28e37af2f4766b2f5e77cf764852e60fe00e552f0f7dbc424612cd5a4fe01` |

ZIP 根目录只有一个 EXE。解压数据流的 SHA-256 与单独交付的 EXE 一致。

| 验收场景 | 结果 |
| --- | --- |
| 构建与自动化 | Windows MSVC Release 构建通过。42 项 Rust 测试和 14 项前端测试通过。PE 导入检查通过。 |
| 真实窗口 | 仅复制最终 EXE 到空目录并启动。系统 WebView2 加载内嵌前端，在本机用约 0.6 秒返回首屏就绪回执，进程和会话标识匹配。 |
| 首次获取工具 | 最终 EXE 使用默认公开 Registry 获取 23 款工具。没有下载实际工具包。重新启动后从本地缓存读回 23 款。 |
| 旧工作区迁移 | 执行旧 0.3.0 发布 EXE 内嵌更新器生成的原版脚本，用最终 ZIP 在隔离副本中替换并启动 Rust 版。复核 53 个配置与工具文件的内容，旧目录中 22 款工具与 14 条安装记录可读。 |
| 工具安装与启动 | 在受控 HTTP Registry 和 ZIP 上验证下载、哈希校验、安装、真实 CMD 启动标记、升级、保留用户配置和数据、卸载。失败包未写入安装记录。 |
| 更新器与恢复 | 实际 helper 进程分别接收健康候选、无效镜像及版本不匹配候选。健康候选完成就绪握手，其余两种恢复原 EXE 的哈希并重新启动。升级进行时，写入请求返回忙状态。 |
| 事务与界面 | 故障注入覆盖部分复制、中断、外部修改保护、重复恢复及批量失败后重试。Codex 内置浏览器验证生产界面和事件契约。浏览器夹具模拟宿主，文件与进程行为由上述真实 EXE 测试覆盖。 |

系统 WebView2 已安装。缺失 Runtime 的提示分支未在干净虚拟机复验。断电场景使用故障注入和中断状态回放。旧版迁移从原更新器入口开始，旧版界面的点击流程未纳入测试。

## 复跑自动化测试

在 Windows x64 的 Visual Studio Developer PowerShell 中，从仓库根目录运行：

```powershell
cargo test --workspace --locked --target x86_64-pc-windows-msvc --features custom-protocol
node --test tests/*.test.mjs
```

本次本机的独立黑盒测试记录位于 `.work/qa/`，旧更新器迁移证据位于 `.work/legacy-upgrade/`。界面夹具的使用说明见 [前端验收](FRONTEND-VALIDATION.md)。
