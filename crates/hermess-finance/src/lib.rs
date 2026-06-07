// crates/hermess-finance/src/lib.rs
// Pluggable financial data layer for Hermess agent.

pub mod providers;
pub mod tool;
pub mod types;

use async_trait::async_trait;

// ── Core trait ──────────────────────────────────────────────────

/// A swappable financial data backend (FTShare, Wind, etc.).
#[async_trait]
pub trait FinancialDataProvider: Send + Sync {
    /// Returns the capabilities this provider supports, used to generate the LLM tool schema.
    fn capabilities(&self) -> Vec<QueryCapability>;

    /// Unified query entry point. Each variant dispatches to a specific data endpoint.
    async fn query(&self, q: FinancialQuery) -> anyhow::Result<FinancialResult>;

    /// Human-readable provider name for logging.
    fn provider_name(&self) -> &str;
}

// ── Query capability (for LLM schema generation) ────────────────

/// Describes one kind of financial query the provider can handle.
pub struct QueryCapability {
    /// Short name, e.g. "stock_quote", "index_detail"
    pub name: String,
    /// Human-readable description for the LLM tool schema
    pub description: String,
    /// JSON Schema object describing the parameters
    pub parameters: serde_json::Value,
}

// ── Query enum — one variant per data domain ───────────────────

/// All supported financial queries. Providers match on the variants they handle.
#[derive(Debug, Clone)]
pub enum FinancialQuery {
    // ── A-Share stocks ──────────────────────────────
    /// Full stock list (沪深京)
    StockList,
    /// Real-time quotes for one or more symbols
    StockQuote {
        symbols: Vec<String>,
        order_by: String,
    },
    /// IPO calendar
    StockIpos { page: u32, page_size: u32 },
    /// Block trades (大宗交易)
    BlockTrades,
    /// Margin trading & short selling (融资融券)
    MarginTrading { page: u32, page_size: u32 },
    /// Stock security info (估值指标, PE/PB/PS etc.)
    StockSecurityInfo { symbol: String },
    /// Stock daily candlestick (OHLCV) data
    StockHistory {
        symbol: String,
        period: String,
        limit: u32,
    },

    // ── Indices ─────────────────────────────────────
    /// List available indices
    IndexList { filter: Option<String> },
    /// Index detail / constituents
    IndexDetail { symbol: String },
    /// Index OHLCV candlestick
    IndexOhlc {
        symbol: String,
        span: String,
        limit: u32,
    },

    // ── Funds ───────────────────────────────────────
    /// Fund basic info
    FundInfo { code: String },
    /// Fund NAV history
    FundNav { code: String, page: u32 },
    /// ETF info
    EtfInfo { code: String },

    // ── HK Stocks ───────────────────────────────────
    /// HK stock quote snapshot
    HkStockQuote { symbols: Vec<String> },
    /// HK stock company overview
    HkCompany { symbol: String },
    /// HK stock candlestick
    HkCandlestick {
        symbol: String,
        period: String,
        limit: u32,
    },

    // ── Macro ───────────────────────────────────────
    /// China macroeconomic indicators
    MacroChina { indicator: String },
    /// US macroeconomic indicators
    MacroUs { indicator_type: String },
}

// ── Query result ────────────────────────────────────────────────

/// Either structured data or an error message.
#[derive(Debug, Clone)]
pub struct FinancialResult {
    pub success: bool,
    /// Human-readable summary (for LLM consumption)
    pub summary: String,
    /// Structured data (for programmatic use)
    pub data: Option<serde_json::Value>,
    /// Error message on failure
    pub error: Option<String>,
}

impl FinancialResult {
    pub fn ok(summary: String, data: Option<serde_json::Value>) -> Self {
        Self {
            success: true,
            summary,
            data,
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            summary: String::new(),
            data: None,
            error: Some(msg.into()),
        }
    }
}

impl std::fmt::Display for FinancialResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.success {
            write!(f, "{}", self.summary)
        } else {
            write!(
                f,
                "查询失败: {}",
                self.error.as_deref().unwrap_or("unknown")
            )
        }
    }
}
