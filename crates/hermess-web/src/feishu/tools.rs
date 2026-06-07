// crates/hermess-web/src/feishu/tools.rs
// 飞书 Wiki / Docs / Drive 的 Tool 封装
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::RwLock;
use tools::{Tool, ToolOutput};

use super::client::FeishuClient;

// ── Wiki Tool ──────────────────────────────────────────────

pub struct FeishuWikiTool {
    client: Arc<RwLock<Arc<FeishuClient>>>,
}

impl FeishuWikiTool {
    pub fn new(client: Arc<RwLock<Arc<FeishuClient>>>) -> Self {
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
        let client = { self.client.read().await.clone() };
        let op = args["operation"].as_str().unwrap_or("");
        match op {
            "list_spaces" => {
                let pt = args["page_token"].as_str();
                let ps = args["page_size"].as_u64();
                let result = client.list_wiki_spaces(pt, ps).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "get_space_detail" => {
                let space_id = get_str(&args, "space_id")?;
                let result = client.get_wiki_space_detail(space_id).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "get_node_tree" => {
                let space_id = get_str(&args, "space_id")?;
                let parent = args["parent_node_token"].as_str();
                let result = client.get_wiki_node_tree(space_id, parent).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "get_node_content" => {
                let node_token = get_str(&args, "node_token")?;
                let content = client.get_wiki_node_content(node_token).await?;
                Ok(ToolOutput::text(content))
            }
            "create_node" => {
                let space_id = get_str(&args, "space_id")?;
                let parent = get_str(&args, "parent_node_token")?;
                let title = get_str(&args, "title")?;
                let content = args["content"].as_str().unwrap_or("");
                let result = client
                    .create_wiki_node(space_id, parent, title, content)
                    .await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "update_node" => {
                let node_token = get_str(&args, "node_token")?;
                let title = args["title"].as_str().unwrap_or("");
                let content = args["content"].as_str().unwrap_or("");
                let result = client.update_wiki_node(node_token, title, content).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "search_wiki" => {
                let query = get_str(&args, "query")?;
                let space_ids: Option<Vec<String>> = args["space_ids"].as_array().map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                });
                let result = client.search_wiki(query, space_ids.as_deref()).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            _ => Ok(ToolOutput::error(format!("未知操作: {op}"))),
        }
    }
}

// ── Docs Tool ──────────────────────────────────────────────

pub struct FeishuDocsTool {
    client: Arc<RwLock<Arc<FeishuClient>>>,
}

impl FeishuDocsTool {
    pub fn new(client: Arc<RwLock<Arc<FeishuClient>>>) -> Self {
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
        let client = { self.client.read().await.clone() };
        let op = args["operation"].as_str().unwrap_or("");
        match op {
            "get_document" => {
                let doc_token = get_str(&args, "doc_token")?;
                let content = client.get_document(doc_token).await?;
                Ok(ToolOutput::text(content))
            }
            "create_document" => {
                let title = get_str(&args, "title")?;
                let folder_token = args["folder_token"].as_str();
                let doc_token = client.create_document(title, folder_token).await?;
                Ok(ToolOutput::text(format!(
                    "文档创建成功\ndoc_token: {doc_token}"
                )))
            }
            "get_document_blocks" => {
                let doc_token = get_str(&args, "doc_token")?;
                let result = client.get_document_blocks(doc_token).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "append_blocks" => {
                let doc_token = get_str(&args, "doc_token")?;
                let block_id = get_str(&args, "block_id")?;
                let blocks = &args["blocks"];
                let result = client
                    .append_document_blocks(doc_token, block_id, blocks)
                    .await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "update_block" => {
                let doc_token = get_str(&args, "doc_token")?;
                let block_id = get_str(&args, "block_id")?;
                let update = &args["update"];
                let result = client
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
    client: Arc<RwLock<Arc<FeishuClient>>>,
}

impl FeishuDriveTool {
    pub fn new(client: Arc<RwLock<Arc<FeishuClient>>>) -> Self {
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
                "file_path": { "type": "string", "description": "本地文件路径（上传/下载时使用）" },
                "file_name": { "type": "string", "description": "文件名（非本地上传时使用）" },
                "mime_type": { "type": "string", "description": "MIME 类型" },
                "page_size": { "type": "integer", "description": "每页数量" },
                "page_token": { "type": "string", "description": "分页 token" }
            },
            "required": ["operation"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let client = { self.client.read().await.clone() };
        let op = args["operation"].as_str().unwrap_or("");
        match op {
            "list_files" => {
                let folder = args["folder_token"].as_str();
                let ps = args["page_size"].as_u64();
                let pt = args["page_token"].as_str();
                let result = client.list_drive_files(folder, ps, pt).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "upload_file" => {
                let folder_token = get_str(&args, "folder_token")?;
                let file_path = get_str(&args, "file_path")?;
                let file_name = args["file_name"].as_str().unwrap_or_else(|| {
                    std::path::Path::new(file_path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                });
                let mime_type = args["mime_type"]
                    .as_str()
                    .unwrap_or("application/octet-stream");
                let data = tokio::fs::read(file_path)
                    .await
                    .map_err(|e| anyhow::anyhow!("无法读取文件 {file_path}: {e}"))?;
                let ft = client
                    .upload_drive_file(folder_token, file_name, data, mime_type)
                    .await?;
                Ok(ToolOutput::text(format!("文件上传成功\nfile_token: {ft}")))
            }
            "download_file" => {
                let file_token = get_str(&args, "file_token")?;
                let file_path = args["file_path"].as_str();
                let data = client.download_drive_file(file_token).await?;
                let size = data.len();
                if let Some(path) = file_path {
                    tokio::fs::write(path, &data)
                        .await
                        .map_err(|e| anyhow::anyhow!("无法写入文件 {path}: {e}"))?;
                    Ok(ToolOutput::text(format!(
                        "文件下载成功\n大小: {size} bytes\n已保存到: {path}"
                    )))
                } else {
                    Ok(ToolOutput::text(format!(
                        "文件下载成功\n大小: {size} bytes\n请指定 file_path 参数来保存到本地文件"
                    )))
                }
            }
            "get_file_info" => {
                let file_token = get_str(&args, "file_token")?;
                let result = client.get_drive_file_info(file_token).await?;
                Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
            }
            "delete_file" => {
                let file_token = get_str(&args, "file_token")?;
                client.delete_drive_file(file_token).await?;
                Ok(ToolOutput::text("文件已删除".to_string()))
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
        let client = Arc::new(RwLock::new(FeishuClient::new(
            "app".into(),
            "secret".into(),
        )));
        let tool = FeishuWikiTool::new(client);
        assert_eq!(tool.name(), "feishu_wiki");
        let schema = tool.schema();
        assert!(schema["properties"]["operation"]["enum"].is_array());
    }

    #[test]
    fn test_docs_tool_schema() {
        let client = Arc::new(RwLock::new(FeishuClient::new(
            "app".into(),
            "secret".into(),
        )));
        let tool = FeishuDocsTool::new(client);
        let schema = tool.schema();
        assert_eq!(schema["required"][0].as_str().unwrap(), "operation");
    }

    #[test]
    fn test_drive_tool_schema() {
        let client = Arc::new(RwLock::new(FeishuClient::new(
            "app".into(),
            "secret".into(),
        )));
        let tool = FeishuDriveTool::new(client);
        assert_eq!(tool.name(), "feishu_drive");
    }
}
