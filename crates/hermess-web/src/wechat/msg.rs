// 企业微信 XML 消息解析与构建
use anyhow::{bail, Context};
use quick_xml::de::from_str;
use serde::Deserialize;

/// 企业微信推送的加密消息结构（内层，解密后）
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename = "xml")]
pub struct InnerMessage {
    #[serde(rename = "ToUserName")]
    pub to_user_name: String,
    #[serde(rename = "FromUserName")]
    pub from_user_name: String,
    #[serde(rename = "CreateTime")]
    pub create_time: String,
    #[serde(rename = "MsgType")]
    pub msg_type: String,
    #[serde(rename = "Content", default)]
    pub content: String,
    #[serde(rename = "MsgId", default)]
    pub msg_id: String,
    #[serde(rename = "AgentID", default)]
    pub agent_id: String,
    /// 事件类型 (subscribe / enter_agent 等)
    #[serde(rename = "Event", default)]
    pub event: String,
}

/// 企业微信 POST 到回调 URL 的加密外层 XML
#[derive(Debug, Deserialize)]
#[serde(rename = "xml")]
pub struct EncryptedMessage {
    #[serde(rename = "ToUserName")]
    pub to_user_name: String,
    /// 企业微信 CorpID
    #[serde(rename = "AgentID", default)]
    pub agent_id: String,
    /// Base64 编码的密文
    #[serde(rename = "Encrypt")]
    pub encrypt: String,
}

/// 解析加密回调的 XML 正文，返回 (encrypted_body, msg_signature, timestamp, nonce 信息)
/// 实际解析发生在 handler 层，这里提供纯 XML → EncryptedMessage 解析
pub fn parse_encrypted_xml(xml_str: &str) -> anyhow::Result<EncryptedMessage> {
    from_str::<EncryptedMessage>(xml_str).context("failed to parse encrypted XML message")
}

/// 解密并解析内层消息
pub fn parse_message(xml_str: &str) -> anyhow::Result<InnerMessage> {
    let msg: InnerMessage =
        from_str(xml_str).context("failed to parse inner XML message")?;

    // 只处理 text 类型消息
    if msg.msg_type != "text" {
        bail!("unsupported message type: {}", msg.msg_type);
    }

    Ok(msg)
}

/// 构建文本回复 XML（用于被动回复，需要加密后包装）
pub fn build_text_reply(to_user: &str, from_user: &str, content: &str) -> String {
    format!(
        r#"<xml>
<ToUserName><![CDATA[{}]]></ToUserName>
<FromUserName><![CDATA[{}]]></FromUserName>
<CreateTime>{}</CreateTime>
<MsgType><![CDATA[text]]></MsgType>
<Content><![CDATA[{}]]></Content>
</xml>"#,
        to_user,
        from_user,
        chrono::Utc::now().timestamp(),
        content
    )
}

/// 包装加密回复 XML（外层）
pub fn wrap_encrypted_reply(
    encrypted_content: &str,
    msg_signature: &str,
    timestamp: u64,
    nonce: &str,
) -> String {
    format!(
        r#"<xml>
<Encrypt><![CDATA[{}]]></Encrypt>
<MsgSignature><![CDATA[{}]]></MsgSignature>
<TimeStamp>{}</TimeStamp>
<Nonce><![CDATA[{}]]></Nonce>
</xml>"#,
        encrypted_content, msg_signature, timestamp, nonce
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_inner_message() {
        let xml = r#"<xml>
<ToUserName><![CDATA[ww123]]></ToUserName>
<FromUserName><![CDATA[user1]]></FromUserName>
<CreateTime>1409659813</CreateTime>
<MsgType><![CDATA[text]]></MsgType>
<Content><![CDATA[帮我查一下天气]]></Content>
<MsgId>1234567890</MsgId>
<AgentID>1000001</AgentID>
</xml>"#;

        let msg = parse_message(xml).unwrap();
        assert_eq!(msg.from_user_name, "user1");
        assert_eq!(msg.content, "帮我查一下天气");
        assert_eq!(msg.msg_type, "text");
    }

    #[test]
    fn test_build_text_reply() {
        let reply = build_text_reply("user1", "ww123", "已完成查询");
        assert!(reply.contains("user1"));
        assert!(reply.contains("已完成查询"));
    }

    #[test]
    fn test_parse_encrypted_message() {
        let xml = r#"<xml>
<ToUserName><![CDATA[ww123]]></ToUserName>
<AgentID><![CDATA[1000002]]></AgentID>
<Encrypt><![CDATA[base64_encrypted_content]]></Encrypt>
</xml>"#;

        let msg = parse_encrypted_xml(xml).unwrap();
        assert_eq!(msg.to_user_name, "ww123");
        assert_eq!(msg.encrypt, "base64_encrypted_content");
    }
}
