# Codex 多 Provider 会话副本同步实施方案

## 1. 文档信息

- 状态：Implemented（App Server 实际副本创建需在客户端中验收）
- 面向版本：下一版架构调整
- 适用平台：Windows、macOS
- 技术栈：Tauri 2、Vue 3、TypeScript、Rust
- 核心场景：多个 Provider 反复覆盖同一个 `CODEX_HOME/config.toml`
- 产品边界：本工具只同步 Codex 本地会话，不提供备份与恢复功能

本文是基于当前项目实现的新方案，不覆盖
`docs/codex-local-session-sync-design.md`。旧文档保留为历史设计参考；涉及“修改原会话
Provider”“跨 `CODEX_HOME` 覆盖目标会话”和“同步前创建备份”的部分，不适用于本方案。

## 2. 决策摘要

项目的主模型从“多个物理 `CODEX_HOME` 之间复制同一个 Thread ID”调整为：

> 在一个物理 `CODEX_HOME` 中，按历史 `model_provider` 对 Codex 会话分组；将其他
> Provider 的原会话复制为新的 Thread，并让新副本属于当前 Provider。首次复制不修改原会话；
> 后续只有用户在当前供应商中显式点击“同步会话”时，才允许更新已登记的来源/副本配对。

必须遵守以下规则：

1. 首次复制不修改来源 rollout、来源 Thread ID、来源 Provider 或来源索引。
2. 不把两条已有会话的 JSONL 直接拼接。
3. 每个同步副本使用新的 Thread ID。
4. 显式同步只更新已有映射中的来源或副本 rollout，Thread ID 和 Provider 保持不变。
5. 不创建 rollout、SQLite、WAL/SHM 或索引备份。
6. 不直接写官方 Codex SQLite；首版只通过官方 App Server 登记和验证线程。
7. 执行失败时删除本次未验证副本，不回滚来源数据。
8. 工具数据库只保存映射、哈希和执行状态，不保存聊天正文，不属于会话备份。
9. 首版只处理 Codex 模式的本地会话，不处理 ChatGPT 普通聊天。

## 3. 问题定义

用户持续覆盖同一个 `~/.codex/config.toml` 或自定义 `CODEX_HOME/config.toml`，例如：

```text
Provider A：官方 openai
Provider B：中转服务一
Provider C：中转服务二（当前配置）
```

这些环境共用一个物理 `CODEX_HOME`。历史 rollout 仍然存在，但各自的
`session_meta.payload.model_provider` 不同。工具直接扫描 JSONL 时能看到全部历史，官方
客户端却可能按当前 Provider、来源类型和归档状态过滤，所以当前项目列表只显示一部分会话。

当前实现不能解决这个场景：

- 左侧 Profile 以物理 `CODEX_HOME` 为唯一键，同一目录不会产生 A、B、C 三个目标；
- 右侧同步要求来源和目标是不同路径；
- 当前 `sync.rs` 保留 Thread ID 并写入另一个目录，不能在同一目录内生成并存副本；
- 当前 `app_server.rs` 只调用一次 `thread/list(limit = 1)`，没有完整分页和逐项验证；
- 当前同步流程会创建备份，与本方案的产品边界冲突。

## 4. 术语

| 术语 | 定义 |
|---|---|
| Home | 一个物理 `CODEX_HOME`，包含配置、rollout 和官方状态库 |
| Provider 分组 | 从 rollout 的 `model_provider` 聚合出的历史会话集合 |
| 当前 Provider | 当前 Codex 客户端实际生效的 `model_provider` |
| 来源会话 | 用户选择复制的历史 Codex Thread，整个流程中保持只读 |
| 同步副本 | 使用新 Thread ID 创建、归属于当前 Provider 的完整历史副本 |
| 映射记录 | 工具数据库中的来源 Thread 与副本 Thread 对应关系，不含聊天正文 |
| 内部线程 | subagent、review、compact 等非主交互线程，首版不展示为可同步会话 |

文案中不再把 Provider A、B、C 称为“实例”。只有物理 `CODEX_HOME` 才叫实例；
`openai`、`OpenAI-API`、`SHUAI-API`、`custom` 等值叫 Provider ID 或 Provider 分组。

## 5. 范围

### 5.1 首版包含

- 扫描一个 Home 下 `sessions/**/*.jsonl` 的有效 Codex 主会话；
- 按 rollout 中真实存在的 Provider ID 分组；
- 识别并固定当前 Provider 作为同步目标；
- 从其他 Provider 选择一条或多条未归档主会话；
- 为每条来源会话创建新 Thread ID 的副本；
- 保留标题、聊天历史、项目路径 `cwd` 和事件顺序；
- 让副本归属于当前 Provider；
- 通过 App Server 触发官方 JSONL 扫描和状态库修复；
- 验证副本出现在当前 Provider 的交互式线程列表；
- 记录来源与副本的映射，阻止重复同步；
- 删除失败过程中由工具新建的未验证副本。

### 5.2 首版不包含

- 备份、备份浏览或备份恢复；
- 修改来源会话的 Provider；
- 自动覆盖两侧都已产生新内容的分叉会话；
- 自动双向同步已经发生分叉的会话；
- 复制 `config.toml`、`auth.json`、Token、API Key、Skills 或 Plugins；
- 同步归档会话；
- 将 subagent 等内部线程作为独立主会话同步；
- ChatGPT 普通聊天、ChatGPT 网页聊天或其他云端聊天同步；
- 首版中直接插入或更新官方 `state_*.sqlite` 表。

## 6. 官方接口边界

本方案依赖以下官方行为：

1. `thread/fork` 会复制已存储历史并生成新的 Thread ID；可选 `lastTurnId` 只复制到指定回合。
2. `thread/list` 支持 `modelProviders`、`sourceKinds`、`archived`、`useStateDbOnly` 和游标分页。
3. `useStateDbOnly` 省略或为 `false` 时，App Server 会扫描 JSONL 并修复状态库元数据。
4. `sourceKinds` 省略或传空数组时默认只返回 `cli` 和 `vscode` 交互式线程。
5. `thread/delete` 可以删除持久化线程及其关联元数据，可用于清理工具创建但验证失败的副本。

官方文档只保证 `thread/fork` 创建新 Thread 并复制历史，没有保证 fork 会把副本切换到当前
Provider。因此实现必须在 fork 后读取副本并检查 `modelProvider`，不能假定结果。

参考：

- [Codex App Server](https://learn.chatgpt.com/docs/app-server)
- [App Server thread/list](https://learn.chatgpt.com/docs/app-server#list-threads-with-pagination--filters)
- [App Server thread/fork](https://learn.chatgpt.com/docs/app-server#start-or-resume-a-thread)

## 7. 产品交互

主界面继续使用三栏工作台，但三栏含义调整为：

```text
Provider 分组 | Codex 会话列表 | 复制到当前 Provider
```

### 7.1 左栏：Provider 分组

每个物理 Home 下展示从两类来源合并得到的 Provider：

- 当前配置：通过 App Server `config/read` 或可靠的配置解析获得；
- 历史会话：从 `session_meta.payload.model_provider` 聚合获得。

每个 Provider 展示：

- 原始 Provider ID；
- 是否为当前 Provider；
- 未归档主会话数量；
- 已同步到当前 Provider 的数量；
- 无法确认的历史配置不伪造显示名，直接展示 Provider ID。

同一个 Provider ID 只形成一个分组。Provider 分组不是独立路径，也不进入现有
`profiles.codex_home UNIQUE` 约束。

### 7.2 中栏：会话列表

默认只列出当前选中 Provider 下满足以下条件的会话：

- 位于 `sessions/`，不是 `archived_sessions/`；
- JSONL 有合法 `session_meta` 和 Thread ID；
- `source` 属于 `vscode` 或 `cli`；
- 不是工具创建的同步副本；
- 来源 Provider 与当前 Provider 不同。

每行展示：

- 标题；
- 项目路径；
- 更新时间；
- 来源 Provider；
- 同步状态：未同步、已同步、来源已更新、不可同步；
- 不可同步原因。

归档会话和内部线程可以显示在只读筛选页，但首版不提供同步按钮。

### 7.3 右栏：复制到当前 Provider

目标固定为当前 Provider，用户不再选择另一个物理实例。面板展示：

- 当前 Provider ID；
- 选中会话数量；
- 将创建的新副本数量；
- 已存在映射而跳过的数量；
- 无效或不可同步数量；
- 预计新增 rollout 大小；
- 执行前预览。

主动作使用“复制到当前 Provider”。不要使用“覆盖”“迁移原会话”或“发送到另一个实例”。

## 8. 数据模型

### 8.1 Provider 分组

```rust
struct ProviderBucket {
    profile_id: String,
    provider_id: String,
    is_current: bool,
    active_root_thread_count: usize,
    archived_thread_count: usize,
    internal_thread_count: usize,
    replicated_count: usize,
}
```

`provider_id` 必须保留原始大小写。展示和匹配不能擅自把 `OpenAI-API` 改成产品名称。

### 8.2 会话扩展字段

```rust
enum ThreadSourceKind {
    Cli,
    Vscode,
    Internal,
    Unknown,
}

enum ReplicationEligibility {
    Eligible,
    CurrentProvider,
    Archived,
    InternalThread,
    InvalidRollout,
    AlreadyReplicated,
    SourceUpdated,
}

struct ProviderSessionRecord {
    thread_id: String,
    provider_id: String,
    source_kind: ThreadSourceKind,
    archived: bool,
    title: String,
    cwd: Option<String>,
    updated_at: Option<String>,
    size_bytes: u64,
    sha256: String,
    eligibility: ReplicationEligibility,
    replica_thread_id: Option<String>,
}
```

### 8.3 工具数据库

新增表只保存同步关系和执行结果：

```sql
CREATE TABLE provider_thread_replicas (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    source_thread_id TEXT NOT NULL,
    source_provider_id TEXT NOT NULL,
    target_provider_id TEXT NOT NULL,
    replica_thread_id TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    replica_sha256 TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    verified_at TEXT,
    deleted_at TEXT,
    UNIQUE(profile_id, source_thread_id, target_provider_id),
    UNIQUE(profile_id, replica_thread_id)
);

```

不得在工具数据库保存 rollout 正文、消息正文、认证信息或操作任务历史。数据库只保存
供应商登记信息和 `provider_thread_replicas` 复制映射；复制/同步结果只在当前操作弹窗中返回。

## 9. Tauri 接口

新增接口：

```text
provider_list(profile_id)
provider_session_list(profile_id, provider_id, scope)
replication_preview(profile_id, source_thread_ids)
replication_execute(profile_id, source_thread_ids)
replication_history(profile_id)
replication_cleanup_orphans(profile_id)
```

建议返回结构：

```rust
struct ReplicationPreview {
    profile_id: String,
    target_provider_id: String,
    items: Vec<ReplicationPlanItem>,
    create_count: usize,
    skip_count: usize,
    invalid_count: usize,
    estimated_bytes: u64,
}

enum ReplicationAction {
    CreateReplica,
    SkipAlreadyReplicated,
    SourceUpdated,
    SkipCurrentProvider,
    SkipArchived,
    SkipInternal,
    Invalid,
}

struct ReplicationResult {
    job_id: String,
    target_provider_id: String,
    created: Vec<ReplicaResultItem>,
    skipped: Vec<ReplicaResultItem>,
    failed: Vec<ReplicaResultItem>,
}
```

前端不得直接读取文件或数据库，仍只调用 Tauri Command。

## 10. App Server 客户端改造

当前 `app_server.rs::rebuild_index` 应拆成可复用客户端，而不是每个请求都写死 ID 和
`limit = 1`：

```rust
struct AppServerClient {
    child: Child,
    stdin: ChildStdin,
    receiver: Receiver<Value>,
    next_request_id: i64,
}
```

至少实现：

```text
initialize()
config_read()
thread_list_all(filters)
thread_read(thread_id, include_turns)
thread_fork(thread_id)
thread_name_set(thread_id, name)
thread_delete(thread_id)
shutdown()
```

`thread_list_all` 必须循环读取 `nextCursor`，不能用一页或一条结果代替完整验证。

默认来源修复调用：

```json
{
  "method": "thread/list",
  "params": {
    "cursor": null,
    "limit": 100,
    "modelProviders": null,
    "sourceKinds": ["cli", "vscode"],
    "archived": false,
    "useStateDbOnly": false
  }
}
```

目标验证调用使用：

```json
{
  "method": "thread/list",
  "params": {
    "cursor": null,
    "limit": 100,
    "modelProviders": ["<current-provider>"],
    "sourceKinds": ["cli", "vscode"],
    "archived": false,
    "useStateDbOnly": false
  }
}
```

具体协议字段必须以运行时 App Server 返回和当前官方 schema 为准。遇到未知字段或方法不支持时
应失败关闭，不能转为直接修改 SQLite。

## 11. 复制算法

### 11.1 预览

1. 读取当前 Home 和已登记 Profile。
2. 通过 App Server `config/read` 获取有效配置中的当前 Provider。
3. 与磁盘 `config.toml` 的解析结果交叉校验；结果冲突时阻止执行并提示重启客户端或重新扫描。
4. 扫描来源会话，校验路径仍位于 Home 内且不是符号链接逃逸。
5. 只允许未归档的 `cli`/`vscode` 根会话。
6. 计算来源 JSONL 的规范化 SHA-256。
7. 查询 `provider_thread_replicas`：
   - 没有映射：`CreateReplica`；
   - 映射存在且来源哈希相同：`SkipAlreadyReplicated`；
   - 映射存在但来源哈希变化：`SourceUpdated`，首版不自动覆盖；
   - 目标就是当前 Provider：`SkipCurrentProvider`。
8. 返回只读预览，不启动写入。

### 11.2 执行前检查

1. 重新读取当前 Provider，必须与预览一致。
2. 重新计算来源哈希，必须与预览一致。
3. 获取 Home 级独占同步锁。
4. 检测客户端状态并请求正常退出，避免同时写官方线程状态。
5. 启动当前 Home 对应的 App Server 并完成 `initialize`。
6. 调用完整分页 `thread/list`，让 App Server 扫描 JSONL 并修复缺失的来源索引。
7. 确认每个来源 Thread 都可以通过 `thread/read` 读取。

### 11.3 单条副本创建

每条来源会话独立执行，允许批量任务部分成功：

1. 调用 `thread/fork(source_thread_id)`，禁止 `ephemeral = true`。
2. 从响应取得新的 `replica_thread_id`，必须与来源 ID 不同。
3. 定位 fork 生成的 rollout，并验证它位于当前 Home 的 `sessions/` 内。
4. 读取副本 `session_meta`，检查 fork 后 Provider：
   - 已等于当前 Provider：不改文件；
   - 仍为来源 Provider：只对这个新副本结构化改写
     `session_meta.payload.model_provider`；
   - 缺少或出现多个互相冲突的 `session_meta`：删除副本并失败。
5. 文件调整必须采用临时文件、完整 JSONL 校验和原子替换。来源文件不参与写入。
6. 使用 `thread/name/set` 恢复来源标题；如果官方接口拒绝标题设置，保留副本但在结果中报告警告。
7. 再次调用目标 Provider 的完整分页 `thread/list(useStateDbOnly = false)`。
8. 验证：
   - 返回列表包含 `replica_thread_id`；
   - `modelProvider` 等于当前 Provider；
   - `cwd` 与来源一致；
   - 副本可由 `thread/read(includeTurns = true)` 读取；
   - 来源 rollout 的执行前后 SHA-256 完全一致。
9. 验证成功后才写入 `provider_thread_replicas(status = verified)`。

### 11.4 失败清理

在映射记录完成前失败时：

1. 优先调用 `thread/delete(replica_thread_id)` 删除工具刚创建的副本及其官方元数据；
2. 再次扫描确认副本不在目标列表；
3. 删除成功则将任务项标记为 `failed_cleaned`；
4. 删除失败则标记为 `orphaned`，保留副本 ID，交给“清理未完成副本”操作处理；
5. 任何失败都不能修改或删除来源会话。

这不是备份回滚。清理对象仅限本次任务创建、尚未验证交付的副本。

### 11.5 显式同步已有会话

顶部“同步会话”按钮只在左侧选中当前供应商时显示，并只处理 verified 映射：

- 点击后先显示同步预览，列出需要同步、冲突和无效条目数量及逐条原因；
- 同步按钮位于右侧操作区的刷新会话按钮左边；
- 只有列表状态为“来源已更新”或“副本已更新”的未归档主会话进入同步计划；
- 首次进入和每次切换供应商时，根据本地 rollout 位置、来源类型和最新哈希重新计算状态；

- 仅来源哈希变化：将来源的完整最新内容写入现有副本，同时保留副本 Thread ID 和 Provider；
- 仅副本哈希变化：将副本的完整最新内容写入现有来源，同时保留来源 Thread ID 和 Provider；
- 两侧哈希都未变化：跳过；
- 两侧哈希都变化：判定为分叉冲突，不自动覆盖任何一侧；
- 同步成功后同时更新映射中的来源哈希和副本哈希，不创建第三个 Thread。

写入使用完整 JSONL 解析、身份字段结构化改写、临时文件和原子替换。同步命令必须在
Tauri 阻塞工作线程执行，不占用窗口事件循环。

## 12. 项目列表可见性

同步副本必须保留来源 `cwd`。验证重点不是“文件已经复制”，而是：

1. App Server 在当前 Provider 过滤下能够列出副本；
2. Codex 客户端重启后能在对应本地项目下看到副本；
3. 项目目录不存在时，会话仍可复制，但结果标记为“项目路径不可用”；
4. 首版不直接编辑 `.codex-global-state.json` 强行添加项目；若客户端不能仅依据线程 `cwd`
   建立项目可见性，应先寻找官方项目接口，再决定后续兼容层。

## 13. ChatGPT 模式边界

官方文档明确说明：Quick chat 创建普通 ChatGPT 聊天，这些聊天不会出现在 Codex 侧栏；客户端
提供从 New chat 打开已有 ChatGPT 聊天并添加到 Codex 聊天的官方入口。

因此：

- `CODEX_HOME/sessions` 中可解析的 Codex rollout 属于本工具范围；
- ChatGPT 普通聊天不假设存在于本地 rollout，不扫描浏览器缓存或客户端私有数据库；
- 不读取 ChatGPT 云端会话 API，不处理账号 Token；
- UI 中不得把“未在本地找到 ChatGPT 普通聊天”显示为扫描失败；
- 用户需要把 ChatGPT 普通聊天带入 Codex 时，使用客户端官方“添加到 Codex 聊天”流程。

参考：[Projects and chats - Quick chat](https://learn.chatgpt.com/docs/projects#use-quick-chat-for-a-quick-question)

## 14. 当前代码改造映射

| 文件 | 改造内容 |
|---|---|
| `src-tauri/src/models.rs` | 增加 Provider 分组、复制资格、预览和结果模型 |
| `src-tauri/src/sessions.rs` | 解析 source kind；标记根线程、内部线程、归档和工具副本 |
| `src-tauri/src/app_server.rs` | 改为长连接客户端；增加 config/read、完整分页 list、read、fork、name/set、delete |
| `src-tauri/src/replication.rs` | 新增只读预览、副本执行、验证和失败清理核心 |
| `src-tauri/src/store.rs` | 增加 replica/job 表和幂等查询，不保存会话正文 |
| `src-tauri/src/lib.rs` | 注册 provider、replication 系列 Tauri Command |
| `src-tauri/src/sync.rs` | 从主流程移除；旧跨 Home 能力如需保留，应放到独立兼容入口且不得触发备份 |
| `src/types.ts` | 对齐新的 Provider、Session、Preview、Result 类型 |
| `src/services/backend.ts` | 增加 provider/replication API 包装和演示数据 |
| `src/stores/workspace.ts` | 状态改为 selectedProviderId、currentProviderId、replicationPreview |
| `src/App.vue` | 三栏由“实例 → 会话 → 目标实例”改为“Provider → 会话 → 当前 Provider” |
| `src/styles.css` | 增加当前 Provider、同步状态、不可同步状态和结果样式 |

已有 `profiles` 表继续管理物理 Home。不要为了 Provider 分组向 `profiles` 表插入同路径伪实例。

## 15. 状态机

任务状态：

```text
planned
  -> running
  -> completed | partial | failed
```

单条项目状态：

```text
planned
  -> source_indexed
  -> forked
  -> provider_checked
  -> target_indexed
  -> verified
```

失败终态：

```text
failed_cleaned   副本已删除，来源未变
orphaned         副本清理失败，保留副本 ID 等待显式清理
```

只有 `verified` 项目可以写入正式映射并计入同步成功。

## 16. 安全约束

- 不读取或记录 `auth.json` 内容；
- 不把 API Key 写入日志、预览或工具数据库；
- 所有文件路径必须 canonicalize 后仍位于当前 Home；
- 拒绝写入符号链接、父目录跳转和 Home 外路径；
- 来源 rollout 全流程只读，并在执行后复核哈希；
- 只有 App Server 返回的新 Thread ID 才能成为可清理副本；
- verified 副本不能由失败清理逻辑自动删除；
- Provider 在预览与执行之间变化时整个任务停止；
- App Server 协议、Provider 调整或索引验证失败时，不降级为直接改官方 SQLite；
- 不创建 `backups/` 新内容，已有历史备份目录不自动删除。

## 17. 测试方案

### 17.1 Rust 单元测试

- 按 Provider ID 分组并保持大小写；
- 正确识别 `cli`、`vscode` 和内部 source；
- 归档、内部、当前 Provider、无效 JSONL 不可同步；
- 相同来源与目标映射保持幂等；
- 来源哈希变化返回 `SourceUpdated`；
- 来源文件在成功和失败流程中哈希不变；
- 路径逃逸和符号链接被拒绝；
- 只允许改写 fork 副本的第一个合法 `session_meta` Provider。

### 17.2 App Server 协议测试

使用可控假服务覆盖：

- `thread/list` 多页游标；
- 来源缺少状态库记录但 JSONL 扫描后出现；
- fork 返回新 Thread ID；
- fork 继承来源 Provider；
- fork 自动使用当前 Provider；
- `thread/name/set` 警告不阻断主体成功；
- 目标列表未出现副本时调用 delete；
- delete 失败产生 orphaned 记录；
- Provider 在执行中变化时停止任务。

### 17.3 集成测试

在临时 Home 构造 A、B、C 三个 Provider：

1. 当前 Provider 为 C；
2. A、B 各有两条 `vscode` 主会话；
3. A 有一条 archived，会被排除；
4. B 有一条 subagent，会被排除；
5. 从 A、B 各复制一条到 C；
6. C 中出现两个新 Thread ID；
7. A、B 来源文件哈希不变；
8. 第二次执行跳过已有映射；
9. 整个过程没有创建任何备份文件。

### 17.4 客户端验收

Windows 和 macOS 各验证：

- 当前 Provider 的项目列表出现同步副本；
- 可以打开并阅读完整历史；
- 可以在副本中继续对话；
- 来源 Provider 原会话仍然存在；
- 切回来源 Provider 后原会话仍可见；
- 归档与内部线程没有进入普通项目列表；
- ChatGPT 普通聊天没有被误报为可同步 Codex 会话。

## 18. 实施阶段

### 阶段 0：能力验证

- 在临时 Home 中验证当前 App Server 的 fork 结果；
- 确认 fork rollout 的定位方式；
- 确认改写新副本 Provider 后 `thread/list(useStateDbOnly = false)` 能修复状态库；
- 确认 `thread/delete` 能完整清理失败副本；
- 任何一项不满足时停止进入写入阶段，不在用户 Home 上实验。

### 阶段 1：只读 Provider 工作台

- 增加 Provider 分组和 source kind；
- 改造左栏与中栏；
- 实现 replication preview；
- 保留旧同步入口但默认隐藏，暂不执行副本写入。

### 阶段 2：副本执行

- 完成 App Server 长连接客户端；
- 实现 fork、Provider 检查、目标扫描、验证和失败清理；
- 增加 replica/job 映射表；
- 删除新流程中的所有备份调用。

### 阶段 3：交互与诊断

- 完成右栏预览、进度和部分成功结果；
- 增加 orphaned 副本诊断与显式清理；
- 增加来源更新状态；
- 完成 Windows/macOS 客户端可见性验收。

### 阶段 4：旧流程收敛

- 根据实际需求决定删除还是保留跨 Home 兼容入口；
- 停止新建 `CodexLocalSync.data/backups`；
- 保留旧数据库记录和用户已有文件，不执行自动清理；
- 更新 README、旧设计文档状态和发布说明。

## 19. 完成标准

以下条件全部满足才算方案落地：

1. 一个 Home 中能看到 A、B、C Provider 分组；
2. 当前为 C 时，可以选择 A/B 的有效主会话；
3. 每条同步结果在 C 下拥有新的 Thread ID；
4. 来源 rollout 的内容和哈希完全不变；
5. 工具不直接修改官方 SQLite；
6. 执行失败只清理本次未验证副本；
7. 重复同步不会产生重复副本；
8. 单侧更新可显式同步到已有配对；两侧都更新时不会被静默覆盖；
9. Codex 客户端当前 Provider 项目列表可以打开副本并继续使用；
10. 全流程不创建备份文件；
11. ChatGPT 普通聊天不进入本地 Codex 同步流程。
