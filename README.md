# Codex Local Sync

在多个本地 `CODEX_HOME` 之间预览、备份并同步 Codex 项目会话。项目使用 Tauri 2、Vue 3、TypeScript 和 Rust，聊天内容不会上传到第三方服务。

## 运行

Vue 静态资源统一生成到项目根目录：

```text
build/
```

桌面发布产物按“版本号 / 系统”保存：

```text
dist/
└─ 0.1.0/
   ├─ windows/
   │  └─ CodexLocalSync.exe
   └─ macos/
      └─ Codex Local Sync.app
```

首次运行时，程序会在可执行文件同级创建：

```text
CodexLocalSync.data/
```

该目录包含工具数据库、托管 Profile、备份、日志和锁。若同级目录需要管理员权限，程序只在初始化阶段请求一次授权并把目录权限授予当前用户。

Windows 和 macOS 分别在对应系统执行 `npm run build:desktop`。Windows 生成单个 `.exe`；macOS 生成 Finder 中的单个 `.app`。运行数据都放在客户端同级的 `CodexLocalSync.data` 中。

## 开发

```bash
npm install
npm run tauri dev
```

正式打包：

```bash
npm run build:desktop
```

验证命令：

```bash
npm run build
npm test
cd src-tauri
cargo fmt --check
cargo test
cargo check
```

## 同步边界

- 只处理 `sessions`、`archived_sessions` 和相关本地索引。
- 不同步 `auth.json`、Token、API Key、Skills、Plugins 或 ChatGPT 云端聊天。
- 写入前会尝试正常关闭目标客户端；超时后必须在 UI 中确认才能强制结束。
- 会话冲突默认保留目标版本，只有显式选择后才覆盖。
- 每次有效写入前都会在 `CodexLocalSync.data/backups` 创建备份。

详细设计见 [docs/codex-local-session-sync-design.md](docs/codex-local-session-sync-design.md)。
