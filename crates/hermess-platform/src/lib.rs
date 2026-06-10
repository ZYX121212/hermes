//! 统一多平台适配层：定义 PlatformAdapter trait 和共享 Message 类型。

use async_trait::async_trait;
use dashmap::DashMap;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// 标准化的入站消息类型。
#[derive(Debug, Clone)]
pub struct InboundMessage {
    /// 唯一消息 ID（平台定义）
    pub message_id: String,
    /// 发送者用户 ID（平台定义）
    pub user_id: String,
    /// 会话/频道 ID（用于路由 agent 实例）
    pub chat_id: String,
    /// 消息文本内容
    pub text: String,
    /// 消息类型
    pub kind: MessageKind,
    /// 平台来源名称（"telegram", "discord", "slack", "feishu", etc.）
    pub platform: String,
    /// 原始消息 payload（平台特定格式）
    pub raw: serde_json::Value,
}

/// 消息类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageKind {
    /// 文本消息
    Text,
    /// 图片消息
    Image {
        url: String,
        caption: Option<String>,
    },
    /// 文件消息
    File {
        url: String,
        name: Option<String>,
        mime: Option<String>,
    },
    /// 命令消息（如 /start, /help）
    Command { command: String, args: String },
    /// 按钮回调
    Button { callback_data: String },
    /// 语音消息
    Voice {
        url: String,
        transcription: Option<String>,
    },
    /// 未知类型
    Unknown,
}

/// 出站消息（平台无关格式）。
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    /// 目标 chat_id
    pub chat_id: String,
    /// 文本内容
    pub text: String,
    /// 可选的审批按钮
    pub approval_buttons: Option<ApprovalButtons>,
    /// 是否作为回复
    pub reply_to: Option<String>,
}

/// 审批按钮配置。
#[derive(Debug, Clone)]
pub struct ApprovalButtons {
    pub action_id: String,
    pub approve_label: String,
    pub deny_label: String,
}

/// 平台适配器 trait：每个平台实现此接口。
#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// 平台名称标识。
    fn platform_name(&self) -> &str;

    /// 启动适配器（开始接收消息）。
    async fn start(&self) -> anyhow::Result<()>;

    /// 停止适配器。
    async fn stop(&self) -> anyhow::Result<()>;

    /// 发送消息到指定 chat。
    async fn send_message(&self, msg: OutboundMessage) -> anyhow::Result<()>;

    /// 获取入站消息接收器（平台将接收到的消息通过此 channel 发送）。
    fn inbound_rx(&self) -> tokio::sync::mpsc::UnboundedReceiver<InboundMessage>;

    /// 是否支持审批按钮。
    fn supports_approval_buttons(&self) -> bool {
        false
    }
}

/// 用户 session：每个用户独立 agent 会话。
#[derive(Debug)]
struct UserSession {
    #[allow(dead_code)]
    user_id: String,
    last_active: Instant,
    message_count: u64,
}

/// LRU 用户 session 缓存。
pub struct AgentSessionCache {
    max_sessions: usize,
    session_ttl: Duration,
    /// 按访问时间排序的队列（最旧在前）。
    order: parking_lot::Mutex<VecDeque<String>>,
    sessions: DashMap<String, UserSession>,
}

impl AgentSessionCache {
    pub fn new(max_sessions: usize, session_ttl: Duration) -> Self {
        Self {
            max_sessions,
            session_ttl,
            order: parking_lot::Mutex::new(VecDeque::new()),
            sessions: DashMap::new(),
        }
    }

    /// 获取或创建用户 session。
    pub fn touch(&self, user_id: &str) -> bool {
        if self.sessions.contains_key(user_id) {
            if let Some(mut s) = self.sessions.get_mut(user_id) {
                s.last_active = Instant::now();
                s.message_count += 1;
            }
            return false; // 已存在
        }
        self.evict_if_needed();
        self.sessions.insert(
            user_id.to_string(),
            UserSession {
                user_id: user_id.to_string(),
                last_active: Instant::now(),
                message_count: 1,
            },
        );
        self.order.lock().push_back(user_id.to_string());
        true
    }

    /// 驱逐过期 session。
    pub fn evict_expired(&self) {
        let now = Instant::now();
        let mut order = self.order.lock();
        order.retain(|uid| {
            let keep = self
                .sessions
                .get(uid)
                .map(|s| now.duration_since(s.last_active) < self.session_ttl)
                .unwrap_or(false);
            if !keep {
                self.sessions.remove(uid);
            }
            keep
        });
    }

    fn evict_if_needed(&self) {
        while self.sessions.len() >= self.max_sessions {
            if let Some(oldest) = self.order.lock().pop_front() {
                self.sessions.remove(&oldest);
            } else {
                break;
            }
        }
    }

    /// Session 数量。
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

pub mod adapters;
