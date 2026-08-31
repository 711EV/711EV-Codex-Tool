# ChatGPT中转工具

ChatGPT中转工具是一款纯本地运行的 ChatGPT Desktop 配置与 Codex 会话管理工具。它可以发现本机的 `CODEX_HOME`，在官方账号与不同中转供应商之间切换，并将其他供应商的本地主会话恢复到当前供应商。

项目基于 Tauri 2、Vue 3、TypeScript 和 Rust，支持 Windows 10、Windows 11 与 macOS。仓库和可执行文件继续使用 `711EV-Codex-Tool` 名称，以保持安装、升级和发布兼容。

## 主要功能

### 配置目录

- 自动识别运行中的 ChatGPT 所使用的 `CODEX_HOME`、环境变量、默认 `~/.codex`、已登记位置和常见托管工具目录。
- 校验 Codex 配置、会话索引和状态数据，排除符号链接、重解析点及明显无效的目录。
- 支持在多个已发现配置目录之间切换，并重新扫描当前目录。
- 只把可重启的 ChatGPT 主程序路径保存到配置目录记录中，不把 CLI 或 Chromium 辅助进程当作桌面客户端。

### 供应商管理

- 从主 `config.toml` 和本地会话中识别官方账号、当前供应商与历史供应商。
- 供应商卡片展示主会话、归档会话、子会话和会话总大小。
- 添加或编辑中转供应商的名称、API 地址和 API 密钥。
- 提供“使用711EV配置”预填，也可以在预填后继续修改。
- 在官方账号与已配置的中转供应商之间切换。
- 单独刷新某个供应商，或刷新当前配置目录下的全部供应商。
- API 地址和 API 密钥可点击复制；界面空间不足时会截断显示，悬停可查看完整内容。

官方供应商在界面中显示为“官方账号”，不可编辑，也不会展示 API 地址或 API 密钥。

### 会话恢复与清理

- “会话恢复”只出现在正在使用的供应商卡片中。
- 恢复弹窗按供应商分组展示其他供应商的未归档主会话，支持折叠分组和多选。
- 恢复会为所选会话创建新的 Thread，来源会话保持不变；目标始终是打开恢复弹窗的供应商。
- “会话清理”对所有供应商常驻，只列出该供应商的已归档会话和子会话。
- 清理支持多选并永久删除所选条目；清理子会话不会删除其所属主会话。
- 恢复和清理执行前都会明确提示关闭 ChatGPT。确认后，工具关闭对应进程树、完成操作，并在可识别启动目标时重新启动 ChatGPT。

本工具不恢复 ChatGPT 云端聊天，也不是会话备份软件。执行删除前请确认相关本地会话不再需要。

### 桌面体验

- 三段式竖屏界面，可视内容根据显示器工作区和 DPI 在 `320×605` 至 `420×794` 逻辑像素之间保持 `9:17` 比例，外窗另留透明阴影空间。
- 无边框圆角窗口、desk 暗色三层阴影、隐藏滚动条和内容区初始化状态。
- 关闭窗口时隐藏到系统托盘；托盘菜单支持显示主窗口、检查更新和退出。
- 应用内检查、下载和安装更新；“检查更新”悬停提示当前版本。
- 内置 QQ 交流群、GitHub、711EV 导航、推荐梯子和 711EV 中转站入口。
- Message 通知支持队列、自动消失、同类消息更新和悬停暂停计时。

## 快速使用

1. 启动应用，等待内容区初始化完成。
2. 在顶部“配置目录”中确认当前 `CODEX_HOME`；有多个目录时可展开切换。
3. 查看供应商卡片。已配置的供应商可点击“立即使用”，未配置的供应商先点击“添加配置”。
4. 切换供应商后，根据弹窗选择是否立即重启 ChatGPT，使新配置生效。
5. 需要找回其他供应商的会话时，在正在使用的供应商卡片中点击“会话恢复”，勾选主会话并点击“恢复选中”。
6. 需要释放空间时，在对应供应商卡片中点击“会话清理”，勾选归档会话或子会话并点击“删除选中”。

## 供应商配置

中转供应商需要填写：

- 供应商名称：对应 `model_provider` 与 `[model_providers.<名称>]` 的 ID，只能使用 ASCII 字母、数字、点、短横线和下划线。
- API 地址：必须是 HTTPS 地址。
- API 密钥：保存在本工具的本地 SQLite 数据库中。

“使用711EV配置”会预填供应商 `711EV` 和 API 地址 `https://ai.711ev.com/v1`。

保存配置不会自动切换供应商。点击“立即使用”后，工具会：

1. 从本地数据库读取该供应商的地址与密钥。
2. 在主 `config.toml` 中新增或更新对应的 `[model_providers.<名称>]`。
3. 将顶层 `model_provider` 设置为目标供应商。
4. 将中转密钥写入 `config.toml` 的 `experimental_bearer_token` 和 `auth.json` 的 `OPENAI_API_KEY`。
5. 原子提交并重新读取校验两个文件；失败时尝试回滚。

工具只管理所选 `CODEX_HOME` 的主 `config.toml` 和 `auth.json`，不会修改 `*.config.toml` Profile 或项目级覆盖配置。它会将主配置的认证存储方式设置为文件模式：

```toml
cli_auth_credentials_store = "file"
```

`disable_response_storage` 和 `service_tier` 仅在缺失时补齐，不覆盖已有值。

### 切换回官方账号

工具会监听各配置目录的官方 OAuth `auth.json`，并保存最近一份通过校验的本地快照。切换回官方账号时会：

1. 将主 `config.toml` 的 `model_provider` 设置为 `openai`。
2. 恢复最近有效的官方 OAuth 快照。
3. 保留已有的中转供应商配置段。
4. 如果没有可用快照，则移除中转使用的 `auth.json`，之后需要在 ChatGPT/Codex 中重新登录。

## 会话处理方式

会话恢复通过本机 Codex App Server 的 `thread/fork` 创建新的 Thread ID。来源 rollout、来源 Thread ID 和来源供应商不变，恢复后的会话归属于当前供应商。

恢复弹窗只展示满足以下条件的会话：

- 来自其他供应商；
- 未归档；
- 属于主会话而不是子会话；
- 本地 rollout 仍然存在并可以安全处理。

会话清理严格限定在打开弹窗的供应商：

- 已归档会话：删除本地归档 rollout。
- 子会话：删除未归档的内部/子 agent rollout，不影响主会话。

后端会在真正删除前重新生成预览，只处理仍然属于该供应商且符合范围的选中会话。

## ChatGPT 探测与重启

Windows 支持 Microsoft Store 的 `OpenAI.Codex*`、`OpenAI.ChatGPT*` 包以及常见独立安装路径；macOS 支持系统和用户 Applications 目录中的 ChatGPT `.app`。

进程识别只接受 ChatGPT 主进程，并排除 Codex CLI、`app-server`、renderer、crashpad 等辅助进程。自定义 `CODEX_HOME` 必须由目标进程显式提供；默认 `~/.codex` 才允许安全回退。

重启时工具会结束目标进程树，按原应用类型和目标 `CODEX_HOME` 启动，并等待确认主进程已经运行。若未找到可重启的 ChatGPT 安装路径，会提示用户手动启动。

## 本地数据与隐私

正式版在主程序或 macOS App 所在目录旁创建 `CodexLocalSync.data`。本地开发时，该目录位于项目根目录：

```text
CodexLocalSync.data/
├─ app.sqlite
├─ app.sqlite-wal
├─ app.sqlite-shm
├─ auth-snapshots/
├─ transactions/
└─ locks/
```

- `app.sqlite` 保存配置目录、供应商配置、用户录入的中转 API 密钥、会话关联和事务元数据，不保存聊天正文。
- API 密钥目前以本地数据形式保存，数据库本身不加密；供应商生效时，同一密钥还会写入对应 `CODEX_HOME`。请妥善保护这两个目录。
- `auth-snapshots` 包含官方 OAuth 登录凭据，只保存在本机，不进入 SQLite、日志、更新文件或 Git。
- `transactions` 保存供应商切换期间的临时候选文件和备份，正常完成后会删除；异常中断时用于下次启动恢复或回滚。
- 会话扫描、恢复与清理均在本机完成。本工具不会上传会话正文，但点击外部链接或检查更新会访问相应网站。

## 已知限制

- 当前只支持自动发现配置目录，界面不提供任意路径的手动添加入口。
- 只能处理本机仍然存在且可读取的 Codex rollout，无法恢复已经彻底删除的会话。
- 不处理 ChatGPT 云端聊天记录，也不会复制 Skills、Plugins、MCP 配置或其他扩展文件。
- Profile 配置或项目级覆盖可能覆盖主 `config.toml` 的最终行为，本工具不会修改这些覆盖项。
- 工具不会主动关闭 Codex CLI；已经运行的 CLI 不会自动重新加载新配置。
- 当前界面不提供会话迁移或双向同步入口；后端仅暂时保留相关 IPC 以兼容旧版本。

## 安装与运行

正式构建产物在 [GitHub Releases](https://github.com/711EV/711EV-Codex-Tool/releases) 发布：

- Windows 安装后桌面快捷方式为“ChatGPT中转工具”，主程序仍为 `711EV-Codex-Tool.exe`。
- Windows 同时提供便携版可执行文件。
- macOS 提供 Intel 与 Apple Silicon 通用 DMG。

首次启动需要对 `CodexLocalSync.data` 具有写权限。程序目录不可写时，应用可能请求提升权限以初始化数据目录。

## 本地开发

需要 Node.js 24、Rust stable，以及对应平台的 Tauri 2 构建环境。GitHub Actions 的正式构建与发布同样使用 Node.js 24。

```text
npm install --no-package-lock
npm test
node node_modules/vue-tsc/bin/vue-tsc.js --noEmit
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```

仅构建前端预览：

```text
npm exec vite -- build
```

完整桌面测试包：

```text
npm run build
```

`npm run build` 会在打包前自动把补丁版本增加一位，并同步版本文件；构建失败时恢复原版本。它还要求 Tauri 更新签名私钥，并在结束后清理 `src-tauri/target`。普通检查不要执行该命令。

本地打包输出：

- 前端静态资源：`build/`
- 本地预览程序：`dist/`
- 便携数据目录：`dist/CodexLocalSync.data/`

## 项目结构

```text
src/                       Vue 界面、状态管理和 IPC 封装
src-tauri/src/             Rust 业务逻辑与桌面生命周期
src-tauri/icons/           应用、托盘和安装器图标
scripts/build.mjs          本地版本递增与桌面测试包脚本
.github/workflows/         Windows/macOS 正式发布流程
```

## 相关链接

- [使用教程](https://docs.711ev.com/#/711ev-relay/guide/codex-tool)
- [QQ 交流群](https://qm.qq.com/q/e9xHZxgN4Q)
- [711EV 导航](https://www.711ev.com/)
- [711EV 中转站](https://ai.711ev.com/)
- [推荐梯子](https://www.tntv2.net/auth/register?code=oow59s)
- [GitHub 项目](https://github.com/711EV/711EV-Codex-Tool)

## 许可证

项目代码采用 [MIT License](LICENSE)，版权归 `711EV` 所有。项目名称、`711EV` 品牌和 Logo 不因 MIT License 自动获得商标或品牌授权。
