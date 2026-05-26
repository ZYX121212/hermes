// crates/tools/src/builtin/reply.rs
// ReplyTool: 直接回应用户，不执行任何外部操作。
// 当任务不需要 shell/文件/搜索时，Planner 应优先选择此工具。
use crate::{Tool, ToolOutput};
use async_trait::async_trait;

/// 直接以自然语言回应用户。用于对话、解释、总结等无需外部工具的任务。
pub struct ReplyTool;

#[async_trait]
impl Tool for ReplyTool {
    fn name(&self) -> &str {
        "reply"
    }

    fn description(&self) -> &str {
        "Directly reply to the user in natural language (Chinese). \
         Use this tool when the task is conversational -- asking questions, \
         giving explanations, making summaries, or providing information \
         that doesn't require reading/writing files, running commands, \
         or searching the web. This is the DEFAULT tool for most requests."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The natural language reply to send back to the user"
                }
            },
            "required": ["content"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let content = args["content"]
            .as_str()
            .unwrap_or("(empty reply)")
            .to_string();
        Ok(ToolOutput::text(content))
    }
}
