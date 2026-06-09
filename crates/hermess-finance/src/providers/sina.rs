// SinaFinanceProvider — 新浪财经实时行情，无需 token，纯 HTTP GET

use async_trait::async_trait;
use std::time::Duration;

use super::retry::{self, RetryConfig};
use crate::{FinancialDataProvider, FinancialQuery, FinancialResult, QueryCapability};

pub struct SinaFinanceProvider {
    client: reqwest::Client,
}

impl Default for SinaFinanceProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SinaFinanceProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .build()
            .expect("Failed to build reqwest client for Sina");
        Self { client }
    }

    /// Convert a symbol to Sina format: "600519" → "sh600519", "000001" → "sz000001"
    pub fn to_sina_code(symbol: &str) -> Option<String> {
        let sym = symbol.trim();
        if sym.contains('.') {
            let parts: Vec<&str> = sym.split('.').collect();
            let exchange = parts.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
            let code = parts[0];
            return Some(format!("{}{}", exchange, code));
        }
        match sym.chars().next() {
            Some('6') => Some(format!("sh{sym}")),
            Some('9') => Some(format!("sh{sym}")),
            Some('0') | Some('3') => Some(format!("sz{sym}")),
            Some('4') | Some('8') => Some(format!("bj{sym}")),
            Some('5') => {
                if sym.len() == 6 {
                    Some(format!("sz{sym}"))
                } else {
                    Some(format!("sh{sym}"))
                }
            }
            _ => None,
        }
    }

    /// Parse Sina's JS-variable format into structured data.
    /// Format: `var hq_str_{code}="name,open,prev_close,price,high,low,..."`
    pub fn parse_quote(raw: &str, code: &str) -> Option<QuoteRow> {
        let start = raw.find('"')? + 1;
        let end = raw.rfind('"')?;
        let inner = &raw[start..end];
        let fields: Vec<&str> = inner.split(',').collect();
        if fields.len() < 6 {
            tracing::debug!(%code, fields = fields.len(), "Sina quote: too few fields");
            return None;
        }

        Some(QuoteRow {
            code: code.to_string(),
            name: fields[0].to_string(),
            open: fields[1].parse().ok(),
            prev_close: fields[2].parse().ok(),
            price: fields[3].parse().ok(),
            high: fields[4].parse().ok(),
            low: fields[5].parse().ok(),
            volume: fields.get(8).and_then(|v| v.parse().ok()),
            amount: fields.get(9).and_then(|v| v.parse().ok()),
            change: fields[3]
                .parse::<f64>()
                .ok()
                .and_then(|p| fields[2].parse::<f64>().ok().map(|prev| p - prev)),
            pct_chg: fields[3].parse::<f64>().ok().and_then(|p| {
                fields[2]
                    .parse::<f64>()
                    .ok()
                    .filter(|prev| *prev != 0.0)
                    .map(|prev| (p - prev) / prev * 100.0)
            }),
        })
    }

    /// Fetch and parse a batch of quotes with retry.
    async fn fetch_quotes(&self, list: &str) -> anyhow::Result<String> {
        let url = format!("http://hq.sinajs.cn/list={list}");
        let retry_cfg = RetryConfig::default();
        retry::with_retry(&retry_cfg, "sina_fetch", || {
            let url = url.clone();
            let client = self.client.clone();
            async move {
                let resp = client
                    .get(&url)
                    .header("Referer", "https://finance.sina.com.cn")
                    .send()
                    .await?
                    .text()
                    .await?;
                if resp.is_empty() || resp.len() < 20 {
                    return Err(anyhow::anyhow!(
                        "Sina returned empty/short response ({} bytes)",
                        resp.len()
                    ));
                }
                Ok(resp)
            }
        })
        .await
    }
}

#[derive(Debug, Clone)]
pub struct QuoteRow {
    pub code: String,
    pub name: String,
    pub open: Option<f64>,
    pub prev_close: Option<f64>,
    pub price: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub volume: Option<f64>,
    pub amount: Option<f64>,
    pub change: Option<f64>,
    pub pct_chg: Option<f64>,
}

impl std::fmt::Display for QuoteRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\t{}\t{}\t{}\t{}\t{}\t{:.2}%\t{}\t{}",
            self.code,
            self.name,
            self.price
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "-".into()),
            self.high
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "-".into()),
            self.low
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "-".into()),
            self.volume
                .map(|v| format!("{:.0}", v))
                .unwrap_or_else(|| "-".into()),
            self.pct_chg.unwrap_or(0.0),
            self.change
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "-".into()),
            self.amount
                .map(|v| format!("{:.0}", v))
                .unwrap_or_else(|| "-".into()),
        )
    }
}

#[async_trait]
impl FinancialDataProvider for SinaFinanceProvider {
    fn capabilities(&self) -> Vec<QueryCapability> {
        vec![
            QueryCapability {
                name: "stock_quote".into(),
                description: "查询股票实时行情（新浪财经源），支持批量查询沪深京A股".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbols": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "股票代码列表，如 [\"600519\", \"000001\", \"300750\"]"
                        }
                    },
                    "required": ["symbols"]
                }),
            },
            QueryCapability {
                name: "index_list".into(),
                description: "查询常用指数实时行情（上证/深证/沪深300/科创50等）".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        ]
    }

    async fn query(&self, q: FinancialQuery) -> anyhow::Result<FinancialResult> {
        match q {
            FinancialQuery::StockQuote { symbols, .. } => {
                if symbols.is_empty() {
                    return Ok(FinancialResult::err("请提供股票代码"));
                }

                let codes: Vec<String> = symbols
                    .iter()
                    .filter_map(|s| Self::to_sina_code(s))
                    .collect();

                if codes.is_empty() {
                    return Ok(FinancialResult::err("无法识别的股票代码格式"));
                }

                let list = codes.join(",");
                let resp = self.fetch_quotes(&list).await?;

                tracing::debug!(len = resp.len(), symbols = ?symbols, "Sina raw response");

                let lines: Vec<&str> = resp.lines().filter(|l| !l.trim().is_empty()).collect();
                let mut rows = Vec::new();
                for (i, line) in lines.iter().enumerate() {
                    if let Some(code) = symbols.get(i) {
                        if let Some(row) = Self::parse_quote(line, code) {
                            rows.push(row);
                        }
                    }
                }

                if rows.is_empty() {
                    tracing::warn!(symbols = ?symbols, code_count = codes.len(), "Sina quote: all parse failed");
                    return Ok(FinancialResult::err(
                        "行情数据解析失败，可能新浪接口返回异常",
                    ));
                }

                let header = "代码\t名称\t现价\t最高\t最低\t成交量\t涨跌幅\t涨跌额\t成交额";
                let body: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
                let summary = format!("{header}\n{}\n{}", "-".repeat(80), body.join("\n"));

                let data = serde_json::json!({
                    "fields": ["code","name","price","high","low","volume","amount","change","pct_chg"],
                    "items": rows.iter().map(|r| serde_json::json!([
                        r.code, r.name,
                        r.price, r.high, r.low, r.volume, r.amount, r.change, r.pct_chg
                    ])).collect::<Vec<_>>()
                });

                Ok(FinancialResult::ok(summary, Some(data)))
            }

            FinancialQuery::IndexList { .. } => {
                let indices = [
                    ("sh000001", "上证综指"),
                    ("sz399001", "深证成指"),
                    ("sh000300", "沪深300"),
                    ("sh000688", "科创50"),
                    ("sz399006", "创业板指"),
                    ("sh000016", "上证50"),
                    ("sz399005", "中小板指"),
                    ("sh000852", "中证1000"),
                ];

                let list = indices
                    .iter()
                    .map(|(c, _)| *c)
                    .collect::<Vec<_>>()
                    .join(",");
                let resp = self.fetch_quotes(&list).await?;

                let lines: Vec<&str> = resp.lines().filter(|l| !l.trim().is_empty()).collect();
                let mut rows = Vec::new();
                for (i, line) in lines.iter().enumerate() {
                    if let Some((code, name)) = indices.get(i) {
                        if let Some(row) = Self::parse_quote(line, code) {
                            rows.push(format!(
                                "{}\t{}\t{}\t{:.2}%",
                                code,
                                name,
                                row.price
                                    .map(|v| format!("{:.2}", v))
                                    .unwrap_or_else(|| "-".into()),
                                row.pct_chg.unwrap_or(0.0)
                            ));
                        }
                    }
                }

                if rows.is_empty() {
                    return Ok(FinancialResult::err("指数数据解析失败"));
                }

                let summary = format!(
                    "代码\t名称\t点位\t涨跌幅\n{}\n{}",
                    "-".repeat(50),
                    rows.join("\n")
                );
                Ok(FinancialResult::ok(summary, None))
            }

            _ => Ok(FinancialResult::err(
                "新浪财经数据源仅支持实时股票行情(stock_quote)和指数列表(index_list)。\n\
                 如需更多数据，请使用 tushare 或 ftshare provider。",
            )),
        }
    }

    fn provider_name(&self) -> &str {
        "sina"
    }
}
