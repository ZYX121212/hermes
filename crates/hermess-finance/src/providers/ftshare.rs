// FtShareProvider — financial data via market.ft.tech API

use async_trait::async_trait;
use std::time::Duration;

use super::retry::{self, RetryConfig};
use crate::{FinancialDataProvider, FinancialQuery, FinancialResult, QueryCapability};

pub struct FtShareProvider {
    client: reqwest::Client,
    base_url: String,
}

impl FtShareProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .user_agent("hermess-finance/0.1")
            .build()
            .expect("Failed to build reqwest client for FtShare");
        Self {
            client,
            base_url: base_url.into(),
        }
    }

    async fn get(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}{}", self.base_url, path);
        let retry_cfg = RetryConfig::default();
        retry::with_retry(&retry_cfg, "ftshare", || {
            let url = url.clone();
            let client = self.client.clone();
            async move {
                let resp = client.get(&url).send().await?;
                let status = resp.status();
                let body_text = resp.text().await?;

                if !status.is_success() {
                    tracing::warn!(%url, %status, body = %body_text, "FtShare HTTP error");
                    return Err(anyhow::anyhow!("FTShare HTTP {status}: {body_text}"));
                }

                let body: serde_json::Value = serde_json::from_str(&body_text)
                    .map_err(|e| {
                        tracing::warn!(%url, body = %body_text, error = %e, "FtShare JSON parse failed");
                        anyhow::anyhow!("FTShare JSON parse error: {e}")
                    })?;

                tracing::debug!(%url, "FtShare response ok");
                Ok(body)
            }
        })
        .await
    }

    /// Format a JSON array into a readable table string (max 15 rows).
    fn format_table(rows: &[serde_json::Value], columns: &[&str]) -> String {
        if rows.is_empty() {
            return "无数据".to_string();
        }
        let limit = rows.len().min(15);
        let mut out = String::new();
        out.push_str(&columns.join("\t"));
        out.push('\n');
        out.push_str(&"-".repeat(columns.len() * 12));
        out.push('\n');
        for row in rows.iter().take(limit) {
            let cells: Vec<String> = columns
                .iter()
                .map(|c| {
                    row.get(c)
                        .map(|v| match v {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default()
                })
                .collect();
            out.push_str(&cells.join("\t"));
            out.push('\n');
        }
        if rows.len() > limit {
            out.push_str(&format!("... 共 {} 条，仅显示前 {limit} 条\n", rows.len()));
        }
        out
    }
}

#[async_trait]
impl FinancialDataProvider for FtShareProvider {
    fn capabilities(&self) -> Vec<QueryCapability> {
        vec![
            QueryCapability {
                name: "stock_list".into(),
                description: "查询全量股票列表（沪深京）".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            QueryCapability {
                name: "stock_quote".into(),
                description: "查询股票实时行情，支持多只股票同时查询".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbols": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "股票代码列表，如 [\"600519\", \"000001\"]"
                        },
                        "order_by": {
                            "type": "string",
                            "description": "排序字段，如 pct_chg（涨跌幅）, volume（成交量）, amount（成交额）",
                            "default": "pct_chg"
                        }
                    },
                    "required": ["symbols"]
                }),
            },
            QueryCapability {
                name: "index_list".into(),
                description: "查询可用指数列表".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "filter": {
                            "type": "string",
                            "description": "可选过滤关键词，如 \"沪深300\", \"科创\""
                        }
                    }
                }),
            },
            QueryCapability {
                name: "index_detail".into(),
                description: "查询指数详情和成分股".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "指数代码，如 sh000001（上证综指）"
                        }
                    },
                    "required": ["symbol"]
                }),
            },
            QueryCapability {
                name: "index_ohlc".into(),
                description: "查询指数日K线数据".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbol": {"type": "string", "description": "指数代码"},
                        "span": {"type": "string", "description": "时间跨度，如 1m=一个月, 1y=一年", "default": "1m"},
                        "limit": {"type": "integer", "description": "返回条数上限", "default": 30}
                    },
                    "required": ["symbol"]
                }),
            },
            QueryCapability {
                name: "fund_info".into(),
                description: "查询基金基本信息".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "code": {"type": "string", "description": "基金代码，如 000001"}
                    },
                    "required": ["code"]
                }),
            },
            QueryCapability {
                name: "macro_china".into(),
                description: "查询中国宏观经济指标（CPI, PPI, GDP, PMI 等）".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "indicator": {
                            "type": "string",
                            "description": "指标名称：cpi, ppi, gdp, pmi, money_supply, trade"
                        }
                    },
                    "required": ["indicator"]
                }),
            },
            QueryCapability {
                name: "hk_stock_quote".into(),
                description: "查询港股实时行情".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbols": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "港股代码列表，如 [\"00700\", \"09988\"]"
                        }
                    },
                    "required": ["symbols"]
                }),
            },
        ]
    }

    async fn query(&self, q: FinancialQuery) -> anyhow::Result<FinancialResult> {
        match q {
            FinancialQuery::StockList => {
                let body = self.get("/api/v1/stock/list").await?;
                let data = body["data"].as_array().cloned();
                let summary = data
                    .as_ref()
                    .map(|rows| Self::format_table(rows, &["symbol", "name", "industry", "market"]))
                    .unwrap_or_else(|| "查询股票列表失败".to_string());
                Ok(FinancialResult::ok(
                    summary,
                    data.map(serde_json::Value::Array),
                ))
            }

            FinancialQuery::StockQuote { symbols, order_by } => {
                let order = if order_by.is_empty() {
                    "pct_chg".to_string()
                } else {
                    order_by
                };
                let encoded_symbols: String = symbols
                    .iter()
                    .map(|s| urlencoding_maybe(s))
                    .collect::<Vec<_>>()
                    .join(",");
                let path = format!(
                    "/api/v1/stock/quote?symbols={}&order_by={}",
                    encoded_symbols, order
                );
                let body = self.get(&path).await?;
                let data = body["data"].as_array().cloned();
                let summary = data
                    .as_ref()
                    .map(|rows| {
                        Self::format_table(
                            rows,
                            &[
                                "symbol", "name", "close", "pct_chg", "volume", "amount", "high",
                                "low",
                            ],
                        )
                    })
                    .unwrap_or_else(|| "查询行情失败".to_string());
                Ok(FinancialResult::ok(
                    summary,
                    data.map(serde_json::Value::Array),
                ))
            }

            FinancialQuery::IndexList { filter } => {
                let mut path = "/api/v1/index/list".to_string();
                if let Some(ref f) = filter {
                    path.push_str(&format!("?filter={f}"));
                }
                let body = self.get(&path).await?;
                let data = body["data"].as_array().cloned();
                let summary = data
                    .as_ref()
                    .map(|rows| Self::format_table(rows, &["symbol", "name", "source"]))
                    .unwrap_or_else(|| "查询指数列表失败".to_string());
                Ok(FinancialResult::ok(
                    summary,
                    data.map(serde_json::Value::Array),
                ))
            }

            FinancialQuery::IndexDetail { symbol } => {
                let body = self
                    .get(&format!("/api/v1/index/detail?symbol={symbol}"))
                    .await?;
                let data = body.get("data").cloned();
                let summary = data
                    .as_ref()
                    .map(|d| {
                        let name = d["name"].as_str().unwrap_or("?");
                        let close = d["close"].as_f64().map(|v| v.to_string()).unwrap_or_default();
                        let pct = d["pct_chg"].as_f64().map(|v| format!("{:.2}%", v)).unwrap_or_default();
                        let constituents = d["constituents"].as_array().map(|c| c.len()).unwrap_or(0);
                        format!("指数: {name}\n收盘: {close}\n涨跌幅: {pct}\n成分股数量: {constituents}")
                    })
                    .unwrap_or_else(|| "查询指数详情失败".to_string());
                Ok(FinancialResult::ok(summary, data))
            }

            FinancialQuery::IndexOhlc {
                symbol,
                span,
                limit,
            } => {
                let span = if span.is_empty() {
                    "1m".to_string()
                } else {
                    span
                };
                let body = self
                    .get(&format!(
                        "/api/v1/index/ohlc?symbol={symbol}&span={span}&limit={limit}"
                    ))
                    .await?;
                let data = body["data"].as_array().cloned();
                let summary = data
                    .as_ref()
                    .map(|rows| {
                        Self::format_table(
                            rows,
                            &["date", "open", "close", "high", "low", "volume"],
                        )
                    })
                    .unwrap_or_else(|| "查询K线失败".to_string());
                Ok(FinancialResult::ok(
                    summary,
                    data.map(serde_json::Value::Array),
                ))
            }

            FinancialQuery::FundInfo { code } => {
                let body = self.get(&format!("/api/v1/fund/info?code={code}")).await?;
                let data = body.get("data").cloned();
                let summary = data
                    .as_ref()
                    .map(|d| {
                        let name = d["name"].as_str().unwrap_or("?");
                        let nav = d["nav"].as_f64().map(|v| v.to_string()).unwrap_or_default();
                        format!("基金: {name}\n最新净值: {nav}")
                    })
                    .unwrap_or_else(|| "查询基金信息失败".to_string());
                Ok(FinancialResult::ok(summary, data))
            }

            FinancialQuery::FundNav { code, page } => {
                let body = self
                    .get(&format!("/api/v1/fund/nav?code={code}&page={page}"))
                    .await?;
                let data = body["data"].as_array().cloned();
                let summary = data
                    .as_ref()
                    .map(|rows| Self::format_table(rows, &["date", "nav", "acc_nav", "pct_chg"]))
                    .unwrap_or_else(|| "查询净值失败".to_string());
                Ok(FinancialResult::ok(
                    summary,
                    data.map(serde_json::Value::Array),
                ))
            }

            FinancialQuery::MacroChina { indicator } => {
                let body = self
                    .get(&format!("/api/v1/macro/china?indicator={indicator}"))
                    .await?;
                let data = body["data"].as_array().cloned();
                let summary = data
                    .as_ref()
                    .map(|rows| Self::format_table(rows, &["date", "value", "unit"]))
                    .unwrap_or_else(|| "查询宏观指标失败".to_string());
                Ok(FinancialResult::ok(
                    summary,
                    data.map(serde_json::Value::Array),
                ))
            }

            FinancialQuery::HkStockQuote { symbols } => {
                let path = format!("/api/v1/hk/stock/quote?symbols={}", symbols.join(","));
                let body = self.get(&path).await?;
                let data = body["data"].as_array().cloned();
                let summary = data
                    .as_ref()
                    .map(|rows| {
                        Self::format_table(
                            rows,
                            &["symbol", "name", "close", "pct_chg", "volume", "amount"],
                        )
                    })
                    .unwrap_or_else(|| "查询港股行情失败".to_string());
                Ok(FinancialResult::ok(
                    summary,
                    data.map(serde_json::Value::Array),
                ))
            }

            FinancialQuery::StockIpos { .. }
            | FinancialQuery::BlockTrades
            | FinancialQuery::MarginTrading { .. }
            | FinancialQuery::StockSecurityInfo { .. }
            | FinancialQuery::StockHistory { .. }
            | FinancialQuery::EtfInfo { .. }
            | FinancialQuery::HkCompany { .. }
            | FinancialQuery::HkCandlestick { .. }
            | FinancialQuery::MacroUs { .. } => Ok(FinancialResult::err(
                "FTShare 暂不支持此查询类型，请等待后续更新".to_string(),
            )),
        }
    }

    fn provider_name(&self) -> &str {
        "ftshare"
    }
}

/// Minimal URL-encoding for symbol characters that break query strings.
fn urlencoding_maybe(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        s.to_string()
    } else {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                    c.to_string()
                } else {
                    format!("%{:02X}", c as u8)
                }
            })
            .collect()
    }
}
