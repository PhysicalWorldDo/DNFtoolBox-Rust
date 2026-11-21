# 物理世界的工具箱

Windows x64 桌面工具箱。Rust 管理工具目录、下载、安装和启动，Tauri 2 通过系统 WebView2 显示随 EXE 嵌入的本地界面。

## 下载与更新

下载 `PhysicalWorldToolbox-<版本>-win-x64.zip`，将其中的 `PhysicalWorldToolbox.exe` 解压到有写入权限的目录后打开。[GitHub Releases](https://github.com/PhysicalWorldDo/DNFtoolBox-Rust/releases/latest)

便携版以 EXE 所在的目录为工作区，首次使用可在「安装与更新」获取工具列表。本体无需 Python，各工具使用自己安装包声明的入口。运行界面需要 [Microsoft Edge WebView2 Evergreen Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。

在「安装与更新」或「设置」点击「检查本体更新」。程序查询 GitHub 的最新正式版，并校验 ZIP 的大小和 SHA-256。下载完成后，点击「应用更新并重启」并确认。更新器备份并替换工作区根目录的 `PhysicalWorldToolbox.exe`，保留 `config/`、`tools/` 和个人文件。新版无法正常启动时会恢复旧 EXE。

在工具库点击工具右侧的图钉即可「固定到首页」。高亮图钉代表已固定，再点一次即可取消。首页轮播、卡片与版本动态都跟随固定名单；首次使用或取消最后一个固定项后，可从首页进入工具库挑选。

在首页单击固定卡片会切换当前工具并保留滚动位置。双击卡片可直接启动已安装且可运行的工具。工具库仍通过「启动工具」按钮启动。

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

发行物的校验值、测试结果与复跑命令见[本地发行验收](tests/ACCEPTANCE.md)。

## 手动发布

打开 **Actions → Windows build and release → Run workflow**。将 `release_tag` 留空会构建选定分支并上传可下载的 Actions 构建产物。推送代码或标签不会触发构建。[GitHub Actions](https://github.com/PhysicalWorldDo/DNFtoolBox-Rust/actions/workflows/windows.yml)

发布前同步 `src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 和 `Cargo.lock` 中的应用版本，提交并推送源码，再为该提交推送同版本的 `vX.Y.Z` 标签。运行工作流时填入该标签。Actions 会检出标签源码，运行 Rust 与前端测试，构建 Windows x64 便携 ZIP，并将该产物发布到对应的 Release。

本项目沿用 [GPL-3.0 许可证](LICENSE)。
