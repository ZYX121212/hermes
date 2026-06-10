// crates/tools/src/vision.rs
// Vision 工具：通过 LLM Vision API 分析图片。
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use std::path::Path;

use crate::{Tool, ToolOutput};

/// 图片分析工具，通过 LLM Vision API 理解图片内容。
pub struct VisionTool {
    /// 用于调用 Vision API 的 HTTP client 和 API 配置
    api_base: String,
    api_key: String,
    model: String,
    http: reqwest::Client,
}

impl VisionTool {
    pub fn new(api_base: String, api_key: String, model: String) -> Self {
        Self {
            api_base,
            api_key,
            model,
            http: reqwest::Client::new(),
        }
    }

    fn read_image_base64(&self, path: &str) -> anyhow::Result<(String, String)> {
        let path = Path::new(path);
        let mime = match path.extension().and_then(|e| e.to_str()) {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some(ext) => anyhow::bail!("unsupported image format: {ext}"),
            None => anyhow::bail!("cannot determine image format (no extension)"),
        };
        let data = std::fs::read(path)?;
        Ok((BASE64.encode(&data), mime.to_string()))
    }

    async fn call_vision_api(&self, image_path: &str, prompt: &str) -> anyhow::Result<String> {
        let (b64, mime) = self.read_image_base64(image_path)?;

        let body = serde_json::json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": prompt},
                    {"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", mime, b64)}}
                ]
            }],
            "max_tokens": 1024,
        });

        let resp = self
            .http
            .post(format!("{}/v1/chat/completions", self.api_base))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("(vision API returned no text)")
            .to_string();
        Ok(text)
    }
}

#[async_trait]
impl Tool for VisionTool {
    fn name(&self) -> &str {
        "vision_analyze"
    }

    fn description(&self) -> &str {
        "Analyze an image file using AI vision. Provide an image path and a question/prompt about the image. Returns a textual description or answer."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "image_path": {
                    "type": "string",
                    "description": "Path to the image file (PNG, JPEG, GIF, WebP)"
                },
                "prompt": {
                    "type": "string",
                    "description": "What to look for or ask about the image"
                }
            },
            "required": ["image_path", "prompt"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let image_path = args["image_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("vision_analyze: 'image_path' is required"))?;
        let prompt = args["prompt"]
            .as_str()
            .unwrap_or("Describe this image in detail.");

        let result = self.call_vision_api(image_path, prompt).await?;
        Ok(ToolOutput::text(result))
    }
}

/// 图片描述生成器：接受截图或图片，生成简短描述。
pub struct VisionDescribeTool {
    inner: VisionTool,
}

impl VisionDescribeTool {
    pub fn new(api_base: String, api_key: String, model: String) -> Self {
        Self {
            inner: VisionTool::new(api_base, api_key, model),
        }
    }
}

#[async_trait]
impl Tool for VisionDescribeTool {
    fn name(&self) -> &str {
        "vision_describe"
    }

    fn description(&self) -> &str {
        "Generate a concise description of an image. Useful for screenshots or photos where you need to quickly understand the visual content."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "image_path": {
                    "type": "string",
                    "description": "Path to the image file to describe"
                }
            },
            "required": ["image_path"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let image_path = args["image_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("vision_describe: 'image_path' is required"))?;
        let result = self
            .inner
            .call_vision_api(
                image_path,
                "Describe this image concisely. What do you see?",
            )
            .await?;
        Ok(ToolOutput::text(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_has_required_fields() {
        let tool = VisionTool::new("http://localhost".into(), "key".into(), "gpt-4".into());
        let schema = tool.schema();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("image_path")));
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("prompt")));
    }

    #[test]
    fn test_describe_schema() {
        let tool = VisionDescribeTool::new("http://localhost".into(), "key".into(), "gpt-4".into());
        let schema = tool.schema();
        assert_eq!(schema["required"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_invalid_extension() {
        let tool = VisionTool::new("http://localhost".into(), "key".into(), "gpt-4".into());
        let result = tool.read_image_base64("test.txt");
        assert!(result.is_err());
    }
}
