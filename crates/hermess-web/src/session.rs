// 用户会话管理
// 每个微信用户对应一个独立的 Agent 会话

use agent_core::AgentEvent;
use dashmap::DashMap;
use hermess_agent::SmallHermesAgent;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};

/// 单个用户会话
pub struct UserSession {
    pub agent: Arc<Mutex<SmallHermesAgent>>,
    last_active: parking_lot::Mutex<Instant>,
}

/// 会话管理器：创建/查找/清理用户会话
pub struct SessionManager {
    sessions: DashMap<String, UserSession>,
    /// 共享的进化引擎
    evolution: Arc<evolution::EvolutionEngine>,
    /// 共享的 LLM 适配器
    llm: Arc<dyn llm::LlmAdapter>,
    /// 共享的工具注册表
    tools: Arc<tools::ToolRegistry>,
    /// 最大并发子任务数
    max_concurrency: usize,
    /// 工作记忆容量
    working_memory_size: usize,
    /// 会话超时秒数
    session_timeout: u64,
}

impl SessionManager {
    pub fn new(
        evolution: Arc<evolution::EvolutionEngine>,
        llm: Arc<dyn llm::LlmAdapter>,
        tools: Arc<tools::ToolRegistry>,
        max_concurrency: usize,
        working_memory_size: usize,
    ) -> Self {
        Self {
            sessions: DashMap::new(),
            evolution,
            llm,
            tools,
            max_concurrency,
            working_memory_size,
            session_timeout: 1800,
        }
    }

    /// 启动后台清理任务
    pub fn start_cleanup(self: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                self.cleanup_expired();
            }
        });
    }

    /// 获取或创建用户会话。
    /// 返回 (agent_arc_mutex, event_rx)。
    /// 调用方 lock agent_arc_mutex 即可串行处理该用户的消息。
    pub fn get_or_create(
        &self,
        user_id: &str,
    ) -> (Arc<Mutex<SmallHermesAgent>>, mpsc::UnboundedReceiver<AgentEvent>) {
        if let Some(entry) = self.sessions.get(user_id) {
            *entry.last_active.lock() = Instant::now();
            // 给已有 agent 换一个新的 event channel
            let (tx, rx) = mpsc::unbounded_channel();
            // 注意：这里在 sync 上下文中尝试 lock async Mutex。
            // DashMap::get 是同步的，但我们可以在 tokio 运行时中处理。
            // 用一个 channel 来延迟设置 event_tx。
            let agent = Arc::clone(&entry.agent);
            let tx_clone = tx;
            tokio::spawn(async move {
                let mut guard = agent.lock().await;
                guard.event_tx = Some(tx_clone);
            });
            return (Arc::clone(&entry.agent), rx);
        }

        // 创建新 agent
        let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEvent>();

        let mut planner = planner::Planner::new(
            Arc::clone(&self.llm) as Arc<dyn llm::LlmAdapter>,
            Arc::clone(&self.evolution),
        )
        .with_streaming(true);
        planner.set_tools(self.tools.describe_all());

        let mut scheduler =
            scheduler::Scheduler::new(Arc::clone(&self.tools), self.max_concurrency);
        scheduler.set_event_sender(event_tx.clone());

        let reflector = reflector::Reflector::new(
            Arc::clone(&self.llm) as Arc<dyn llm::LlmAdapter>,
        );

        let agent = SmallHermesAgent {
            planner,
            scheduler,
            reflector,
            evolution: Arc::clone(&self.evolution),
            working_memory: memory::WorkingMemory::new(self.working_memory_size),
            llm: Arc::clone(&self.llm) as Arc<dyn llm::LlmAdapter>,
            turn: 0,
            usage_tracker: Arc::new(llm::UsageTracker::new("unknown")),
            event_tx: Some(event_tx),
            max_replans: 3,
            compress_threshold: 20,
            compress_keep_ratio: 0.5,
            conversation_history: Vec::new(),
            tui_input: None,
        };

        let agent_arc = Arc::new(Mutex::new(agent));
        let session = UserSession {
            agent: Arc::clone(&agent_arc),
            last_active: parking_lot::Mutex::new(Instant::now()),
        };

        self.sessions.insert(user_id.to_string(), session);
        (agent_arc, event_rx)
    }

    fn cleanup_expired(&self) {
        let timeout = std::time::Duration::from_secs(self.session_timeout);
        let mut removed = 0;
        self.sessions.retain(|user_id, session| {
            let active = *session.last_active.lock();
            if active.elapsed() > timeout {
                tracing::info!(%user_id, "session expired, removed");
                removed += 1;
                false
            } else {
                true
            }
        });
        if removed > 0 {
            tracing::info!(removed, "expired sessions cleaned up");
        }
    }
}
