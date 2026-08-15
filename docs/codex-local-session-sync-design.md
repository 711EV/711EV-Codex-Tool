# Codex 本地会话同步工具设计方案

## 1. 文档信息

- 状态：Draft
- 目标平台：macOS、Windows
- 技术栈：Tauri 2、Vue 3、TypeScript、Rust
- 产品范围：同步 ChatGPT/Codex 桌面客户端的本地 Codex 项目会话
- 非目标：ChatGPT 网页端普通聊天、任何云端聊天同步

## 2. 背景与目标

ChatGPT/Codex 桌面客户端在使用不同登录账号或自定义 API Provider 时，可以使用不同的 `CODEX_HOME` 保存本地配置和会话。切换账号、Provider 或实例后，会话可能因为存储目录隔离、`model_provider` 不匹配或本地索引未重建而不出现在左侧会话列表中。

本工具用于管理这些本地 Codex 实例，并在实例之间复制或同步完整会话，使目标实例能够在官方客户端左侧列表中显示并继续使用这些会话。

核心目标：

1. 管理多个登录账号和自定义 API 对应的本地 Codex 实例。
2. 查看所有实例中的本地项目会话。
3. 将会话安全地同步到一个或多个目标实例。
4. 修复 Provider 切换后本地会话不可见的问题。
5. 支持同步预览、冲突检测、备份和恢复。
6. 所有会话同步仅在本机完成，不上传聊天内容。
7. 工具采用便携式单文件分发，运行数据统一保存在启动文件同级的专用目录中。

## 3. 范围

### 3.1 包含范围

- `sessions/**/*.jsonl`
- `archived_sessions/**/*.jsonl`
- `session_index.jsonl`
- `.codex-global-state.json` 中与项目可见性有关的路径索引
- 官方 Codex SQLite 会话索引的重建或有限修复
- 会话标题、时间、项目路径、Provider 和归档状态
- 登录账号实例与自定义 API 实例之间的双向同步
- macOS 和 Windows 的客户端定位、进程检测和路径处理

### 3.2 不包含范围

- ChatGPT 网页端普通聊天记录
- ChatGPT 云端会话或云端项目同步
- 上传聊天内容到第三方服务器
- 自动复制 `auth.json`、登录 Token 或 API Key
- 自动同步 `config.toml`
- 自动同步 Skills、Plugins、Rules 或 `AGENTS.md`
- 首版中对已经分叉的同 ID 会话进行无提示自动合并

## 4. 关键假设

1. 每个账号或自定义 Provider 使用独立的 `CODEX_HOME`。
2. 目标机器已经安装官方 ChatGPT/Codex 桌面客户端，或安装了兼容的 Codex CLI。
3. 本地会话的完整记录以 rollout JSONL 为主要事实来源。
4. `session_index.jsonl` 和 SQLite 是会话发现与列表展示所需的索引，不是聊天内容的唯一事实来源。
5. 官方客户端内部格式可能随版本变化，因此所有写入都必须经过格式探测、备份和验证。

## 5. 总体架构

```text
Vue 3 + TypeScript
├─ 实例管理
├─ 会话浏览
├─ 同步预览
├─ 冲突处理
├─ 备份恢复
└─ 设置与诊断
        │
        │ Tauri Commands / Events
        ▼
Rust Core
├─ Profile Manager
├─ Session Scanner
├─ Rollout Parser
├─ Sync Planner
├─ Sync Executor
├─ Visibility Repair
├─ Official App Server Client
├─ Backup Manager
├─ Process Monitor
└─ Local Metadata Store
        │
        ├─ 多个 CODEX_HOME
        ├─ 工具自己的 SQLite
        └─ 官方 codex app-server
```

### 5.1 前端

- Vue 3 Composition API
- TypeScript
- Vite
- Pinia
- Vue Router
- Tauri JavaScript API

前端只负责展示和用户操作，不直接读写 Codex 数据文件。

### 5.2 Rust 后端

建议依赖：

```text
serde / serde_json
toml
rusqlite
sha2
uuid
chrono
zip
fs2
notify
tracing / tracing-subscriber
thiserror
```

Rust 后端负责所有文件、数据库、进程和 App Server 操作。

### 5.3 单文件与便携模式

本项目不依赖 Node.js、Python 或外部运行时。Vue 静态资源编译后嵌入 Tauri 资源，Rust 同步核心编译进主程序。发布时只分发一个平台启动文件：

```text
Windows: CodexLocalSync.exe
macOS:   CodexLocalSync.app
```

Windows 可以是一个真正的单独 `.exe` 文件。macOS 的 GUI 应用通常以 `.app` bundle 分发；它在 Finder 中表现为一个应用文件，但技术上是一个目录。数据目录应放在 `.app` 同级，而不是写入 `.app/Contents`：

```text
Windows/
├─ CodexLocalSync.exe
└─ CodexLocalSync.data/

macOS/
├─ CodexLocalSync.app
└─ CodexLocalSync.data/
```

工具数据根目录由启动文件位置计算，不使用 `AppData`、`~/Library/Application Support` 或其他系统默认数据目录。macOS 需要从应用包内的可执行文件路径向上解析到 `.app` 根目录，再取其父目录。

如果同级目录没有写权限，工具优先发起一次系统管理员授权，用提权辅助进程创建 `CodexLocalSync.data`，再把目录所有权和读写权限授予当前登录用户。之后主程序以普通用户权限运行，不应为每次同步重复提权。挂载的只读 DMG 或真正的只读文件系统无法通过提权变为可写，此时必须提示用户将应用移动到可写卷。工具不能静默回退到系统数据目录。开发模式可以通过 `CODEX_SYNC_DATA_DIR` 覆盖数据根目录。

```text
CodexLocalSync.data/
├─ app.sqlite
├─ profiles/
├─ backups/
├─ exports/
├─ logs/
├─ locks/
└─ migrations/
```

所有工具状态、同步历史、备份、导入导出包和日志都写入该目录。日志禁止写入聊天正文、Token 或 API Key。

## 6. 实例模型

### 6.0 自动发现范围

工具面向 Windows 和 macOS 的普遍安装场景执行有界发现，不依赖开发机路径。候选来源包括：

- 当前进程和系统提供的 `CODEX_HOME`；
- 官方默认目录 `~/.codex`；
- 运行中的 Codex、ChatGPT 或实例切换工具进程暴露的环境变量/启动参数；
- `CodexLocalSync.data/profiles` 中的托管实例；
- 常见切换工具的标准数据目录及其 `codex_instances.json` 路径引用；
- 已通过指纹校验的实例的直接同级目录；
- 用户通过 `CODEX_SYNC_DISCOVERY_ROOTS` 显式提供的额外根目录。

候选目录必须存在 `config.toml`、`sessions/`、`archived_sessions/`、`session_index.jsonl`、`state_*.sqlite`、`sqlite/state_*.sqlite` 或 `.codex-global-state.json` 等 Codex 指纹才会登记。扫描限制目录项与配置文件大小，不执行整盘递归搜索。发现器不会读取 `auth.json`，也不会仅凭该文件登记实例。

一个物理 `CODEX_HOME` 是一个同步实例。同一目录中 `[model_providers.<id>]`、`[profiles.<name>]` 和 `<name>.config.toml` 声明的 Provider/Profile 作为该实例元数据展示和刷新，不拆分为多个虚假的同步目标。已注销账号只有在独立本地目录或实例清单引用仍然存在时才能被发现。

每个拥有独立本地会话目录的账号或自定义 API 环境都对应一个实例：

```rust
struct Profile {
    id: Uuid,
    name: String,
    kind: ProfileKind,
    codex_home: PathBuf,
    provider_id: String,
    app_path: Option<PathBuf>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

enum ProfileKind {
    ChatGptAccount,
    CustomApi,
}
```

目录示例（便携模式下由工具管理的 Profile）：

```text
默认登录账号
<application>/CodexLocalSync.data/profiles/chatgpt-default/

自定义 API A
<application>/CodexLocalSync.data/profiles/api-a/

自定义 API B
<application>/CodexLocalSync.data/profiles/api-b/
```

### 6.1 外部 Profile 与托管 Profile

当前已经登录的官方客户端通常已经有一个外部 `CODEX_HOME`，例如 `~/.codex`。工具不能擅自移动或删除它，因此 Profile 分为两种：

- `external`：工具只保存外部目录引用，源数据仍由官方客户端管理；默认只读扫描，写入前要求明确确认。
- `managed`：完整目录位于 `CodexLocalSync.data/profiles/<id>`，由工具负责备份、同步和生命周期。

“所有工具记录都在同级目录”意味着工具数据库、配置、同步记录和备份都位于 `.data`。若还要求所有 Codex 会话副本也位于同级目录，则必须使用 `managed` Profile。首次导入外部账号时可以复制为托管 Profile，但不自动复制认证凭据；用户需要在该 Profile 中重新登录或配置 Provider。

当目标是官方桌面客户端当前正在使用的外部 Profile 时，工具可以在备份后写入其会话文件。这属于对外部 Codex 数据的受控修改，UI 必须展示准确路径并确认目标客户端已经关闭。

启动对应客户端或 App Server 时设置：

```text
CODEX_HOME=<profile.codex_home>
```

如果多个 Provider 共用同一个 `CODEX_HOME`，同一 thread ID 只能保存一套 `model_provider` 可见性元数据。这种配置只支持切换可见性，不支持真正隔离的双向同步，因此产品应默认引导用户创建独立实例。

## 7. Codex 本地数据模型

### 7.1 Rollout

会话内容位于：

```text
<CODEX_HOME>/sessions/**/rollout-*.jsonl
<CODEX_HOME>/archived_sessions/**/rollout-*.jsonl
```

文件第一条有效记录通常为 `session_meta`，其中至少需要读取：

```json
{
  "type": "session_meta",
  "payload": {
    "id": "thread-id",
    "cwd": "/path/to/project",
    "model_provider": "openai"
  }
}
```

同步到目标实例时，需要将 `model_provider` 改为目标实例当前使用的 Provider。

### 7.2 会话索引

```text
<CODEX_HOME>/session_index.jsonl
```

索引用于快速获取会话标题、更新时间和部分展示元数据。同步 rollout 后应插入或更新相应条目。

### 7.3 项目索引

```text
<CODEX_HOME>/.codex-global-state.json
```

同步时需要将会话 `cwd` 加入客户端使用的项目顺序和已保存工作区集合。写入键必须经过当前客户端版本探测；不能识别格式时只同步 rollout，并交由 App Server 扫描。

### 7.4 SQLite

常见位置可能包括：

```text
<CODEX_HOME>/state_*.sqlite
<CODEX_HOME>/sqlite/state_*.sqlite
```

原则上不复制整个 SQLite 数据库。优先通过官方 App Server 的 `thread/list` 触发扫描和索引重建。只有经过 schema 探测且存在明确兼容逻辑时，才允许在备份后有限更新 `threads.model_provider` 等已识别字段。

## 8. 同步流程

### 8.1 扫描

1. 枚举已配置 Profile。
2. 递归扫描 `sessions` 和 `archived_sessions`。
3. 读取每个 rollout 第一条有效 `session_meta`。
4. 读取 `session_index.jsonl` 并关联标题和更新时间。
5. 计算文件 SHA-256、大小、修改时间和事件摘要。
6. 生成统一 `ThreadSnapshot`。

```rust
struct ThreadSnapshot {
    thread_id: String,
    profile_id: Uuid,
    rollout_path: PathBuf,
    archived: bool,
    title: Option<String>,
    cwd: Option<PathBuf>,
    provider_id: Option<String>,
    updated_at: Option<DateTime<Utc>>,
    size_bytes: u64,
    sha256: String,
    event_fingerprint: String,
}
```

### 8.2 计划

同步前生成只读计划：

```text
新增
无需变更
目标是源的旧版本
源是目标的旧版本
已分叉
无效或不兼容
```

计划中必须展示来源、目标、会话标题、项目路径、动作、风险和预计备份空间。

### 8.3 执行

```text
检查源和目标路径
  ↓
获取实例级文件锁
  ↓
确认目标客户端状态
  ↓
创建备份
  ↓
复制 rollout 到临时文件
  ↓
校验 JSONL 和 thread ID
  ↓
改写目标 model_provider
  ↓
原子替换目标 rollout
  ↓
更新 session_index.jsonl
  ↓
更新项目路径索引
  ↓
调用官方 App Server 重建索引
  ↓
重新扫描并验证
  ↓
记录同步结果和基线
```

首版默认只允许目标客户端停止时写入。运行中只允许扫描和预览。

## 9. 冲突策略

### 9.1 判断规则

| 状态 | 判断 | 默认操作 |
|---|---|---|
| 目标不存在 | 目标无相同 thread ID | 复制 |
| 完全相同 | SHA-256 相同 | 跳过 |
| 源领先 | 目标事件序列是源的完整前缀 | 更新目标 |
| 目标领先 | 源事件序列是目标的完整前缀 | 跳过并提示 |
| 已分叉 | 双方都有独有事件 | 阻止自动覆盖 |
| 格式无效 | 缺少有效 `session_meta` 等 | 跳过并报告 |

### 9.2 分叉处理

首版支持：

- 保留目标
- 用源覆盖目标，覆盖前备份
- 两份都保留为独立记录，仅当能够安全生成新 thread ID 并重写所有相关引用时启用

禁止默认采用“将 JSONL 行按时间排序后合并”的方式解决分叉。事件时间相邻不代表对话语义连续，工具调用和响应也可能形成不可拆分的关联。

## 10. Provider 可见性修复

会话存在但不显示时，按以下顺序处理：

1. 读取目标 `config.toml` 中的当前 `model_provider`。
2. 修改目标 rollout 中 `session_meta.payload.model_provider`。
3. 更新 `session_index.jsonl`。
4. 启动目标 `CODEX_HOME` 对应的官方 App Server。
5. 调用 `initialize`。
6. 调用 `thread/list`，不限制 `modelProviders` 和 `sourceKinds`。
7. 验证会话是否进入官方线程索引。
8. 必要时在 schema 检查和备份后修复 SQLite Provider 字段。

## 11. 官方 App Server 集成

### 11.1 可执行文件定位

macOS 候选路径：

```text
/Applications/ChatGPT.app/Contents/Resources/codex
/Applications/Codex.app/Contents/Resources/codex
```

Windows 候选位置：

```text
<ChatGPT.exe 所在目录>/resources/codex.exe
<Codex.exe 所在目录>/resources/codex.exe
```

还应允许用户手动配置路径，并支持环境变量覆盖用于开发和测试。

### 11.2 启动方式

```text
CODEX_HOME=<目标目录>
codex app-server --listen stdio://
```

最小 JSON-RPC 流程：

```json
{"method":"initialize","id":1,"params":{"clientInfo":{"name":"codex-sync","version":"0.1.0"}}}
{"method":"initialized","params":{}}
{"method":"thread/list","id":2,"params":{"cursor":null,"limit":1,"modelProviders":null,"sourceKinds":[]}}
```

必须设置响应超时、读取 stderr、关闭子进程，并在 Windows 使用无窗口启动标志。

## 12. 备份与恢复

每次写入前建立操作级备份：

```text
backups/<job-id>/
├─ manifest.json
├─ session_index.jsonl
├─ .codex-global-state.json
├─ sqlite/
└─ rollouts/
```

备份清单记录：

- 操作 ID
- 创建时间
- 源和目标 Profile
- 文件原始路径
- 写入前 SHA-256
- 写入后 SHA-256
- 客户端版本
- App Server 版本
- 恢复状态

恢复操作必须先停止目标实例，并清理对应 SQLite 的 WAL/SHM sidecar 后再恢复数据库文件。

## 13. 工具自己的数据库

工具使用独立 SQLite，不修改 Codex 数据库来保存自身状态。

数据库固定位于：

```text
<launcher-directory>/CodexLocalSync.data/app.sqlite
```

建议表：

```text
profiles
provider_thread_replicas
```

`provider_thread_replicas` 用于记录同一逻辑线程在不同 Provider 中的来源、副本和内容指纹。
旧版 `sync_baselines`、`sync_jobs` 及任务历史表不再创建或使用。

## 14. Tauri Command 设计

第一阶段建议提供：

```text
profile_list
profile_create
profile_update
profile_delete
profile_detect_default
profile_validate

session_scan
session_list
session_get_details

sync_preview
sync_execute
sync_cancel
sync_history

backup_list
backup_restore

app_server_detect
app_server_rebuild_index
process_get_profile_status
```

长时间任务通过 Tauri Event 报告进度：

```text
sync://progress
sync://completed
sync://failed
scan://progress
index://progress
```

## 15. 界面设计

主界面采用三栏工作台：

```text
实例列表 | 会话列表 | 会话详情与同步操作
```

### 15.1 实例列表

- Profile 名称
- 登录账号或自定义 API 类型
- Provider
- `CODEX_HOME`
- 客户端运行状态
- 会话数量
- 最后扫描时间

### 15.2 会话列表

- 标题
- 项目路径
- 最后更新时间
- 所在实例
- 归档状态
- 同步状态
- 冲突状态

支持按标题、内容、项目和实例筛选。

### 15.3 同步面板

- 来源实例
- 目标实例
- 选中会话
- 变更预览
- 冲突决策
- 预计备份大小
- 执行进度

### 15.4 历史与恢复

- 同步任务历史
- 成功、跳过、失败数量
- 错误详情
- 备份路径
- 恢复操作

## 16. 跨平台处理

### 16.1 macOS

- 支持 Apple Silicon 和 Intel。
- 定位 `.app/Contents/Resources/codex`。
- 使用 `.app` 同级的 `CodexLocalSync.data` 保存工具状态，不使用系统 Application Support 目录。
- 发布版本需要 Developer ID 签名和 notarization。
- 处理文件系统隐私授权和应用启动的子进程归属提示。
- 同级目录无写权限时，使用签名的 privileged helper 或受控的 Authorization Services 流程请求管理员授权，只创建目标 `.data` 目录并将其所有权交给当前用户。
- `/Applications` 可能需要管理员授权；只读 DMG 即使授权也不能写入，必须先移动应用。

### 16.2 Windows

- 支持桌面安装和 Microsoft Store 安装路径。
- 使用无窗口方式启动 App Server。
- 处理 SQLite 文件锁、WAL 和 SHM。
- 统一处理 `C:\`、UNC 和长路径前缀。
- 发布 MSI 和可选 NSIS 安装包。
- 便携版不在启动文件同级 `CodexLocalSync.data` 之外写入工具状态。安装版也保持同样的数据布局，除非用户明确切换为系统安装模式。
- 同级目录无写权限时，通过 UAC 启动一次性辅助进程创建 `.data` 目录，并为当前用户设置明确的目录 ACL；主程序随后降回普通权限运行。

### 16.3 客户端安全退出与重启

同步前由工具自动处理目标 ChatGPT/Codex 客户端，不要求用户手动关闭：

```text
检测目标 Profile 对应进程
  → 检查是否存在运行中任务
  → 请求客户端正常退出
  → 等待进程退出和文件句柄释放
  → 超时后提示用户选择继续等待或强制结束
  → 执行同步与索引重建
  → 按用户设置重新启动原实例
```

macOS 优先发送应用级 Quit 事件；Windows 优先发送正常窗口关闭请求。两端都必须等待子进程、App Server 和 SQLite 文件句柄释放。正常退出失败时不能直接静默 `kill`，因为可能中断正在运行的任务或丢失尚未落盘的数据。用户确认强制结束后，工具才可以终止准确匹配目标 Profile 的进程树。

进程识别不能只按进程名称判断，必须结合可执行文件路径、启动参数、`CODEX_HOME` 和已记录 PID，避免关闭其他账号实例。同步完成后，只有此前由工具关闭的实例才允许自动重启。

### 16.4 路径规范化

内部数据结构使用 `PathBuf`，不以字符串拼接路径。仅在 UI 和序列化边界转换为字符串。比较路径时尽可能使用规范化后的绝对路径，但不能要求目标路径已经存在。

## 17. 安全要求

1. 不读取或同步无关目录。
2. 不将 Token、API Key 或聊天内容写入日志。
3. 所有目标路径必须验证位于用户选择或已注册的 Profile 目录内。
4. 不跟随会逃逸 Profile 根目录的符号链接。
5. 写入采用临时文件、`fsync` 和原子替换。
6. 同步任务使用实例级锁，避免两个任务同时写同一目录。
7. 删除操作默认进入工具废纸篓，不直接永久删除。
8. 导入包需要校验 manifest、相对路径和 SHA-256，防止路径穿越。

## 18. 错误与恢复策略

- 扫描失败：保留其他实例结果并标记单实例错误。
- rollout 无效：跳过该会话，不影响其他会话。
- 索引更新失败：恢复索引备份，删除本次新增 rollout。
- App Server 重建失败：保留已同步文件，提示重启客户端后重试索引修复。
- 验证失败：自动尝试回滚本次任务。
- 进程仍在运行：自动请求正常退出；超时后等待用户确认是否强制结束，未确认则取消同步。
- 磁盘空间不足：在备份和复制前中止。

## 19. 测试策略

### 19.1 Rust 单元测试

- rollout 解析
- `session_meta` 识别
- Provider 改写
- JSONL 完整性校验
- 前缀和分叉判断
- session index upsert
- Windows/macOS 路径规范化
- 备份和恢复
- 导入包路径穿越防护

### 19.2 集成测试

- 两个临时 `CODEX_HOME` 的单向复制
- 已存在相同会话时跳过
- 源领先时更新
- 双端分叉时产生冲突
- App Server 索引重建
- 失败后的事务回滚
- Windows SQLite WAL 场景

### 19.3 端到端测试

1. 在源 Profile 创建真实会话。
2. 关闭目标客户端。
3. 使用工具同步会话。
4. 启动目标 Profile。
5. 确认左侧列表出现会话。
6. 打开会话并检查历史。
7. 在目标端继续一次对话。
8. 扫描并验证源、目标状态判断正确。

## 20. 开发阶段

### 阶段 0：技术验证

- 创建两个临时 `CODEX_HOME`。
- 复制一条真实 rollout。
- 改写 Provider。
- 更新 `session_index.jsonl`。
- 调用官方 App Server 重建索引。
- 在 macOS 和 Windows 客户端确认左侧列表可见。

退出条件：两个平台至少各验证一个当前正式客户端版本。

### 阶段 1：MVP

- Profile 管理
- 会话扫描和列表
- 手动单向同步
- Provider 可见性修复
- App Server 索引重建
- 自动备份和恢复
- 目标运行时禁止写入

### 阶段 2：双向同步

- 同步基线
- 增量检测
- 前缀判断
- 冲突提示
- 同步历史
- 批量同步

### 阶段 3：产品化

- 自动同步
- ZIP 导入导出
- 废纸篓
- 崩溃恢复
- 自动更新
- macOS 签名和公证
- Windows 安装和签名

## 21. MVP 验收标准

1. 可以在 macOS 和 Windows 创建至少两个独立 Profile。
2. 可以扫描并展示每个 Profile 的本地会话。
3. 可以将目标中不存在的会话同步过去。
4. 同步后的会话能够出现在目标官方客户端左侧列表。
5. 会话标题、正文、项目路径和更新时间基本保持一致。
6. 同步前自动创建可恢复备份。
7. 相同会话不会重复复制。
8. 分叉会话不会被静默覆盖。
9. 目标客户端运行时，工具可以自动安全退出目标实例，确认文件句柄释放后再写入，并可在同步完成后恢复启动。
10. 全流程不上传聊天内容，不复制认证凭据。
11. Windows 单个 `.exe` 旁边只生成一个 `CodexLocalSync.data` 数据目录。
12. macOS `.app` 旁边只生成一个 `CodexLocalSync.data` 数据目录，不向 `.app/Contents` 写入运行数据。
13. 删除应用启动文件不会自动删除 `.data` 目录，用户可以单独备份或恢复该目录。
14. 同级目录需要管理员权限时，只在初始化阶段提权创建目录并授予当前用户权限，后续同步不重复提权。
15. 强制结束客户端必须明确确认，并且只能作用于与目标 Profile 精确匹配的进程树。

## 22. 已知风险

- 官方本地存储格式和 SQLite schema 可能随客户端版本变化。
- 某些工具调用、附件或生成文件依赖 Profile 外部路径，复制 rollout 不代表相关文件也存在。
- 同一会话跨 Provider 继续时，目标模型不一定支持原始工具或加密推理项。
- 官方 App Server 的索引重建行为可能发生变化。
- 客户端运行时写入可能覆盖外部同步结果，因此 MVP 必须要求目标实例停止。

应为每个受支持客户端版本保存兼容性测试结果，检测到未知格式时切换为只读模式。

## 23. 许可证与实现边界

本方案参考了 `cockpit-tools` 已公开展示的产品行为和总体技术路径。该项目使用 CC BY-NC-SA 4.0，并限制商业使用。

如果本工具存在商业化可能，应采用 clean-room 方式实现：

- 不复制其源码、测试代码或资源。
- 依据公开的 Codex 格式、官方 App Server 文档和独立测试重新设计实现。
- 保留本项目自己的命名、模块边界、数据结构和测试夹具。
- 如需直接使用其代码，应先取得作者的书面商业授权。

## 24. 下一步

第一项实现任务是完成阶段 0 的双 `CODEX_HOME` 技术验证。验证程序只需提供命令行入口，先确认当前 macOS 和 Windows 客户端版本中以下链路稳定：

```text
扫描 rollout
  → 复制到目标
  → 改写 Provider
  → 更新 session index
  → 调用官方 App Server
  → 左侧列表可见
```

验证通过后再搭建完整的 Tauri + Vue 界面，避免在底层兼容性尚未确认前投入大量 UI 开发。
