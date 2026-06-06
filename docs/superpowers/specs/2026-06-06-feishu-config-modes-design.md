# Hermess 飞书多模式配置设计

**日期**: 2026-06-06  
**状态**: 已确认  
**依赖**: 飞书 Bot 集成 + Wiki/Docs/Drive API 已完成

---

## 目标

提供三种飞书配置途径，支持运行时热加载：

1. **TUI 设置面板** — 在 Settings 中新增"飞书"tab
2. **CLI + 环境变量** — `--feishu-app-id` / `--feishu-app-secret` 和 `FEISHU_APP_ID` / `FEISHU_APP_SECRET`
3. **Agent 对话** — 用户对 agent 说"接入飞书"，agent 写配置并调 reload API
4. **Admin API** — `POST /admin/feishu/reload` 运行时热加载

## 文件变更

### 修改

| 文件 | 变更 |
|------|------|
| `crates/tui/src/settings_store.rs` | UserSettings 新增 `feishu_app_id`, `feishu_app_secret`；保存时同步写入 config/feishu.toml |
| `crates/tui/src/state.rs` | SettingsTab 枚举新增 `Feishu` |
| `crates/tui/src/panels/settings.rs` | 新增飞书 tab 的两个 Text 字段定义 |
| `crates/hermess-web/src/server.rs` | AppState 改为 RwLock 包装；新增 `/admin/feishu/reload` 和 `/admin/feishu/status` |
| `crates/hermess-web/src/main.rs` | CLI 新增 `--feishu-app-id` / `--feishu-app-secret`；环境变量读取 |
| `crates/hermess-web/src/feishu/tools.rs` | Tool 内部 client 改用 `Arc<RwLock<Arc<FeishuClient>>>` |
| `crates/hermess-web/src/feishu/bot.rs` | 不变（通过外部 abort+respawn 实现重连） |

### 不变

- `crates/hermess-web/src/feishu/client.rs` — 无需改动
- `crates/hermess-web/src/feishu/event.rs` — 无需改动
- `crates/hermess-web/src/session.rs` — 无需改动

## 关键数据结构变更

### AppState（server.rs）

```rust
// 变更前
pub struct AppState {
    pub feishu_client: Arc<FeishuClient>,
    pub sessions: Arc<SessionManager>,
    pub api_key: String,
}

// 变更后
pub struct AppState {
    pub feishu_client: Arc<RwLock<Arc<FeishuClient>>>,
    pub feishu_bot_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    pub sessions: Arc<SessionManager>,
    pub api_key: String,
}
```

### UserSettings（settings_store.rs）

新增字段（均 `#[serde(default)]`）：

```rust
pub feishu_app_id: String,
pub feishu_app_secret: String,
```

### SettingsTab（state.rs）

```rust
pub enum SettingsTab {
    Llm, Search, Finance, Theme,
    Feishu,  // ← 新增
}
```

Tab 顺序：LLM → 搜索 → 金融 → 飞书 → 主题

## Admin API

### POST /admin/feishu/reload

请求体：
```json
{"app_id": "cli_xxx", "app_secret": "yyy"}
```

处理流程：
1. 验证 Bearer token（复用现有 auth middleware）
2. 创建新 FeishuClient
3. write lock 替换 AppState.feishu_client
4. abort 旧 FeishuBot 任务
5. spawn 新 FeishuBot 任务
6. Tool 内部通过 RwLock 自动跟随新 client
7. 返回 `{"status": "ok"}`

### GET /admin/feishu/status

返回：
```json
{"connected": true, "reconnect_count": 0, "app_id": "cli_xxx"}
```

## 配置优先级

```
CLI --feishu-app-id > 环境变量 FEISHU_APP_ID > config/feishu.toml [feishu].app_id
```

## Agent 对话配置流程

用户 → agent: "接入飞书，app_id 是 cli_xxx，secret 是 yyy"

Agent 执行：
1. `curl -X POST localhost:8080/admin/feishu/reload -H "Authorization: Bearer $HERMESS_API_KEY" -d '{"app_id":"...","app_secret":"..."}'`
2. 验证返回 `{"status": "ok"}`
3. 回复用户 "飞书已连接成功"

(Agent 有 BashTool，可以直接 curl)
