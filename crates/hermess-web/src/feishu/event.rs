// crates/hermess-web/src/feishu/event.rs
// 飞书 WebSocket 事件类型定义
use serde::Deserialize;

/// WebSocket 推送事件的外层信封
#[derive(Debug, Deserialize)]
pub struct EventEnvelope {
    pub schema: String,
    pub header: EventHeader,
    pub event: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct EventHeader {
    pub event_id: String,
    pub event_type: String,
    pub create_time: String,
    pub token: String,
    pub app_id: String,
}

/// im.message.receive_v1 事件体
#[derive(Debug, Deserialize)]
pub struct MessageReceiveEvent {
    pub sender: Sender,
    pub message: Message,
}

#[derive(Debug, Deserialize)]
pub struct Sender {
    pub sender_id: SenderId,
}

#[derive(Debug, Deserialize)]
pub struct SenderId {
    pub open_id: String,
    #[serde(default)]
    pub union_id: String,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub message_id: String,
    pub chat_id: String,
    pub chat_type: String,
    pub message_type: String,
    pub content: String,
}

/// 文本消息 content 字段的 JSON 结构
#[derive(Debug, Deserialize)]
pub struct TextContent {
    pub text: String,
}

/// 解析消息 content（JSON 字符串 → TextContent）
pub fn parse_text_content(content: &str) -> Option<String> {
    serde_json::from_str::<TextContent>(content)
        .ok()
        .map(|t| t.text.trim().to_string())
        .filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_envelope() {
        let json = r#"{"schema":"im.message.receive_v1","header":{"event_id":"ev_001","event_type":"im.message.receive_v1","create_time":"1700000000000","token":"t_001","app_id":"cli_a"},"event":{}}"#;
        let env: EventEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(env.schema, "im.message.receive_v1");
        assert_eq!(env.header.event_id, "ev_001");
    }

    #[test]
    fn test_parse_text_content() {
        let content = r#"{"text":" 你好世界 "}"#;
        assert_eq!(parse_text_content(content), Some("你好世界".into()));

        assert_eq!(parse_text_content(r#"{"text":"  "#), None);
        assert_eq!(parse_text_content("not json"), None);
    }
}
