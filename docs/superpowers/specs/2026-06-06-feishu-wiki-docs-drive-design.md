# Hermess 飞书 Wiki / Docs / Drive 集成设计

**日期**: 2026-06-06  
**状态**: 已确认  
**依赖**: 飞书 Bot 集成已完成 (docs/superpowers/specs/2026-06-06-feishu-integration-design.md)

---

## 目标

在现有 FeishuClient 基础上，添加飞书 Wiki（知识库）、Docs（文档）、Drive（云盘）的完整 REST API 方法，并封装为 tools::Tool 供 Agent 调用。

## 范围

### Wiki 知识库 API（7 个方法）

| 方法 | HTTP | 说明 |
|------|------|------|
| `list_spaces` | GET /open-apis/wiki/v2/spaces | 列出可访问的知识空间 |
| `get_space_detail` | GET /open-apis/wiki/v2/spaces/{space_id} | 获取空间详情 |
| `get_node_tree` | GET /open-apis/wiki/v2/spaces/{space_id}/nodes | 获取节点树（可按 parent_token 子级） |
| `get_node_content` | GET /open-apis/wiki/v2/spaces/{space_id}/nodes/{node_token} | 读取节点完整内容 |
| `create_node` | POST /open-apis/wiki/v2/spaces/{space_id}/nodes | 创建文档节点 |
| `update_node` | PUT /open-apis/wiki/v2/spaces/{space_id}/nodes/{node_token} | 更新节点标题或内容 |
| `search_wiki` | GET /open-apis/wiki/v2/search | 全文搜索知识库 |

### Docs 文档 API（5 个方法）

| 方法 | HTTP | 说明 |
|------|------|------|
| `get_document` | GET /open-apis/docx/v1/documents/{doc_token} | 读取文档完整内容（纯文本） |
| `create_document` | POST /open-apis/docx/v1/documents | 创建 Doc |
| `get_document_blocks` | GET /open-apis/docx/v1/documents/{doc_token}/blocks | 获取所有块 |
| `append_blocks` | POST /open-apis/docx/v1/documents/{doc_token}/blocks/{block_id}/children | 追加子块 |
| `update_block` | PATCH /open-apis/docx/v1/documents/{doc_token}/blocks/{block_id} | 更新块内容 |

### Drive 云盘 API（5 个方法）

| 方法 | HTTP | 说明 |
|------|------|------|
| `list_files` | GET /open-apis/drive/v1/files | 列出文件/文件夹 |
| `upload_file` | POST /open-apis/drive/v1/files/upload_all | 上传文件（multipart） |
| `download_file` | GET /open-apis/drive/v1/files/{file_token}/download | 下载文件（返回 bytes） |
| `get_file_info` | GET /open-apis/drive/v1/files/{file_token} | 获取文件元信息 |
| `delete_file` | DELETE /open-apis/drive/v1/files/{file_token} | 删除文件 |

## 文件变更

### 修改

- `crates/hermess-web/src/feishu/client.rs` — 新增 17 个 API 方法
- `crates/hermess-web/src/feishu/mod.rs` — 添加 `pub mod tools;`
- `crates/hermess-web/src/main.rs` — 注册 3 个新 Tool

### 新增

- `crates/hermess-web/src/feishu/tools.rs` — 3 个 Tool 实现（FeishuWikiTool, FeishuDocsTool, FeishuDriveTool）

## 架构

```
FeishuClient (client.rs)
  ├── get_tenant_access_token()  [已有]
  ├── reply_text()               [已有]
  ├── Wiki 方法 × 7              [新增]
  ├── Docs 方法 × 5              [新增]
  └── Drive 方法 × 5             [新增]

FeishuWikiTool (tools.rs)        [新增]
  └── call(args) → match operation →
        "list_spaces" | "get_node_content" | "search_wiki" | ...

FeishuDocsTool (tools.rs)        [新增]
  └── call(args) → match operation →
        "get_document" | "create_document" | "append_blocks" | ...

FeishuDriveTool (tools.rs)       [新增]
  └── call(args) → match operation →
        "list_files" | "upload_file" | "download_file" | ...

main.rs:
  tools.register(FeishuWikiTool)
  tools.register(FeishuDocsTool)
  tools.register(FeishuDriveTool)
```

## Tool Schema 设计

每个 Tool 使用统一的 `operation` 参数模式：

```json
{
  "type": "object",
  "properties": {
    "operation": {
      "type": "string",
      "enum": ["list_spaces", "get_content", "search", ...],
      "description": "要执行的操作"
    },
    // 各操作的特定参数（均为 optional，按需提供）
    "space_id":    { "type": "string" },
    "node_token":  { "type": "string" },
    "doc_token":   { "type": "string" },
    "query":       { "type": "string" },
    "title":       { "type": "string" },
    "content":     { "type": "string" },
    "folder_token":{ "type": "string" },
    "file_path":   { "type": "string" }  // 本地文件路径，用于上传
  },
  "required": ["operation"]
}
```

## 错误处理

- 所有 API 返回 `{code, msg, data}`，统一检查 `code != 0` → `anyhow::Error`
- 超时默认 30s（大文件上传/下载可调整）
- ToolOutput::error 返回 user-friendly 中文错误信息

## 测试

- FeishuClient 新增方法：单元测试只测类型创建（类似已有 pattern）
- Tool schema：验证 schema JSON 格式正确
- API 集成测试依赖飞书真实环境，不在本次范围内
