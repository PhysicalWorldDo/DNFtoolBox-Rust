# 物理世界的工具箱

Windows x64 桌面工具箱。Rust 管理工具目录、下载、安装和启动，Tauri 2 通过系统 WebView2 显示随 EXE 嵌入的本地界面。

## 使用本地构建

打开 `dist/PhysicalWorldToolbox.exe`。便携版以 EXE 所在目录为工作区，请放在当前用户有写入权限的目录。Windows 需要已安装 [Microsoft Edge WebView2 Evergreen Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。工具箱本体无需 Python，各工具使用自己安装包声明的入口。

首次使用空目录时，可在「安装与更新」获取工具列表。使用旧工作区时，先退出旧版并备份原 EXE，再将新版 EXE 放到原来的 `config/` 和 `tools/` 旁。安装状态来自该目录的 `config/installed.json`。复制单个 EXE 到另一个文件夹会使用另一个工作区。

在工具库点击工具右侧的图钉即可「固定到首页」。高亮图钉代表已固定，再点一次即可取消。首页轮播、卡片与版本动态都跟随固定名单；首次使用或取消最后一个固定项后，可从首页进入工具库挑选。

## 源码

| 目录 | 职责 |
| --- | --- |
| `crates/toolbox-core/` | Registry 和旧配置兼容、版本选择、工具操作和任务状态 |
| `src-tauri/` | Windows 原生宿主、前后端通信和本体更新 |
| `frontend/` | 随 EXE 分发的 HTML、CSS、JavaScript 与图片 |
| `tests/` | 前端契约与集成验收 |

## 构建

在 Windows x64 上安装 Rust MSVC 工具链、Visual Studio C++ 构建工具和 Windows SDK，然后在本仓库根目录用 PowerShell 7 执行：

```powershell
pwsh -File scripts/package.ps1
```

脚本会定位 Windows 资源编译器，构建并检查 EXE 的 DLL 依赖，将可执行文件与单文件 ZIP 写入 `dist/`。只需编译时执行 `pwsh -File scripts/build.ps1`，产物位于 `target/x86_64-pc-windows-msvc/release/PhysicalWorldToolbox.exe`。

发行物的校验值、测试结果与复跑命令见 [0.4.0 本地发行验收](tests/ACCEPTANCE.md)。

本项目沿用 [GPL-3.0 许可证](LICENSE)。
