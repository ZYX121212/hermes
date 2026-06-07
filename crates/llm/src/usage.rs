// crates/llm/src/usage.rs
// Token 使用量追踪和费用估算。

use parking_lot::Mutex;

/// 单次 LLM 调用的 token 用量。
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl TokenUsage {
    pub fn new(prompt: u64, completion: u64, total: u64) -> Self {
        Self {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
        }
    }
}

/// 会话累计用量追踪器（线程安全）。
pub struct UsageTracker {
    pub prompt_tokens: Mutex<u64>,
    pub completion_tokens: Mutex<u64>,
    pub total_calls: Mutex<u64>,
    pub model: String,
}

impl UsageTracker {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            prompt_tokens: Mutex::new(0),
            completion_tokens: Mutex::new(0),
            total_calls: Mutex::new(0),
            model: model.into(),
        }
    }

    /// 记录一次 LLM 调用。
    pub fn record(&self, usage: &TokenUsage) {
        *self.prompt_tokens.lock() += usage.prompt_tokens;
        *self.completion_tokens.lock() += usage.completion_tokens;
        *self.total_calls.lock() += 1;
    }

    /// 估算累计费用（美元）。
    pub fn estimated_cost_usd(&self) -> f64 {
        let p = *self.prompt_tokens.lock();
        let c = *self.completion_tokens.lock();
        let pricing = match_model(&self.model);
        (p as f64 / 1_000_000.0) * pricing.input_price
            + (c as f64 / 1_000_000.0) * pricing.output_price
    }

    /// 返回当前累计：调用次数、输入 tokens、输出 tokens、费用
    pub fn snapshot(&self) -> UsageSnapshot {
        let prompt = *self.prompt_tokens.lock();
        let completion = *self.completion_tokens.lock();
        let calls = *self.total_calls.lock();
        let pricing = match_model(&self.model);
        let cost = (prompt as f64 / 1_000_000.0) * pricing.input_price
            + (completion as f64 / 1_000_000.0) * pricing.output_price;
        UsageSnapshot {
            total_calls: calls,
            prompt_tokens: prompt,
            completion_tokens: completion,
            estimated_cost_usd: cost,
            model: self.model.clone(),
        }
    }
}

/// 某时刻的用量快照
#[derive(Debug, Clone)]
pub struct UsageSnapshot {
    pub total_calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub estimated_cost_usd: f64,
    pub model: String,
}

impl std::fmt::Display for UsageSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}次调用 | 入:{}K 出:{}K | ${:.4}",
            self.total_calls,
            self.prompt_tokens / 1000,
            self.completion_tokens / 1000,
            self.estimated_cost_usd
        )
    }
}

// ── 定价表（每百万 token 美元价格） ──

struct Pricing {
    input_price: f64,
    output_price: f64,
}

fn match_model(model: &str) -> Pricing {
    let m = model.to_lowercase();
    if m.contains("deepseek") {
        Pricing {
            input_price: 0.27,
            output_price: 1.10,
        }
    } else if m.contains("claude-opus") {
        Pricing {
            input_price: 15.0,
            output_price: 75.0,
        }
    } else if m.contains("claude-sonnet") {
        Pricing {
            input_price: 3.0,
            output_price: 15.0,
        }
    } else if m.contains("claude-haiku") {
        Pricing {
            input_price: 0.80,
            output_price: 4.0,
        }
    } else if m.contains("gpt-4o") {
        Pricing {
            input_price: 2.50,
            output_price: 10.0,
        }
    } else if m.contains("gpt-4") {
        Pricing {
            input_price: 30.0,
            output_price: 60.0,
        }
    } else if m.contains("gpt-3.5") {
        Pricing {
            input_price: 0.50,
            output_price: 1.50,
        }
    } else {
        // 未知模型默认
        Pricing {
            input_price: 1.0,
            output_price: 5.0,
        }
    }
}
