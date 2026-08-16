# Codex Local Sync

在同一个本地 `CODEX_HOME` 中按历史 Provider 查看 Codex 会话，并把来源会话复制为当前 Provider 下可继续使用的新 Thread。项目使用 Tauri 2、Vue 3、TypeScript 和 Rust，聊天内容不会上传到第三方服务。

## 运行

Vue 静态资源统一生成到项目根目录：

```text
build/
```

`dist` 是本地预览目录，只保存便携客户端；运行预览程序后会在同级生成运行数据目录：

```text
dist/
├─ 711EV-Codex-Tool.exe                  # Windows 便携客户端
├─ 711EV-Codex-Tool.app                  # macOS（在 macOS 构建）
└─ CodexLocalSync.data/                  # 首次运行后生成
```

本地待发布产物统一保存在 `releases`：

```text
releases/
├─ 711EV-Codex-Tool.exe                  # Windows 便携版附件
├─ 711EV-Codex-Tool-Setup.exe            # Windows 安装/升级程序
├─ 711EV-Codex-Tool-Setup.exe.sig        # 自动更新签名
└─ latest.json                            # 在线更新清单
```

`releases` 整目录被 Git 忽略，仅用于本地构建输出。GitHub 正式版本由 `.github/workflows/release.yml` 自动构建和发布。

首次运行时，程序会在可执行文件同级创建：

```text
CodexLocalSync.data/
```

该目录包含工具数据库、托管 Profile 和操作锁。工具数据库只保存来源 Thread 与副本 Thread 的映射，不保存聊天正文。若同级目录需要管理员权限，程序只在初始化阶段请求一次授权并把目录权限授予当前用户。

Windows 和 macOS 分别在对应系统执行打包命令。Windows 同时生成便携 `.exe` 和 NSIS 安装程序；安装时可以选择目录，完成页默认勾选“创建桌面快捷方式”和“运行应用”。主程序文件名为 `711EV-Codex-Tool.exe`，快捷方式名称为 `711EVCodex工具`。macOS 生成 Finder 中的单个 `.app`。运行数据都放在客户端同级的 `CodexLocalSync.data` 中，覆盖升级不会删除该目录。

## 构建

```bash
npm install
npm run build
```

`npm run build` 是统一打包入口。该命令执行 TypeScript 检查、前端构建和 Tauri 桌面打包，自动增加补丁版本号，并分别生成 `dist` 预览程序和 `releases` 发布产物。打包时会清除旧产物，但始终保留 `dist/CodexLocalSync.data` 运行数据。

Windows 升级包使用 Tauri 签名。默认私钥路径为：

```text
C:\Users\当前用户名\.tauri\711ev-codex-tool.key
```

私钥只存在于发布电脑，不会进入 Git、安装程序或用户目录。必须单独安全备份；丢失私钥后，已安装的旧客户端无法验证后续升级。需要改用其他磁盘时，通过 `TAURI_SIGNING_PRIVATE_KEY_PATH` 指定绝对路径。

## 发布到 GitHub Releases

远程仓库为 `https://github.com/711EV/711EV-Codex-Tool`。仓库需要配置 Actions Secret `TAURI_SIGNING_PRIVATE_KEY`，内容为 Tauri 更新私钥。推送版本标签后，GitHub Actions 会自动构建 Windows 和 macOS Universal 版本，生成跨平台 `latest.json` 并创建 GitHub Release：

```powershell
npm run build

$version = (Get-Content package.json | ConvertFrom-Json).version
git add .
git commit -m "release: $version"
git tag "v$version"
git push origin master
git push origin "v$version"
```

Actions 发布内容包括 Windows 便携版、Windows 安装版、macOS Intel/Apple Silicon Universal DMG、两端更新包签名和 `latest.json`。客户端的“检查更新”读取 GitHub 最新 Release 中的公开 `latest.json`，用户端不需要 GitHub 令牌。`CodexLocalSync.data` 是本机运行数据，不会上传。

测试命令：

```bash
npm test
cd src-tauri
cargo fmt --check
cargo test
cargo check
```

## 同步边界

- 启动时自动发现当前 `CODEX_HOME`、`~/.codex`、运行中的 Codex/ChatGPT 客户端、工具自身托管目录和受支持切换工具的本地实例；也可在侧栏手动重新发现。
- 发现过程只扫描有限的标准位置、实例清单和已验证实例的同级目录，不全盘搜索磁盘。自定义目录可通过 `CODEX_SYNC_DISCOVERY_ROOTS`（系统路径列表）加入发现范围。
- 每个物理 `CODEX_HOME` 只对应一个本地目录；Provider 分组来自当前配置和历史 rollout 的真实 `model_provider`。
- 当前配置生效的 Provider 固定为复制目标，不能选择另一个已有会话作为覆盖目标。
- 每条成功结果由官方 App Server `thread/fork` 创建新的 Thread ID，来源 rollout、来源 Thread ID 和来源 Provider 保持不变。
- 只允许复制 `sessions` 中未归档的 `cli` / `vscode` 主会话；subagent 等内部线程不参与复制，可按当前供应商单独预览并清理。
- 不同步 `auth.json`、Token、API Key、Skills、Plugins 或 ChatGPT 云端聊天。
- 发现器不会读取 `auth.json` 内容，也不会仅凭一个同名文件判断实例。已删除、已移动且没有任何本地路径引用的历史实例无法自动恢复。
- 写入前会尝试正常关闭匹配当前 Home 的客户端；超时后必须在 UI 中确认才能强制结束。
- 新流程不创建备份，不直接写官方 Codex SQLite；验证失败时只删除本次创建且尚未交付的副本。
- 已验证映射保持幂等。来源后续更新时不会覆盖可能已经继续聊天的旧副本。
