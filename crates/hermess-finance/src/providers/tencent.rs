// TencentFinanceProvider — 腾讯财经免费 HTTP API，覆盖 A股/港股/美股行情与 K 线(JSON)

use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;

use super::retry::{self, RetryConfig};
use crate::{FinancialDataProvider, FinancialQuery, FinancialResult, QueryCapability};

pub struct TencentFinanceProvider {
    client: reqwest::Client,
}

impl Default for TencentFinanceProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl TencentFinanceProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .build()
            .expect("Failed to build reqwest client for Tencent");
        Self { client }
    }

    /// Build the quote request prefix for qt.gtimg.cn
    fn market_prefix(symbol: &str) -> Option<String> {
        let sym = symbol.trim();
        match sym.chars().next() {
            Some('6') | Some('9') => Some(format!("sh{sym}")),
            Some('0') | Some('3') | Some('4') | Some('8') => Some(format!("sz{sym}")),
            _ => None,
        }
    }

    /// Detect HK stock (5-digit numeric code, sometimes zero-padded)
    fn is_hk(sym: &str) -> bool {
        let s = sym.trim();
        s.len() >= 4 && s.len() <= 5 && s.chars().all(|c| c.is_ascii_digit())
    }

    /// Fetch real-time quotes from qt.gtimg.cn (text format: "v_<code>=\"fields~separated\"")
    async fn fetch_quotes(&self, symbols: &[String]) -> anyhow::Result<Vec<TencentQuote>> {
        let query: Vec<String> = symbols
            .iter()
            .map(|s| {
                let s = s.trim();
                if TencentFinanceProvider::is_hk(s) {
                    format!("hk{}", s.trim_start_matches('0'))
                } else if s.chars().all(|c| c.is_ascii_alphabetic()) {
                    // US stock like "BABA"
                    format!("us{}", s.to_uppercase())
                } else if let Some(prefix) = TencentFinanceProvider::market_prefix(s) {
                    prefix
                } else {
                    s.to_string()
                }
            })
            .collect();

        let url = format!("http://qt.gtimg.cn/q={}", query.join(","));

        let retry_cfg = RetryConfig::with_delays(3, 500, 10000);
        retry::with_retry(&retry_cfg, "tencent_quote", || {
            let url = url.clone();
            let client = self.client.clone();
            async move {
                let resp = client
                    .get(&url)
                    .header("Referer", "https://gu.qq.com")
                    .send()
                    .await?
                    .text()
                    .await?;
                if resp.is_empty() || resp.len() < 20 {
                    return Err(anyhow::anyhow!(
                        "Tencent returned short response ({} bytes)",
                        resp.len()
                    ));
                }

                let mut quotes = Vec::new();
                for line in resp.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    // Format: v_sh600519="1~name~code~price~..."
                    if let Some(start) = line.find('"') {
                        let end = line.rfind('"').unwrap_or(line.len());
                        let inner = &line[start + 1..end];
                        let fields: Vec<&str> = inner.split('~').collect();
                        if fields.len() >= 40 {
                            quotes.push(TencentQuote {
                                name: fields.get(1).unwrap_or(&"").to_string(),
                                code: fields.get(2).unwrap_or(&"").to_string(),
                                price: fields.get(3).and_then(|s| s.parse().ok()),
                                prev_close: fields.get(4).and_then(|s| s.parse().ok()),
                                open: fields.get(5).and_then(|s| s.parse().ok()),
                                volume: fields.get(6).and_then(|s| s.parse().ok()),
                                high: fields.get(33).and_then(|s| s.parse().ok()),
                                low: fields.get(34).and_then(|s| s.parse().ok()),
                                change: fields.get(31).and_then(|s| s.parse().ok()),
                                pct_chg: fields.get(32).and_then(|s| s.parse().ok()),
                                amount: fields
                                    .get(37)
                                    .and_then(|s| s.parse::<f64>().ok())
                                    .map(|v| v * 10000.0),
                                turnover: fields.get(38).and_then(|s| s.parse().ok()),
                                pe_ttm: fields.get(39).and_then(|s| s.parse().ok()),
                                amplitude: fields.get(43).and_then(|s| s.parse().ok()),
                                circ_mcap: fields.get(44).and_then(|s| s.parse().ok()),
                                total_mcap: fields.get(45).and_then(|s| s.parse().ok()),
                                pb: fields.get(46).and_then(|s| s.parse().ok()),
                                high_limit: fields.get(47).and_then(|s| s.parse().ok()),
                                low_limit: fields.get(48).and_then(|s| s.parse().ok()),
                                qty_ratio: fields.get(49).and_then(|s| s.parse().ok()),
                            });
                        }
                    }
                }
                Ok(quotes)
            }
        })
        .await
    }

    /// Fetch K-line history from Tencent JSON API
    async fn fetch_kline(
        &self,
        symbol: &str,
        period: &str,
        limit: u32,
    ) -> anyhow::Result<TencentKlineResult> {
        let sym = if TencentFinanceProvider::is_hk(symbol) {
            format!("hk{}", symbol.trim().trim_start_matches('0'))
        } else if let Some(p) = TencentFinanceProvider::market_prefix(symbol) {
            p
        } else {
            symbol.to_string()
        };

        let prd = match period {
            "d" | "day" => "day",
            "w" | "week" => "week",
            "m" | "month" => "month",
            _ => "day",
        };

        let url = format!(
            "https://web.ifzq.gtimg.cn/appstock/app/fqkline/get?param={},{},1900-01-01,2099-12-31,{},qfq",
            sym, prd, limit.clamp(10, 2000)
        );

        let retry_cfg = RetryConfig::with_delays(3, 500, 10000);
        retry::with_retry(&retry_cfg, "tencent_kline", || {
            let url = url.clone();
            let client = self.client.clone();
            async move {
                let resp = client
                    .get(&url)
                    .header("Referer", "https://gu.qq.com")
                    .send()
                    .await?
                    .text()
                    .await?;
                let parsed: TencentKlineResponse = serde_json::from_str(&resp)
                    .map_err(|e| anyhow::anyhow!("Tencent kline parse error: {e}"))?;
                Ok(parsed.into_result())
            }
        })
        .await
    }

    /// Fetch minute/intraday data
    #[allow(dead_code)]
    async fn fetch_minute(&self, symbol: &str) -> anyhow::Result<serde_json::Value> {
        let sym = if let Some(p) = TencentFinanceProvider::market_prefix(symbol) {
            p
        } else {
            symbol.to_string()
        };
        let url = format!(
            "https://web.ifzq.gtimg.cn/appstock/app/minute/query?code={}",
            sym
        );

        let retry_cfg = RetryConfig::with_delays(3, 500, 10000);
        retry::with_retry(&retry_cfg, "tencent_minute", || {
            let url = url.clone();
            let client = self.client.clone();
            async move {
                let resp = client.get(&url).send().await?.text().await?;
                let val: serde_json::Value = serde_json::from_str(&resp)
                    .map_err(|e| anyhow::anyhow!("Tencent minute parse error: {e}"))?;
                Ok(val)
            }
        })
        .await
    }
}

// ── Quote struct ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct TencentQuote {
    name: String,
    code: String,
    price: Option<f64>,
    prev_close: Option<f64>,
    open: Option<f64>,
    volume: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    change: Option<f64>,
    pct_chg: Option<f64>,
    amount: Option<f64>,
    turnover: Option<f64>,
    pe_ttm: Option<f64>,
    amplitude: Option<f64>,
    circ_mcap: Option<f64>,
    total_mcap: Option<f64>,
    pb: Option<f64>,
    high_limit: Option<f64>,
    low_limit: Option<f64>,
    qty_ratio: Option<f64>,
}

impl std::fmt::Display for TencentQuote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\t{}\t{}\t{}\t{}\t{}\t{:.2}%\tPE:{:.2}",
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
            self.pe_ttm.unwrap_or(0.0),
        )
    }
}

// ── K-line JSON types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TencentKlineResponse {
    data: Option<serde_json::Value>,
}

/// K-line result extracted from Tencent's nested JSON structure.
struct TencentKlineResult {
    name: String,
    code: String,
    klines: Vec<String>,
}

impl TencentKlineResponse {
    fn into_result(self) -> TencentKlineResult {
        let data = self.data.unwrap_or_default();
        // Navigate: data -> {stock_code} -> qfqday (or day)
        // The response shape: {"data": {"sh600519": {"qfqday": [...], "day": [...]}}}
        let stock_obj = data.as_object();
        let mut name = String::new();
        let mut code = String::new();
        let mut klines = Vec::new();

        for (key, val) in stock_obj.iter().flat_map(|o| o.iter()) {
            if key == "name" {
                name = val.as_str().unwrap_or("").to_string();
            } else if key == "code" {
                code = val.as_str().unwrap_or("").to_string();
            } else if let Some(obj) = val.as_object() {
                // Look for qfqday, hfqday, or day keys
                for (period_key, period_val) in obj.iter() {
                    if period_key.contains("day")
                        || period_key.contains("week")
                        || period_key.contains("month")
                    {
                        if let Some(arr) = period_val.as_array() {
                            for item in arr {
                                if let Some(arr2) = item.as_array() {
                                    let row: Vec<String> = arr2
                                        .iter()
                                        .map(|v| v.as_str().unwrap_or("").to_string())
                                        .collect();
                                    klines.push(row.join(","));
                                }
                            }
                        }
                    }
                }
            }
        }

        TencentKlineResult { name, code, klines }
    }
}

// ── FinancialDataProvider impl ───────────────────────────────────────

#[async_trait]
impl FinancialDataProvider for TencentFinanceProvider {
    fn capabilities(&self) -> Vec<QueryCapability> {
        vec![
            QueryCapability {
                name: "stock_quote".into(),
                description:
                    "查询股票实时行情（腾讯财经源），支持沪深A股、港股、美股，含市盈率/市值等87字段"
                        .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbols": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "股票代码列表，如 [\"600519\", \"000001\", \"00700\", \"BABA\"]"
                        }
                    },
                    "required": ["symbols"]
                }),
            },
            QueryCapability {
                name: "stock_history".into(),
                description:
                    "查询股票历史K线（腾讯财经源），支持日/周/月线，返回JSON格式含前复权数据".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbol": {"type": "string", "description": "股票代码，如 600519、00700"},
                        "period": {"type": "string", "description": "日=d, 周=w, 月=m"},
                        "limit": {"type": "integer", "description": "K线条数，最大 2000"}
                    },
                    "required": ["symbol"]
                }),
            },
            QueryCapability {
                name: "hk_stock_quote".into(),
                description: "查询港股实时行情（腾讯财经源）".into(),
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
            QueryCapability {
                name: "index_list".into(),
                description: "查询常用指数实时行情（上证/深证/沪深300/恒指/纳指等）".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            },
        ]
    }

    async fn query(&self, q: FinancialQuery) -> anyhow::Result<FinancialResult> {
        match q {
            FinancialQuery::StockQuote { symbols, .. } => {
                if symbols.is_empty() {
                    return Ok(FinancialResult::err("请提供股票代码"));
                }
                let quotes = self.fetch_quotes(&symbols).await?;
                if quotes.is_empty() {
                    return Ok(FinancialResult::err("腾讯财经行情数据为空，请稍后重试"));
                }

                let header = "代码\t名称\t现价\t最高\t最低\t成交量\t涨跌幅\t市盈率";
                let body: Vec<String> = quotes.iter().map(|r| r.to_string()).collect();
                let summary = format!("{header}\n{}\n{}", "-".repeat(70), body.join("\n"));

                let data = serde_json::json!({
                    "items": quotes.iter().map(|r| serde_json::json!({
                        "code": r.code, "name": r.name,
                        "price": r.price, "high": r.high, "low": r.low, "open": r.open,
                        "prev_close": r.prev_close, "volume": r.volume, "amount": r.amount,
                        "pct_chg": r.pct_chg, "change": r.change, "turnover": r.turnover,
                        "pe_ttm": r.pe_ttm, "pb": r.pb,
                        "total_mcap": r.total_mcap, "circ_mcap": r.circ_mcap,
                        "amplitude": r.amplitude, "qty_ratio": r.qty_ratio,
                        "high_limit": r.high_limit, "low_limit": r.low_limit,
                    })).collect::<Vec<_>>()
                });
                Ok(FinancialResult::ok(summary, Some(data)))
            }

            FinancialQuery::HkStockQuote { symbols } => {
                if symbols.is_empty() {
                    return Ok(FinancialResult::err("请提供港股代码"));
                }
                let quotes = self.fetch_quotes(&symbols).await?;
                if quotes.is_empty() {
                    return Ok(FinancialResult::err("港股行情数据为空"));
                }
                let header = "代码\t名称\t现价(HKD)\t最高\t最低\t成交量\t涨跌幅\t市盈率";
                let body: Vec<String> = quotes.iter().map(|r| r.to_string()).collect();
                let summary = format!("{header}\n{}\n{}", "-".repeat(70), body.join("\n"));
                Ok(FinancialResult::ok(summary, None))
            }

            FinancialQuery::StockHistory {
                symbol,
                period,
                limit,
            } => {
                let result = self.fetch_kline(&symbol, &period, limit).await?;
                if result.klines.is_empty() {
                    return Ok(FinancialResult::err(format!("{} 无K线数据", symbol)));
                }

                let period_label = match period.as_str() {
                    "d" | "day" => "日线",
                    "w" | "week" => "周线",
                    "m" | "month" => "月线",
                    _ => "日线",
                };

                let parsed: Vec<String> = result
                    .klines
                    .iter()
                    .map(|k| {
                        let parts: Vec<&str> = k.split(',').collect();
                        if parts.len() >= 6 {
                            format!(
                                "{}\tO:{}\tC:{}\tH:{}\tL:{}\tV:{}",
                                parts[0],
                                parts.get(1).unwrap_or(&"-"),
                                parts.get(2).unwrap_or(&"-"),
                                parts.get(3).unwrap_or(&"-"),
                                parts.get(4).unwrap_or(&"-"),
                                parts.get(5).unwrap_or(&"-"),
                            )
                        } else {
                            k.clone()
                        }
                    })
                    .collect();

                let summary = format!(
                    "{} {} {} ({}, 前复权)\n日期\t开盘\t收盘\t最高\t最低\t成交量\n{}\n{}",
                    result.code,
                    result.name,
                    period_label,
                    result.klines.len(),
                    "-".repeat(65),
                    parsed.join("\n"),
                );
                let data = serde_json::json!({
                    "code": result.code, "name": result.name,
                    "klines": result.klines,
                });
                Ok(FinancialResult::ok(summary, Some(data)))
            }

            FinancialQuery::IndexList { .. } => {
                let indices = [
                    "sh000001", "sz399001", "sh000300", "sh000688", "sz399006", "sh000016",
                    "sz399005", "sh000852",
                ];
                let symbols: Vec<String> = indices.iter().map(|s| s.to_string()).collect();
                let quotes = self.fetch_quotes(&symbols).await?;

                let names = [
                    "上证综指",
                    "深证成指",
                    "沪深300",
                    "科创50",
                    "创业板指",
                    "上证50",
                    "中小板指",
                    "中证1000",
                ];

                let mut items = Vec::new();
                for (i, q) in quotes.iter().enumerate() {
                    let label = names.get(i).unwrap_or(&"");
                    items.push(format!(
                        "{}\t{:.2}\t{:.2}%",
                        label,
                        q.price.unwrap_or(0.0),
                        q.pct_chg.unwrap_or(0.0),
                    ));
                }
                let summary = format!(
                    "指数\t点位\t涨跌幅\n{}\n{}",
                    "-".repeat(40),
                    items.join("\n")
                );
                Ok(FinancialResult::ok(summary, None))
            }

            _ => Ok(FinancialResult::err(
                "腾讯财经数据源支持: stock_quote, stock_history, hk_stock_quote, index_list",
            )),
        }
    }

    fn provider_name(&self) -> &str {
        "tencent"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_prefix_shanghai() {
        assert_eq!(
            TencentFinanceProvider::market_prefix("600519"),
            Some("sh600519".into())
        );
    }

    #[test]
    fn market_prefix_shenzhen() {
        assert_eq!(
            TencentFinanceProvider::market_prefix("000001"),
            Some("sz000001".into())
        );
        assert_eq!(
            TencentFinanceProvider::market_prefix("300750"),
            Some("sz300750".into())
        );
    }

    #[test]
    fn market_prefix_beijing() {
        assert_eq!(
            TencentFinanceProvider::market_prefix("430047"),
            Some("sz430047".into())
        );
    }

    #[test]
    fn is_hk_stock() {
        assert!(TencentFinanceProvider::is_hk("00700"));
        assert!(TencentFinanceProvider::is_hk("09988"));
        assert!(!TencentFinanceProvider::is_hk("600519"));
    }

    #[test]
    fn kline_response_into_result_empty() {
        let resp = TencentKlineResponse { data: None };
        let result = resp.into_result();
        assert!(result.klines.is_empty());
    }
}
