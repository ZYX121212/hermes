// TuShareProvider — financial data via tushare.pro API

use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

use super::retry::{self, RetryConfig};
use crate::{FinancialDataProvider, FinancialQuery, FinancialResult, QueryCapability};

pub struct TuShareProvider {
    client: reqwest::Client,
    token: String,
}

impl TuShareProvider {
    pub fn new(token: impl Into<String>) -> Self {
        let token = token.into();
        if token.is_empty() {
            tracing::warn!("TuShare token is empty — all API calls will be rejected");
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .user_agent("hermess-finance/0.1")
            .build()
            .expect("Failed to build reqwest client for TuShare");
        Self { client, token }
    }

    async fn call(&self, api_name: &str, params: Value, fields: &str) -> anyhow::Result<Value> {
        let body = serde_json::json!({
            "api_name": api_name,
            "token": self.token,
            "params": params,
            "fields": fields,
        });

        // Use retry with backoff. TuShare free tier has 1 req/s rate limit;
        // exponential backoff handles transient 429 / connection errors.
        let retry_cfg = RetryConfig {
            max_retries: 3,
            base_delay_ms: 1_000, // Start with 1s to respect rate limits
            max_delay_ms: 8_000,
        };

        let client = self.client.clone();
        let body_clone = body.clone();
        retry::with_retry(&retry_cfg, "tushare", || {
            let client = client.clone();
            let body = body_clone.clone();
            async move {
                let resp = client
                    .post("https://api.tushare.pro")
                    .json(&body)
                    .send()
                    .await?;

                let status = resp.status();
                let body_text = resp.text().await?;

                if !status.is_success() {
                    tracing::warn!(%status, body = %body_text, "TuShare HTTP error");
                    return Err(anyhow::anyhow!("TuShare HTTP {status}: {body_text}"));
                }

                let json: Value = serde_json::from_str(&body_text).map_err(|e| {
                    tracing::warn!(body = %body_text, error = %e, "TuShare JSON parse failed");
                    anyhow::anyhow!("TuShare JSON parse error: {e}")
                })?;

                let code = json["code"].as_i64().unwrap_or(-1);
                if code != 0 {
                    let msg = json["msg"].as_str().unwrap_or("unknown error");
                    // Rate-limit errors are transient — let retry handle them
                    if msg.contains("频率") || msg.contains("rate") || msg.contains("limit") {
                        return Err(anyhow::anyhow!("TuShare rate limited (code={code}): {msg}"));
                    }
                    return Err(anyhow::anyhow!("TuShare API error (code={code}): {msg}"));
                }

                tracing::debug!(api = %api_name, "TuShare response ok");
                Ok(json)
            }
        })
        .await
    }

    /// Format Tushare response into a readable table.
    fn format_response(json: &Value, max_rows: usize) -> String {
        let data = match json.get("data") {
            Some(d) => d,
            None => return "无数据".to_string(),
        };

        let fields: Vec<&str> = data["fields"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let items = match data["items"].as_array() {
            Some(items) => items,
            None => return "无数据".to_string(),
        };

        if items.is_empty() || fields.is_empty() {
            return "无数据".to_string();
        }

        let limit = items.len().min(max_rows);
        let mut out = String::new();

        out.push_str(&fields.join("\t"));
        out.push('\n');
        out.push_str(&"-".repeat(fields.len() * 12));
        out.push('\n');

        for item in items.iter().take(limit) {
            let cells: Vec<String> = item
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            Value::Number(n) => n.to_string(),
                            Value::Null => "-".to_string(),
                            other => other.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.push_str(&cells.join("\t"));
            out.push('\n');
        }

        if items.len() > limit {
            out.push_str(&format!("... 共 {} 条，仅显示前 {limit} 条\n", items.len()));
        }

        out
    }
}

#[async_trait]
impl FinancialDataProvider for TuShareProvider {
    fn capabilities(&self) -> Vec<QueryCapability> {
        vec![
            QueryCapability {
                name: "stock_list".into(),
                description: "查询全量股票列表（沪深京），支持按市场/上市状态过滤".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "market": {
                            "type": "string",
                            "description": "市场过滤: SSE(上交所), SZSE(深交所), BSE(北交所)"
                        }
                    }
                }),
            },
            QueryCapability {
                name: "stock_quote".into(),
                description: "查询股票日线行情（OHLCV），支持日期范围和代码列表".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbols": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "股票代码列表，如 [\"000001.SZ\", \"600519.SH\"]"
                        },
                        "start_date": {"type": "string", "description": "起始日期 YYYYMMDD"},
                        "end_date": {"type": "string", "description": "结束日期 YYYYMMDD"}
                    },
                    "required": ["symbols"]
                }),
            },
            QueryCapability {
                name: "index_list".into(),
                description: "查询指数列表（上证指数、深证成指、沪深300等）".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "market": {
                            "type": "string",
                            "description": "市场代码: SSE, SZSE, CSI, 留空查全部"
                        }
                    }
                }),
            },
            QueryCapability {
                name: "index_detail".into(),
                description: "查询指数日线行情".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbol": {"type": "string", "description": "指数代码，如 000001.SH（上证综指）"},
                        "start_date": {"type": "string"},
                        "end_date": {"type": "string"}
                    },
                    "required": ["symbol"]
                }),
            },
            QueryCapability {
                name: "fund_info".into(),
                description: "查询基金基本信息（名称、类型、管理人）".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "code": {"type": "string", "description": "基金代码"}
                    },
                    "required": ["code"]
                }),
            },
            QueryCapability {
                name: "fund_nav".into(),
                description: "查询基金净值历史".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "code": {"type": "string", "description": "基金代码"},
                        "start_date": {"type": "string"},
                        "end_date": {"type": "string"}
                    },
                    "required": ["code"]
                }),
            },
            QueryCapability {
                name: "macro_china".into(),
                description: "查询中国宏观经济指标（CPI, PPI, GDP, PMI, M2等）".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "indicator": {
                            "type": "string",
                            "description": "指标: cpi, ppi, gdp, pmi, money_supply, trade"
                        }
                    },
                    "required": ["indicator"]
                }),
            },
            QueryCapability {
                name: "trade_cal".into(),
                description: "查询交易日历".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "start_date": {"type": "string", "description": "起始日期 YYYYMMDD"},
                        "end_date": {"type": "string", "description": "结束日期 YYYYMMDD"}
                    }
                }),
            },
        ]
    }

    async fn query(&self, q: FinancialQuery) -> anyhow::Result<FinancialResult> {
        match q {
            FinancialQuery::StockList => {
                let json = self
                    .call(
                        "stock_basic",
                        serde_json::json!({
                            "exchange": "",
                            "list_status": "L",
                            "fields": "ts_code,symbol,name,area,industry,market,list_date"
                        }),
                        "",
                    )
                    .await?;
                let summary = Self::format_response(&json, 15);
                let data = json.get("data").cloned();
                Ok(FinancialResult::ok(summary, data))
            }

            FinancialQuery::StockQuote {
                symbols,
                order_by: _,
            } => {
                let ts_codes: Vec<String> = symbols
                    .iter()
                    .map(|s| {
                        if s.contains('.') {
                            s.clone()
                        } else if s.starts_with('6') {
                            format!("{s}.SH")
                        } else {
                            format!("{s}.SZ")
                        }
                    })
                    .collect();

                // Use yesterday as fallback — today's daily bar may not exist
                // during trading hours (TuShare updates after market close).
                let today = chrono::Utc::now().format("%Y%m%d").to_string();
                let yesterday = (chrono::Utc::now() - chrono::Duration::days(1))
                    .format("%Y%m%d")
                    .to_string();
                let json = self
                    .call(
                        "daily",
                        serde_json::json!({
                            "ts_code": ts_codes.join(","),
                            "start_date": &yesterday,
                            "end_date": &today,
                        }),
                        "ts_code,trade_date,open,high,low,close,vol,amount,pct_chg",
                    )
                    .await?;
                let summary = Self::format_response(&json, 15);
                let data = json.get("data").cloned();
                Ok(FinancialResult::ok(summary, data))
            }

            FinancialQuery::StockHistory {
                symbol,
                period: _,
                limit,
            } => {
                let ts_code = if symbol.contains('.') {
                    symbol.clone()
                } else if symbol.starts_with('6') {
                    format!("{symbol}.SH")
                } else {
                    format!("{symbol}.SZ")
                };
                let json = self
                    .call(
                        "daily",
                        serde_json::json!({
                            "ts_code": ts_code,
                            "limit": limit,
                        }),
                        "ts_code,trade_date,open,high,low,close,vol,amount,pct_chg",
                    )
                    .await?;
                let summary = Self::format_response(&json, limit as usize);
                let data = json.get("data").cloned();
                Ok(FinancialResult::ok(summary, data))
            }

            FinancialQuery::IndexList { filter } => {
                let market = filter.unwrap_or_default();
                let json = self
                    .call(
                        "index_basic",
                        serde_json::json!({"market": market}),
                        "ts_code,name,market,publisher,category,base_date,base_point",
                    )
                    .await?;
                let summary = Self::format_response(&json, 15);
                let data = json.get("data").cloned();
                Ok(FinancialResult::ok(summary, data))
            }

            FinancialQuery::IndexDetail { symbol } => {
                let json = self
                    .call(
                        "index_daily",
                        serde_json::json!({
                            "ts_code": symbol,
                            "limit": 1,
                        }),
                        "ts_code,trade_date,open,high,low,close,vol,amount,pct_chg",
                    )
                    .await?;
                let data = json.get("data").cloned();
                let summary = data
                    .as_ref()
                    .and_then(|d| {
                        let items = d["items"].as_array()?;
                        let fields = d["fields"].as_array()?;
                        items.first().map(|row| {
                            let arr = row.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                            let mut out = String::from("指数行情:\n");
                            for (i, field) in fields.iter().enumerate() {
                                let val: String = arr
                                    .get(i)
                                    .map(|v| match v {
                                        Value::String(s) => s.clone(),
                                        Value::Number(n) => n.to_string(),
                                        _ => "-".to_string(),
                                    })
                                    .unwrap_or_else(|| "-".to_string());
                                out.push_str(&format!(
                                    "  {}: {}\n",
                                    field.as_str().unwrap_or("?"),
                                    val
                                ));
                            }
                            out
                        })
                    })
                    .unwrap_or_else(|| "查询指数详情失败".to_string());
                Ok(FinancialResult::ok(summary, data))
            }

            FinancialQuery::IndexOhlc {
                symbol,
                span: _,
                limit,
            } => {
                let json = self
                    .call(
                        "index_daily",
                        serde_json::json!({
                            "ts_code": symbol,
                            "limit": limit,
                        }),
                        "ts_code,trade_date,open,high,low,close,vol,amount,pct_chg",
                    )
                    .await?;
                let summary = Self::format_response(&json, limit as usize);
                let data = json.get("data").cloned();
                Ok(FinancialResult::ok(summary, data))
            }

            FinancialQuery::FundInfo { code } => {
                let json = self
                    .call(
                        "fund_basic",
                        serde_json::json!({"ts_code": code}),
                        "ts_code,name,management,found_date,type,invest_type,a_share",
                    )
                    .await?;
                let data = json.get("data").cloned();
                let summary = data
                    .as_ref()
                    .and_then(|d| {
                        let items = d["items"].as_array()?;
                        items.first().map(|row| {
                            let arr = row.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                            format!(
                                "基金: {} | 管理人: {} | 类型: {} | 成立: {}",
                                arr.get(1).and_then(|v| v.as_str()).unwrap_or("?"),
                                arr.get(2).and_then(|v| v.as_str()).unwrap_or("?"),
                                arr.get(4).and_then(|v| v.as_str()).unwrap_or("?"),
                                arr.get(3).and_then(|v| v.as_str()).unwrap_or("?"),
                            )
                        })
                    })
                    .unwrap_or_else(|| "查询基金信息失败".to_string());
                Ok(FinancialResult::ok(summary, data))
            }

            FinancialQuery::FundNav { code, page: _ } => {
                let json = self
                    .call(
                        "fund_nav",
                        serde_json::json!({"ts_code": code, "limit": 30}),
                        "ts_code,end_date,unit_nav,accum_nav,adj_nav,pct_chg",
                    )
                    .await?;
                let summary = Self::format_response(&json, 15);
                let data = json.get("data").cloned();
                Ok(FinancialResult::ok(summary, data))
            }

            FinancialQuery::MacroChina { indicator } => {
                let (api_name, fields) = match indicator.as_str() {
                    "cpi" => ("cpi", "month,cpi_monthly,ntn_val,cpi_yearly"),
                    "ppi" => ("ppi", "month,ppi_yearly,ppi_mp_yearly,ppi_rm_yearly"),
                    "gdp" => ("gdp", "quarter,gdp,gdp_yoy,pi,si,ti"),
                    "pmi" => ("pmi", "month,pmi010000,pmibs,pmi_nbs"),
                    "money_supply" => ("money_supply", "month,m0,m1,m2,m0_yoy,m1_yoy,m2_yoy"),
                    "trade" => ("trade", "month,export,import,trade_balance"),
                    _ => {
                        return Ok(FinancialResult::err(format!(
                            "不支持的宏观指标: {indicator}。支持: cpi, ppi, gdp, pmi, money_supply, trade"
                        )));
                    }
                };
                let json = self
                    .call(api_name, serde_json::json!({"limit": 20}), fields)
                    .await?;
                let summary = Self::format_response(&json, 15);
                let data = json.get("data").cloned();
                Ok(FinancialResult::ok(summary, data))
            }

            FinancialQuery::StockIpos { .. }
            | FinancialQuery::BlockTrades
            | FinancialQuery::MarginTrading { .. }
            | FinancialQuery::StockSecurityInfo { .. }
            | FinancialQuery::EtfInfo { .. }
            | FinancialQuery::HkStockQuote { .. }
            | FinancialQuery::HkCompany { .. }
            | FinancialQuery::HkCandlestick { .. }
            | FinancialQuery::MacroUs { .. } => Ok(FinancialResult::err(
                "TuShare 暂不支持此查询类型，请使用 ftshare 或其他 provider",
            )),
        }
    }

    fn provider_name(&self) -> &str {
        "tushare"
    }
}
