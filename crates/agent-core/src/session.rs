// crates/agent-core/src/session.rs
// 会话保存与恢复：将 agent 的完整对话状态序列化为 JSON。

use crate::MemoryChunk;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 会话快照：包含 agent 在某一时刻的完整运行状态。
/// 用于 `--save` / `--resume` 功能。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// 架构版本号（用于向前兼容）
    pub version: u32,
    /// 会话保存时间
    pub saved_at: DateTime<Utc>,
    /// 当前轮次
    pub turn: u64,
    /// 对话历史：(用户输入, 执行摘要)
    pub conversation_history: Vec<(String, String)>,
    /// 工作记忆快照
    pub working_memory_chunks: Vec<MemoryChunk>,
}

impl SessionState {
    const CURRENT_VERSION: u32 = 1;

    /// 创建一个新的会话快照。
    pub fn new(
        turn: u64,
        conversation_history: Vec<(String, String)>,
        working_memory_chunks: Vec<MemoryChunk>,
    ) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            saved_at: Utc::now(),
            turn,
            conversation_history,
            working_memory_chunks,
        }
    }

    /// 保存到 JSON 文件。
    pub fn save_to_file(&self, path: &str) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        tracing::info!(path = %path, turn = %self.turn, "会话已保存");
        Ok(())
    }

    /// 从 JSON 文件加载。
    pub fn load_from_file(path: &str) -> anyhow::Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let state: Self = serde_json::from_str(&data)?;
        if state.version > Self::CURRENT_VERSION {
            tracing::warn!(
                version = state.version,
                supported = Self::CURRENT_VERSION,
                "会话文件版本较新，可能不兼容"
            );
        }
        tracing::info!(path = %path, turn = %state.turn, "会话已加载");
        Ok(state)
    }
}
