# Codex Local Sync

在同一个本地 `CODEX_HOME` 中按历史 Provider 查看 Codex 会话，并把来源会话复制为当前 Provider 下可继续使用的新 Thread。项目使用 Tauri 2、Vue 3、TypeScript 和 Rust，聊天内容不会上传到第三方服务。

## 运行

Vue 静态资源统一生成到项目根目录：

```text
build/
```

桌面发布产物固定保存在 `dist` 根目录，每次构建只保留当前版本：

```text
dist/
├─ 711EV-Codex-Tool.exe                  # Windows 便携客户端
├─ 711EV-Codex-Tool-Setup.exe            # Windows 安装/升级程序
├─ 711EV-Codex-Tool-Setup.exe.sig        # 安装/升级程序签名
├─ latest.json                            # 在线更新清单
├─ 711EV-Codex-Tool.app                  # macOS（在 macOS 构建）
└─ CodexLocalSync.data/                  # 首次运行后生成
```

首次运行时，程序会在可执行文件同级创建：

```text
CodexLocalSync.data/
```

该目录包含工具数据库、托管 Profile 和操作锁。工具数据库只保存来源 Thread 与副本 Thread 的映射，不保存聊天正文。若同级目录需要管理员权限，程序只在初始化阶段请求一次授权并把目录权限授予当前用户。

Windows 和 macOS 分别在对应系统执行打包命令。Windows 同时生成便携 `.exe` 和 NSIS 安装程序；安装时可以选择目录，完成页默认勾选“创建桌面快捷方式”和“运行应用”。主程序文件名为 `711EV-Codex-Tool.exe`，快捷方式名称为 `711EVCodex工具`。macOS 生成 Finder 中的单个 `.app`。运行数据都放在客户端同级的 `CodexLocalSync.data` 中，覆盖升级不会删除该目录。

## 构建

```bash
npm install
npm run package
```

`npm run package` 是日常打包的唯一入口。该命令会关闭正在运行的当前客户端，执行前端测试、Rust 格式检查和 Rust 测试，调用底层构建，自动增加补丁版本号，校验 `dist` 目录，重新启动客户端并确认进程响应正常。打包时会清除旧客户端，但始终保留同级的 `CodexLocalSync.data` 运行数据。

Windows 升级包使用 Tauri 签名。默认私钥路径为：

```text
C:\Users\当前用户名\.tauri\711ev-codex-tool.key
```

私钥只存在于发布电脑，不会进入 Git、安装程序或用户目录。必须单独安全备份；丢失私钥后，已安装的旧客户端无法验证后续升级。需要改用其他磁盘时，通过 `TAURI_SIGNING_PRIVATE_KEY_PATH` 指定绝对路径。

只需要生成产物、不需要执行完整测试和启动验证时，可单独调用底层构建器：

```bash
npm run build
```

该命令会生成根目录 `build/` 中的 Vue 文件和 `dist/` 中的最终客户端，并自动增加补丁版本号。

## 发布到 Gitea 软件包

远程仓库为 `https://git.711ev.com/711ev/711EV-Codex-Tool`，二进制版本发布到 Gitea Generic Packages。先在 Gitea 的“设置 -> 应用 -> 访问令牌”创建具有软件包读写权限的令牌，然后在 PowerShell 中执行：

```powershell
npm run package

git add .
git commit -m "release: 0.1.66"
git tag v0.1.66
git push origin master
git push origin v0.1.66

$secureToken = Read-Host "Gitea Token" -AsSecureString
$env:GITEA_TOKEN = [Net.NetworkCredential]::new("", $secureToken).Password
npm run release:gitea -- --notes "本次版本更新说明"
Remove-Item Env:GITEA_TOKEN
```

版本号以实际 `package.json` 为准。`git push` 只发布源码和版本标签；`npm run release:gitea` 才会通过 Gitea API 上传便携客户端、安装程序、升级包、签名和 `latest.json`。同一版本需要重新上传时，确认旧软件包可以覆盖后执行：

```powershell
npm run release:gitea -- --replace --notes "修正后的版本说明"
```

客户端的“检查更新”读取公开地址 `https://git.711ev.com/api/packages/711ev/generic/711ev-codex-tool/latest/latest.json`，用户端不需要 Gitea 令牌。

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
