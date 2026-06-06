# Hermess 飞书集成设计

**日期**: 2026-06-06  
**状态**: 已确认

---

## 目标

用飞书（Feishu/Lark）替换现有企业微信集成，使 Hermess Agent 可通过飞书机器人交互，并支持飞书知识库/文档/云盘操作。

## 范围分解

| 优先级 | 子系统 | 说明 |
|--------|--------|------|
| P0 | Bot 消息（WebSocket） | 飞书机器人收发消息，替换 WeChat 回调 |
| P1 | Wiki 知识库 | 读取/搜索企业知识库，可接入 memory 做 RAG |
| P2 | Docs 文档 | 读写 Doc/Sheet 等在线文档 |
| P3 | Drive 云盘 | 文件上传/下载/列表 |

**本次实现 P0**，P1-P3 的 API 方法在 `FeishuClient` 中预留接口签名，具体注册为 tools 的流程后续再完善。

## 技术选型

- **连接方式**：飞书 WebSocket 长连接（`/open-apis/ws/v1/url`）
- **认证方式**：`tenant_access_token`（app_id + app_secret，自动缓存刷新）
- **消息格式**：JSON（飞书原生格式，无需 XML/加解密）
- **会话复用**：使用现有 `SessionManager` + `SmallHermesAgent`，不做修改

## 文件变更清单

### 删除

- `crates/hermess-web/src/wechat/mod.rs`
- `crates/hermess-web/src/wechat/client.rs`
- `crates/hermess-web/src/wechat/crypto.rs`
- `crates/hermess-web/src/wechat/msg.rs`

### 新增

- `crates/hermess-web/src/feishu/mod.rs` — 模块入口
- `crates/hermess-web/src/feishu/client.rs` — REST API 客户端（token 管理 + 消息/Wiki/Docs/Drive API）
- `crates/hermess-web/src/feishu/bot.rs` — WebSocket 长连接 + 事件分发
- `crates/hermess-web/src/feishu/event.rs` — 飞书事件 JSON 类型定义

### 修改

- `crates/hermess-web/src/lib.rs` — `WebAppConfig.wechat` → `.feishu`；`wechat_config` → `feishu_config`
- `crates/hermess-web/src/server.rs` — `AppState` 中 `wx_config/wx_client` → `feishu_client`；移除 `/wechat/callback` 路由
- `crates/hermess-web/src/main.rs` — 初始化 `FeishuClient` → `FeishuBot::run()` spawn
- `config/wechat.toml` → `config/feishu.toml`（格式适配）
- `crates/hermess-web/Cargo.toml` — 移除 wechat 相关依赖（`aes`, `sha1`, `quick_xml`），可能保留 base64/hex 等公共依赖

### 不变

- `crates/hermess-web/src/session.rs` — 接口不变，直接复用
- `crates/hermess-agent/` — 不变
- `crates/memory/` — 不变

## 架构

```
main.rs:
  FeishuClient::new(app_id, app_secret)
       ├── Http server (axum)  ←  /health, /chat 保留
       └── FeishuBot::run()    ←  独立的 tokio task
              └── WebSocket event loop (获取ws_url → 连接 → 读帧 → dispatch)
                     └── on_message()
                            └── SessionManager::get_or_create(user_id)
                                   └── SmallHermesAgent::run_loop(ctx)
                                          └── reply via FeishuClient::reply_text()
```

## 关键数据结构

### FeishuConfig

```rust
pub struct FeishuConfig {
    pub app_id: String,
    pub app_secret: String,
    pub verification_token: String,   // 可选，预留
    pub encrypt_key: Option<String>,  // 可选，预留
}
```

### EventEnvelope（WebSocket 每帧）

```rust
struct EventEnvelope {
    schema: String,              // "im.message.receive_v1" 等
    header: EventHeader,
    event: serde_json::Value,
}
```

### FeishuClient（REST API 客户端）

- `get_tenant_access_token()` — 自动缓存+5分钟提前刷新
- `reply_text(msg_id, content)` — 回复单条消息
- `send_text_to_chat(chat_id, content)` — 发消息到群
- Wiki API 签名预留：`list_spaces`, `get_node_content`, `search_nodes`
- Docs API 签名预留：`get_doc_content`, `create_doc`, `append_to_doc`
- Drive API 签名预留：`upload_file`, `download_file`, `list_files`

### FeishuBot（WebSocket 长连接）

- `run()` — 启动事件循环（阻塞）
- `event_loop()` — 获取 ws_url → 连接 → 循环读帧 → dispatch
- `dispatch(envelope)` — 按 schema 分发到 handler
- `on_message(event)` — 文本消息处理，调用 agent

## 错误处理 & 重连

- Token 获取失败 → 重试（anyhow propagate）
- WebSocket 断开 → 自动重连，退避 1s/2s/4s/8s/16s/30s(cap)，无限重试
- 消息处理失败 → 回复用户错误提示

## Config 迁移

`.toml` 格式从：

```toml
[wechat]
corp_id = "..."
agent_id = "..."
secret = "..."
token = "..."
encoding_aes_key = "..."
```

变为：

```toml
[feishu]
app_id = "..."
app_secret = "..."
```
