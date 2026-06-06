# Feishu Wiki / Docs / Drive API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 17 Feishu API methods (Wiki ×7, Docs ×5, Drive ×5) to FeishuClient and wrap them as 3 Tool implementations for Agent use.

**Architecture:** API methods go into FeishuClient following the existing reqwest + token pattern. Each domain gets a Tool struct in `feishu/tools.rs` implementing `tools::Tool`, dispatched by an `operation` string field. Tools are registered in main.rs.

**Tech Stack:** reqwest (REST), serde_json (response parsing), existing tools::Tool trait

---

### Task 1: client.rs — Wiki API 方法

**Files:**
- Modify: `crates/hermess-web/src/feishu/client.rs`

- [ ] **Step 1: 在 FeishuClient impl 块末尾添加 Wiki 方法**

在 `reply_text` 方法的闭括号 `}` 之后（`}` 结束后），`}` （impl 块结束之前）插入以下代码：

```rust
    // ── Wiki 知识库 API ──────────────────────────────────────

    /// 列出可访问的知识空间
    pub async fn list_wiki_spaces(
        &self,
        page_token: Option<&str>,
        page_size: Option<u64>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!("{}/open-apis/wiki/v2/spaces", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .query(&[
                ("page_token", page_token.unwrap_or("")),
                ("page_size", &page_size.unwrap_or(20).to_string()),
            ])
            .send()
            .await
            .context("failed to list wiki spaces")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    /// 获取知识空间详情
    pub async fn get_wiki_space_detail(&self, space_id: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!(
                "{}/open-apis/wiki/v2/spaces/{space_id}",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to get wiki space detail")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    /// 获取知识空间节点树
    pub async fn get_wiki_node_tree(
        &self,
        space_id: &str,
        parent_node_token: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let mut query = vec![];
        if let Some(pt) = parent_node_token {
            query.push(("parent_node_token", pt.to_string()));
        }
        let resp = self
            .http
            .get(format!(
                "{}/open-apis/wiki/v2/spaces/{space_id}/nodes",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .query(&query.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>())
            .send()
            .await
            .context("failed to get wiki node tree")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    /// 读取知识库节点内容（纯文本）
    pub async fn get_wiki_node_content(&self, node_token: &str) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!(
                "{}/open-apis/wiki/v2/spaces/-/nodes/{node_token}",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to get wiki node")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        // 提取纯文本内容
        let content = body["data"]["node"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(content)
    }

    /// 在知识空间中创建文档节点
    pub async fn create_wiki_node(
        &self,
        space_id: &str,
        parent_node_token: &str,
        title: &str,
        content: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .post(format!(
                "{}/open-apis/wiki/v2/spaces/{space_id}/nodes",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({
                "parent_node_token": parent_node_token,
                "node_type": "docx",
                "title": title,
                "content": content,
            }))
            .send()
            .await
            .context("failed to create wiki node")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    /// 更新知识库节点
    pub async fn update_wiki_node(
        &self,
        node_token: &str,
        title: &str,
        content: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .put(format!(
                "{}/open-apis/wiki/v2/spaces/-/nodes/{node_token}",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({
                "title": title,
                "content": content,
            }))
            .send()
            .await
            .context("failed to update wiki node")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    /// 全文搜索知识库
    pub async fn search_wiki(
        &self,
        query: &str,
        space_ids: Option<&[String]>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let mut body_json = serde_json::json!({ "query": query });
        if let Some(ids) = space_ids {
            body_json["space_ids"] = serde_json::json!(ids);
        }
        let resp = self
            .http
            .post(format!("{}/open-apis/wiki/v2/search", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .json(&body_json)
            .send()
            .await
            .context("failed to search wiki")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }
```

- [ ] **Step 2: 在文件末尾（test 模块之前）添加 check_feishu_code 辅助函数**

```rust
/// 检查飞书 API 响应的 code 字段，非 0 返回错误
fn check_feishu_code(body: &serde_json::Value) -> anyhow::Result<()> {
    let code = body["code"].as_i64().unwrap_or(-1);
    if code != 0 {
        let msg = body["msg"].as_str().unwrap_or("unknown error");
        anyhow::bail!("feishu api error: code={code} msg={msg}");
    }
    Ok(())
}
```

- [ ] **Step 3: Build + test**

```bash
cargo check -p hermess-web 2>&1
cargo test -p hermess-web 2>&1
```

Expected: 编译通过，已有测试全部通过。

- [ ] **Step 4: Commit**

```bash
git add crates/hermess-web/src/feishu/client.rs
git commit -m "feat(feishu): add Wiki API methods (list, search, CRUD nodes)"
```

---

### Task 2: client.rs — Docs API 方法

**Files:**
- Modify: `crates/hermess-web/src/feishu/client.rs`

- [ ] **Step 1: 在 Wiki 方法之后（impl 块内）添加 Docs 方法**

```rust
    // ── Docs 文档 API ──────────────────────────────────────

    /// 读取 Doc 完整内容（纯文本）
    pub async fn get_document(&self, doc_token: &str) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!(
                "{}/open-apis/docx/v1/documents/{doc_token}",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to get document")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        let content = body["data"]["document"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(content)
    }

    /// 创建 Doc 文档，返回 doc_token
    pub async fn create_document(
        &self,
        title: &str,
        folder_token: Option<&str>,
    ) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let mut json = serde_json::json!({ "title": title });
        if let Some(ft) = folder_token {
            json["folder_token"] = serde_json::json!(ft);
        }
        let resp = self
            .http
            .post(format!("{}/open-apis/docx/v1/documents", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .json(&json)
            .send()
            .await
            .context("failed to create document")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        let doc_token = body["data"]["document"]["document_id"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(doc_token)
    }

    /// 获取文档所有块
    pub async fn get_document_blocks(&self, doc_token: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!(
                "{}/open-apis/docx/v1/documents/{doc_token}/blocks",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to get document blocks")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    /// 向文档块追加子块
    pub async fn append_document_blocks(
        &self,
        doc_token: &str,
        block_id: &str,
        blocks: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .post(format!(
                "{}/open-apis/docx/v1/documents/{doc_token}/blocks/{block_id}/children",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({
                "children": blocks,
            }))
            .send()
            .await
            .context("failed to append document blocks")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    /// 更新文档块
    pub async fn update_document_block(
        &self,
        doc_token: &str,
        block_id: &str,
        update: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .patch(format!(
                "{}/open-apis/docx/v1/documents/{doc_token}/blocks/{block_id}",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .json(update)
            .send()
            .await
            .context("failed to update document block")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }
```

- [ ] **Step 2: Build + test**

```bash
cargo check -p hermess-web 2>&1
cargo test -p hermess-web 2>&1
```

Expected: 编译通过，已有测试全部通过。

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-web/src/feishu/client.rs
git commit -m "feat(feishu): add Docs API methods (read, create, append blocks)"
```

---

### Task 3: client.rs — Drive API 方法

**Files:**
- Modify: `crates/hermess-web/src/feishu/client.rs`

- [ ] **Step 1: 在 Docs 方法之后（impl 块内）添加 Drive 方法**

```rust
    // ── Drive 云盘 API ──────────────────────────────────────

    /// 列出文件夹内容
    pub async fn list_drive_files(
        &self,
        folder_token: Option<&str>,
        page_size: Option<u64>,
        page_token: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let mut query: Vec<(&str, String)> = vec![];
        if let Some(ft) = folder_token {
            query.push(("folder_token", ft.to_string()));
        }
        if let Some(ps) = page_size {
            query.push(("page_size", ps.to_string()));
        }
        if let Some(pt) = page_token {
            query.push(("page_token", pt.to_string()));
        }
        let resp = self
            .http
            .get(format!("{}/open-apis/drive/v1/files", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .query(&query.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>())
            .send()
            .await
            .context("failed to list drive files")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    /// 上传文件到云盘（返回 file_token）
    pub async fn upload_drive_file(
        &self,
        folder_token: &str,
        file_name: &str,
        data: Vec<u8>,
        mime_type: &str,
    ) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let part = reqwest::multipart::Part::bytes(data)
            .file_name(file_name.to_string())
            .mime_str(mime_type)
            .context("invalid mime type")?;
        let form = reqwest::multipart::Form::new()
            .text("folder_token", folder_token.to_string())
            .text("file_name", file_name.to_string())
            .part("file", part);
        let resp = self
            .http
            .post(format!(
                "{}/open-apis/drive/v1/files/upload_all",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await
            .context("failed to upload file")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        let file_token = body["data"]["file_token"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(file_token)
    }

    /// 下载文件内容
    pub async fn download_drive_file(&self, file_token: &str) -> anyhow::Result<Vec<u8>> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!(
                "{}/open-apis/drive/v1/files/{file_token}/download",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to download file")?;
        let bytes = resp.bytes().await.context("failed to read file bytes")?;
        Ok(bytes.to_vec())
    }

    /// 获取文件元信息
    pub async fn get_drive_file_info(&self, file_token: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!(
                "{}/open-apis/drive/v1/files/{file_token}",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to get file info")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    /// 删除文件
    pub async fn delete_drive_file(&self, file_token: &str) -> anyhow::Result<()> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .delete(format!(
                "{}/open-apis/drive/v1/files/{file_token}",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to delete file")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(())
    }
```

- [ ] **Step 2: Build + test**

```bash
cargo check -p hermess-web 2>&1
cargo test -p hermess-web 2>&1
```

Expected: 编译通过，已有测试全部通过。

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-web/src/feishu/client.rs
git commit -m "feat(feishu): add Drive API methods (list, upload, download, delete)"
```

---

### Task 4: tools.rs — 3 个 Tool 实现

**Files:**
- Create: `crates/hermess-web/src/feishu/tools.rs`

- [ ] **Step 1: 创建 tools.rs**

```rust
// crates/hermess-web/src/feishu/tools.rs
// 飞书 Wiki / Docs / Drive 的 Tool 封装
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tools::{Tool, ToolOutput};

use super::client::FeishuClient;

// ── Wiki Tool ──────────────────────────────────────────────

pub struct FeishuWikiTool {
    client: Arc<FeishuClient>,
}

impl FeishuWikiTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuWikiTool {
    fn name(&self) -> &str {
        "feishu_wiki"
    }

    fn description(&self) -> &str {
        "飞书知识库操作：列出空间、搜索内容、读写节点。支持 operation: list_spaces, get_space_detail, get_node_tree, get_node_content, create_node, update_node, search_wiki"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["list_spaces", "get_space_detail", "get_node_tree", "get_node_content", "create_node", "update_node", "search_wiki"],
                    "description": "要执行的操作"
                },
                "space_id": { "type": "string", "description": "知识空间 ID" },
                "node_token": { "type": "string", "description": "节点 token" },
                "parent_node_token": { "type": "string", "description": "父节点 token" },
                "title": { "type": "string", "description": "文档标题" },
                "content": { "type": "string", "description": "文档内容（纯文本/Markdown）" },
                "query": { "type": "string", "description": "搜索关键词" },
                "page_token": { "type": "string", "description": "分页 token" },
                "page_size": { "type": "integer", "description": "每页数量，默认 20" }
            },
            "required": ["operation"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let op = args["operation"].as_str().unwrap_or("");
        match op {
            "list_spaces" => {
                let pt = args["page_token"].as_str();
                let ps = args["page_size"].as_u64();
                let result = self.client.list_wiki_spaces(pt, ps).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "get_space_detail" => {
                let space_id = get_str(&args, "space_id")?;
                let result = self.client.get_wiki_space_detail(space_id).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "get_node_tree" => {
                let space_id = get_str(&args, "space_id")?;
                let parent = args["parent_node_token"].as_str();
                let result = self.client.get_wiki_node_tree(space_id, parent).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "get_node_content" => {
                let node_token = get_str(&args, "node_token")?;
                let content = self.client.get_wiki_node_content(node_token).await?;
                Ok(ToolOutput::text(content))
            }
            "create_node" => {
                let space_id = get_str(&args, "space_id")?;
                let parent = get_str(&args, "parent_node_token")?;
                let title = get_str(&args, "title")?;
                let content = args["content"].as_str().unwrap_or("");
                let result = self
                    .client
                    .create_wiki_node(space_id, parent, title, content)
                    .await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "update_node" => {
                let node_token = get_str(&args, "node_token")?;
                let title = args["title"].as_str().unwrap_or("");
                let content = args["content"].as_str().unwrap_or("");
                let result = self
                    .client
                    .update_wiki_node(node_token, title, content)
                    .await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "search_wiki" => {
                let query = get_str(&args, "query")?;
                let space_ids: Option<Vec<String>> = args["space_ids"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
                let result = self
                    .client
                    .search_wiki(query, space_ids.as_deref())
                    .await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            _ => Ok(ToolOutput::error(format!("未知操作: {op}"))),
        }
    }
}

// ── Docs Tool ──────────────────────────────────────────────

pub struct FeishuDocsTool {
    client: Arc<FeishuClient>,
}

impl FeishuDocsTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuDocsTool {
    fn name(&self) -> &str {
        "feishu_docs"
    }

    fn description(&self) -> &str {
        "飞书文档操作：读写 Doc 文档。支持 operation: get_document, create_document, get_document_blocks, append_blocks, update_block"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["get_document", "create_document", "get_document_blocks", "append_blocks", "update_block"],
                    "description": "要执行的操作"
                },
                "doc_token": { "type": "string", "description": "文档 token" },
                "block_id": { "type": "string", "description": "块 ID" },
                "title": { "type": "string", "description": "文档标题（创建时使用）" },
                "folder_token": { "type": "string", "description": "文件夹 token（创建时使用）" },
                "content": { "type": "string", "description": "文本内容" },
                "blocks": { "type": "array", "description": "要追加的块数组（JSON）" },
                "update": { "type": "object", "description": "要更新的块内容（JSON）" }
            },
            "required": ["operation"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let op = args["operation"].as_str().unwrap_or("");
        match op {
            "get_document" => {
                let doc_token = get_str(&args, "doc_token")?;
                let content = self.client.get_document(doc_token).await?;
                Ok(ToolOutput::text(content))
            }
            "create_document" => {
                let title = get_str(&args, "title")?;
                let folder_token = args["folder_token"].as_str();
                let doc_token = self.client.create_document(title, folder_token).await?;
                Ok(ToolOutput::text(format!("文档创建成功\ndoc_token: {doc_token}")))
            }
            "get_document_blocks" => {
                let doc_token = get_str(&args, "doc_token")?;
                let result = self.client.get_document_blocks(doc_token).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "append_blocks" => {
                let doc_token = get_str(&args, "doc_token")?;
                let block_id = get_str(&args, "block_id")?;
                let blocks = &args["blocks"];
                let result = self
                    .client
                    .append_document_blocks(doc_token, block_id, blocks)
                    .await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "update_block" => {
                let doc_token = get_str(&args, "doc_token")?;
                let block_id = get_str(&args, "block_id")?;
                let update = &args["update"];
                let result = self
                    .client
                    .update_document_block(doc_token, block_id, update)
                    .await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            _ => Ok(ToolOutput::error(format!("未知操作: {op}"))),
        }
    }
}

// ── Drive Tool ─────────────────────────────────────────────

pub struct FeishuDriveTool {
    client: Arc<FeishuClient>,
}

impl FeishuDriveTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuDriveTool {
    fn name(&self) -> &str {
        "feishu_drive"
    }

    fn description(&self) -> &str {
        "飞书云盘操作：文件上传/下载/列表/删除。支持 operation: list_files, upload_file, download_file, get_file_info, delete_file"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["list_files", "upload_file", "download_file", "get_file_info", "delete_file"],
                    "description": "要执行的操作"
                },
                "folder_token": { "type": "string", "description": "文件夹 token" },
                "file_token": { "type": "string", "description": "文件 token" },
                "file_path": { "type": "string", "description": "本地文件路径（上传时使用）" },
                "file_name": { "type": "string", "description": "文件名（非本地上传时使用）" },
                "mime_type": { "type": "string", "description": "MIME 类型" },
                "page_size": { "type": "integer", "description": "每页数量" },
                "page_token": { "type": "string", "description": "分页 token" }
            },
            "required": ["operation"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let op = args["operation"].as_str().unwrap_or("");
        match op {
            "list_files" => {
                let folder = args["folder_token"].as_str();
                let ps = args["page_size"].as_u64();
                let pt = args["page_token"].as_str();
                let result = self.client.list_drive_files(folder, ps, pt).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "upload_file" => {
                let folder_token = get_str(&args, "folder_token")?;
                let file_path = get_str(&args, "file_path")?;
                let file_name = args["file_name"]
                    .as_str()
                    .unwrap_or_else(|| std::path::Path::new(file_path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown"));
                let mime_type = args["mime_type"].as_str().unwrap_or("application/octet-stream");
                let data = tokio::fs::read(file_path).await.map_err(|e| {
                    anyhow::anyhow!("无法读取文件 {file_path}: {e}")
                })?;
                let ft = self
                    .client
                    .upload_drive_file(folder_token, file_name, data, mime_type)
                    .await?;
                Ok(ToolOutput::text(format!("文件上传成功\nfile_token: {ft}")))
            }
            "download_file" => {
                let file_token = get_str(&args, "file_token")?;
                let file_path = args["file_path"].as_str();
                let data = self.client.download_drive_file(file_token).await?;
                let size = data.len();
                // 如果指定了 file_path，将内容写入本地文件
                if let Some(path) = file_path {
                    tokio::fs::write(path, &data).await.map_err(|e| {
                        anyhow::anyhow!("无法写入文件 {path}: {e}")
                    })?;
                    Ok(ToolOutput::text(format!("文件下载成功\n大小: {size} bytes\n已保存到: {path}")))
                } else {
                    // 无 file_path 时只返回大小信息
                    Ok(ToolOutput::text(format!("文件下载成功\n大小: {size} bytes\n请指定 file_path 参数来保存到本地文件")))
                }
            }
            "get_file_info" => {
                let file_token = get_str(&args, "file_token")?;
                let result = self.client.get_drive_file_info(file_token).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "delete_file" => {
                let file_token = get_str(&args, "file_token")?;
                self.client.delete_drive_file(file_token).await?;
                Ok(ToolOutput::text("文件已删除"))
            }
            _ => Ok(ToolOutput::error(format!("未知操作: {op}"))),
        }
    }
}

// ── Helper ─────────────────────────────────────────────────

fn get_str<'a>(args: &'a serde_json::Value, key: &str) -> anyhow::Result<&'a str> {
    args[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("缺少参数: {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wiki_tool_schema() {
        let client = FeishuClient::new("app".into(), "secret".into());
        let tool = FeishuWikiTool::new(client);
        assert_eq!(tool.name(), "feishu_wiki");
        let schema = tool.schema();
        assert!(schema["properties"]["operation"]["enum"].is_array());
    }

    #[test]
    fn test_docs_tool_schema() {
        let client = FeishuClient::new("app".into(), "secret".into());
        let tool = FeishuDocsTool::new(client);
        let schema = tool.schema();
        assert_eq!(
            schema["required"][0].as_str().unwrap(),
            "operation"
        );
    }

    #[test]
    fn test_drive_tool_schema() {
        let client = FeishuClient::new("app".into(), "secret".into());
        let tool = FeishuDriveTool::new(client);
        assert_eq!(tool.name(), "feishu_drive");
    }
}
```

- [ ] **Step 2: Build + test**

```bash
cargo check -p hermess-web 2>&1
cargo test -p hermess-web 2>&1
```

Expected: 编译通过，所有测试通过（包括新的 3 个 tool schema 测试）。

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-web/src/feishu/tools.rs
git commit -m "feat(feishu): add Wiki/Docs/Drive Tool implementations"
```

---

### Task 5: mod.rs + main.rs — 注册

**Files:**
- Modify: `crates/hermess-web/src/feishu/mod.rs`
- Modify: `crates/hermess-web/src/main.rs`

- [ ] **Step 1: 更新 mod.rs**

将 `crates/hermess-web/src/feishu/mod.rs` 改为：

```rust
// crates/hermess-web/src/feishu/mod.rs
pub mod bot;
pub mod client;
pub mod event;
pub mod tools;
```

- [ ] **Step 2: 更新 main.rs — 注册 3 个 Tool**

在 `main.rs` 中找到 `tools.register(Arc::new(hermess_finance::tool::FinancialTool::new(` 之前的位置，插入：

```rust
    // ── 飞书 Wiki/Docs/Drive Tools ──────────────────────────
    tools.register(Arc::new(
        hermess_web::feishu::tools::FeishuWikiTool::new(Arc::clone(&feishu_client)),
    ));
    tools.register(Arc::new(
        hermess_web::feishu::tools::FeishuDocsTool::new(Arc::clone(&feishu_client)),
    ));
    tools.register(Arc::new(
        hermess_web::feishu::tools::FeishuDriveTool::new(Arc::clone(&feishu_client)),
    ));
```

- [ ] **Step 3: Build + test**

```bash
cargo check --workspace 2>&1
cargo test -p hermess-web 2>&1
```

Expected: 编译通过，所有测试通过。

- [ ] **Step 4: Commit**

```bash
git add crates/hermess-web/src/feishu/mod.rs crates/hermess-web/src/main.rs
git commit -m "feat(feishu): register Wiki/Docs/Drive tools in main"
```

---

### Task 6: 最终验证

- [ ] **Step 1: 全 workspace 编译**

```bash
cargo check --workspace 2>&1
```

Expected: 0 errors

- [ ] **Step 2: 运行所有测试**

```bash
cargo test -p hermess-web 2>&1
```

Expected: 所有测试通过（包括 3 个 event 测试 + 3 个 tool schema 测试 = 6 个）

- [ ] **Step 3: 如有编译错误，修复并 commit**

```bash
git add -A && git commit -m "fix(feishu): compile/test fixes for Wiki/Docs/Drive"
```
