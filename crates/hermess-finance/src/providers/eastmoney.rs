// EastMoneyProvider — 东方财富免费 HTTP API，覆盖 A股/港股/美股/指数行情与 K 线

use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;

use super::retry::{self, RetryConfig};
use crate::{FinancialDataProvider, FinancialQuery, FinancialResult, QueryCapability};

pub struct EastMoneyProvider {
    client: reqwest::Client,
}

impl Default for EastMoneyProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl EastMoneyProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::REFERER,
                    reqwest::header::HeaderValue::from_static("https://quote.eastmoney.com"),
                );
                headers
            })
            .build()
            .expect("Failed to build reqwest client for EastMoney");
        Self { client }
    }

    /// Convert symbol to EastMoney secid: "600519" → "1.600519", "000001" → "0.000001"
    fn to_secid(symbol: &str, market: u8) -> String {
        format!("{}.{}", market, symbol.trim())
    }

    /// Infer market from code prefix. Returns None for unrecognized codes.
    fn infer_market(sym: &str) -> Option<u8> {
        match sym.chars().next() {
            Some('6') | Some('9') => Some(1), // Shanghai
            Some('0') | Some('3') => Some(0), // Shenzhen
            Some('4') | Some('8') => Some(0), // Beijing (use SZ market)
            _ => None,
        }
    }

    /// Detect HK stock by checking for 5-digit code starting with 0
    fn is_hk(sym: &str) -> bool {
        // HK stocks have codes like 00700, 09988 (5 digits, often zero-padded)
        sym.len() == 5 && sym.chars().all(|c| c.is_ascii_digit())
    }

    /// Fetch real-time quotes via EastMoney push2 API (JSON).
    async fn fetch_quotes(&self, secids: &str) -> anyhow::Result<EastMoneyQuoteResponse> {
        let fields = [
            "f57",  // code
            "f58",  // name
            "f43",  // latest price
            "f44",  // high
            "f45",  // low
            "f46",  // open
            "f60",  // prev_close
            "f47",  // volume (手)
            "f48",  // amount (元)
            "f170", // pct_chg
            "f169", // change
            "f168", // turnover rate
            "f167", // PB
            "f162", // PE (TTM)
            "f116", // total_market_cap
            "f117", // circulating_market_cap
            "f50",  // quantity_ratio
            "f51",  // amplitude
            "f52",  // high_limit
            "f53",  // low_limit
        ];

        let url = format!(
            "https://push2.eastmoney.com/api/qt/stock/get?secid={}&fields={}",
            secids,
            fields.join(",")
        );

        let retry_cfg = RetryConfig::with_delays(3, 500, 10_000);
        retry::with_retry(&retry_cfg, "eastmoney_quote", || {
            let url = url.clone();
            let client = self.client.clone();
            async move {
                let resp = client.get(&url).send().await?;
                let text = resp.text().await?;
                if text.is_empty() || text.len() < 30 {
                    return Err(anyhow::anyhow!(
                        "EastMoney returned short response ({} bytes)",
                        text.len()
                    ));
                }
                let parsed: EastMoneyQuoteResponse = serde_json::from_str(&text).map_err(|e| {
                    anyhow::anyhow!("EastMoney parse error: {e} (raw {} bytes)", text.len())
                })?;
                Ok(parsed)
            }
        })
        .await
    }

    /// Fetch K-line data
    async fn fetch_kline(
        &self,
        secid: &str,
        klt: u32, // 101=day, 102=week, 103=month, 1/5/15/30/60=minute
        fqt: u32, // 0=不复权, 1=前复权, 2=后复权
        limit: u32,
    ) -> anyhow::Result<KlineResponse> {
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid={}&klt={}&fqt={}&lmt={}&fields=f57,f58,f43,f44,f45,f46,f60,f47,f48,f51,f170,f169,f168,f162,f52,f53",
            secid, klt, fqt, limit
        );

        let retry_cfg = RetryConfig::with_delays(3, 500, 10_000);
        retry::with_retry(&retry_cfg, "eastmoney_kline", || {
            let url = url.clone();
            let client = self.client.clone();
            async move {
                let resp = client.get(&url).send().await?;
                let text = resp.text().await?;
                let parsed: KlineResponse = serde_json::from_str(&text)
                    .map_err(|e| anyhow::anyhow!("EastMoney kline parse error: {e}"))?;
                Ok(parsed)
            }
        })
        .await
    }

    /// Fetch stock/market list via clist endpoint
    async fn fetch_clist(&self, fs: &str, pz: u32) -> anyhow::Result<ClistResponse> {
        let url = format!(
            "https://push2.eastmoney.com/api/qt/clist/get?pn=1&pz={}&po=1&np=1&fltt=2&invt=2&fid=f12&fs={}&fields=f2,f3,f4,f5,f6,f7,f8,f9,f10,f12,f14,f15,f16,f17,f18,f20,f21,f23",
            pz, fs
        );

        let retry_cfg = RetryConfig::with_delays(3, 500, 10_000);
        retry::with_retry(&retry_cfg, "eastmoney_clist", || {
            let url = url.clone();
            let client = self.client.clone();
            async move {
                let resp = client.get(&url).send().await?;
                let text = resp.text().await?;
                let parsed: ClistResponse = serde_json::from_str(&text)
                    .map_err(|e| anyhow::anyhow!("EastMoney clist parse error: {e}"))?;
                Ok(parsed)
            }
        })
        .await
    }
}

// ── JSON response types ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct EastMoneyQuoteResponse {
    data: Option<QuoteData>,
}

#[derive(Debug, Deserialize)]
struct QuoteData {
    #[serde(rename = "total")]
    _total: Option<i64>,
    #[serde(default)]
    diff: Vec<QuoteItem>,
}

#[derive(Debug, Deserialize, Clone)]
struct QuoteItem {
    #[serde(default)]
    f57: Option<String>, // code (string from API)
    #[serde(default)]
    f58: Option<String>, // name
    #[serde(default)]
    f43: Option<f64>, // latest price
    #[serde(default)]
    f44: Option<f64>, // high
    #[serde(default)]
    f45: Option<f64>, // low
    #[serde(default)]
    f46: Option<f64>, // open
    #[serde(default)]
    f60: Option<f64>, // prev_close
    #[serde(default)]
    f47: Option<f64>, // volume
    #[serde(default)]
    f48: Option<f64>, // amount
    #[serde(default)]
    f170: Option<f64>, // pct_chg
    #[serde(default)]
    f169: Option<f64>, // change
    #[serde(default)]
    f168: Option<f64>, // turnover rate
    #[serde(default)]
    f167: Option<f64>, // PB
    #[serde(default)]
    f162: Option<f64>, // PE TTM
    #[serde(default)]
    f116: Option<f64>, // total market cap
    #[serde(default)]
    f117: Option<f64>, // circulating market cap
    #[serde(default)]
    f50: Option<f64>, // quantity ratio
    #[serde(default)]
    f51: Option<f64>, // amplitude
    #[serde(default)]
    f52: Option<f64>, // high limit
    #[serde(default)]
    f53: Option<f64>, // low limit
}

impl std::fmt::Display for QuoteItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\t{}\t{}\t{}\t{}\t{}\t{:.2}%\tPE:{}",
            self.f57.as_deref().unwrap_or("-"),
            self.f58.as_deref().unwrap_or("-"),
            self.f43
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "-".into()),
            self.f44
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "-".into()),
            self.f45
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "-".into()),
            self.f47
                .map(|v| format!("{:.0}", v))
                .unwrap_or_else(|| "-".into()),
            self.f170.unwrap_or(0.0),
            self.f162
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "-".into()),
        )
    }
}

#[derive(Debug, Deserialize)]
struct KlineResponse {
    data: Option<KlineData>,
}

#[derive(Debug, Deserialize)]
struct KlineData {
    #[serde(default)]
    klines: Vec<String>,
    name: Option<String>,
    code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClistResponse {
    data: Option<ClistData>,
}

#[derive(Debug, Deserialize)]
struct ClistData {
    #[serde(rename = "diff")]
    stocks: Option<Vec<serde_json::Value>>,
}

// ── FinancialDataProvider impl ───────────────────────────────────────

#[async_trait]
impl FinancialDataProvider for EastMoneyProvider {
    fn capabilities(&self) -> Vec<QueryCapability> {
        vec![
            QueryCapability {
                name: "stock_quote".into(),
                description: "查询股票实时行情（东方财富源），支持沪深A股、港股、美股".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbols": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "股票代码列表，如 [\"600519\", \"000001\", \"00700\"]"
                        }
                    },
                    "required": ["symbols"]
                }),
            },
            QueryCapability {
                name: "stock_list".into(),
                description: "获取沪深A股全市场列表（含涨跌幅、换手率等）".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            },
            QueryCapability {
                name: "stock_history".into(),
                description: "查询股票历史K线（东方财富源），支持日/周/月/分钟线，可选前复权"
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbol": {"type": "string", "description": "股票代码如 600519"},
                        "period": {"type": "string", "description": "日=d, 周=w, 月=m, 5/15/30/60 分钟"},
                        "limit": {"type": "integer", "description": "K线条数，最大 2000"}
                    },
                    "required": ["symbol"]
                }),
            },
            QueryCapability {
                name: "index_list".into(),
                description: "查询常用指数实时行情（上证/深证/沪深300/科创50/创业板/恒指/纳指等）"
                    .into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
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
            FinancialQuery::StockQuote { symbols, .. } => {
                if symbols.is_empty() {
                    return Ok(FinancialResult::err("请提供股票代码"));
                }
                let secids: Vec<String> = symbols
                    .iter()
                    .filter_map(|s| {
                        let s = s.trim();
                        if EastMoneyProvider::is_hk(s) {
                            Some(EastMoneyProvider::to_secid(s, 116))
                        } else {
                            EastMoneyProvider::infer_market(s)
                                .map(|m| EastMoneyProvider::to_secid(s, m))
                        }
                    })
                    .collect();

                if secids.is_empty() {
                    return Ok(FinancialResult::err("无法识别的股票代码格式"));
                }

                let mut all_items = Vec::new();
                for chunk in secids.chunks(20) {
                    let secids_str = chunk.join(",");
                    match self.fetch_quotes(&secids_str).await {
                        Ok(resp) => {
                            if let Some(data) = resp.data {
                                all_items.extend(data.diff);
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "EastMoney batch failed, continuing with remaining");
                        }
                    }
                }

                if all_items.is_empty() {
                    return Ok(FinancialResult::err("东方财富行情数据为空，请稍后重试"));
                }

                let header = "代码\t名称\t现价\t最高\t最低\t成交量\t涨跌幅\t市盈率";
                let body: Vec<String> = all_items.iter().map(|r| r.to_string()).collect();
                let summary = format!("{header}\n{}\n{}", "-".repeat(70), body.join("\n"));

                let data = serde_json::json!({
                    "items": all_items.iter().map(|r| serde_json::json!({
                        "code": r.f57, "name": r.f58,
                        "price": r.f43, "high": r.f44, "low": r.f45, "open": r.f46,
                        "prev_close": r.f60, "volume": r.f47, "amount": r.f48,
                        "pct_chg": r.f170, "change": r.f169, "turnover": r.f168,
                        "pe_ttm": r.f162, "pb": r.f167,
                        "total_mcap": r.f116, "circ_mcap": r.f117,
                        "amplitude": r.f51, "qty_ratio": r.f50,
                        "high_limit": r.f52, "low_limit": r.f53
                    })).collect::<Vec<_>>()
                });
                Ok(FinancialResult::ok(summary, Some(data)))
            }

            FinancialQuery::StockList => {
                // m:0+t:6 = 深A, m:0+t:80 = 深B, m:1+t:2 = 沪A, m:1+t:23 = 沪B
                let fs = "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23";
                let resp = self.fetch_clist(fs, 200).await?;
                match resp.data {
                    Some(data) => {
                        let stocks = data.stocks.unwrap_or_default();
                        let count = stocks.len();
                        let top: Vec<String> = stocks.iter().take(30).map(|item| {
                            let code = item["f12"].as_str().unwrap_or("-");
                            let name = item["f14"].as_str().unwrap_or("-");
                            let price = item["f2"].as_f64().map(|v| format!("{:.2}", v)).unwrap_or_else(|| "-".into());
                            let pct = item["f3"].as_f64().unwrap_or(0.0);
                            format!("{code}\t{name}\t{price}\t{pct:.2}%")
                        }).collect();
                        let summary = format!(
                            "代码\t名称\t现价\t涨跌幅\n{}\n{} (共 {count} 只)\n...仅显示前 30 条",
                            "-".repeat(50),
                            top.join("\n"),
                        );
                        let data = serde_json::json!({"total": count, "stocks": stocks});
                        Ok(FinancialResult::ok(summary, Some(data)))
                    }
                    None => Ok(FinancialResult::err("东方财富股票列表返回为空")),
                }
            }

            FinancialQuery::StockHistory { symbol, period, limit } => {
                let sym = symbol.trim();
                let market = if EastMoneyProvider::is_hk(sym) {
                    116
                } else {
                    EastMoneyProvider::infer_market(sym)
                        .ok_or_else(|| anyhow::anyhow!("无法识别股票代码: {sym}"))?
                };
                let secid = EastMoneyProvider::to_secid(sym, market);

                let klt = match period.as_str() {
                    "d" | "day" => 101,
                    "w" | "week" => 102,
                    "m" | "month" => 103,
                    "1" => 1,
                    "5" => 5,
                    "15" => 15,
                    "30" => 30,
                    "60" => 60,
                    _ => 101,
                };

                let resp = self.fetch_kline(&secid, klt, 1, limit.clamp(10, 2000)).await?;
                match resp.data {
                    Some(data) => {
                        let name = data.name.unwrap_or_default();
                        let code = data.code.unwrap_or_default();
                        let klines = &data.klines;
                        if klines.is_empty() {
                            return Ok(FinancialResult::err(format!("{code} 无K线数据")));
                        }
                        // K-line format: "date,open,close,high,low,volume,amount,amplitude,pct_chg,change,turnover"
                        let parsed: Vec<String> = klines.iter().map(|k| {
                            let parts: Vec<&str> = k.split(',').collect();
                            if parts.len() >= 11 {
                                format!(
                                    "{}\tO:{}\tC:{}\tH:{}\tL:{}\tV:{}\t{:.2}%",
                                    parts[0],
                                    parts.get(1).unwrap_or(&"-"),
                                    parts.get(2).unwrap_or(&"-"),
                                    parts.get(3).unwrap_or(&"-"),
                                    parts.get(4).unwrap_or(&"-"),
                                    parts.get(5).unwrap_or(&"-"),
                                    parts.get(8).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0),
                                )
                            } else {
                                k.clone()
                            }
                        }).collect();

                        let summary = format!(
                            "{code} {name} K线 ({period}, 前复权)\n日期\t开盘\t收盘\t最高\t最低\t成交量\t涨跌幅\n{}\n{}",
                            "-".repeat(70),
                            parsed.join("\n"),
                        );
                        let data = serde_json::json!({ "code": code, "name": name, "klines": klines });
                        Ok(FinancialResult::ok(summary, Some(data)))
                    }
                    None => Ok(FinancialResult::err("东方财富K线数据为空")),
                }
            }

            FinancialQuery::IndexList { .. } => {
                let indices = [
                    ("1.000001", "上证综指"), ("0.399001", "深证成指"),
                    ("1.000300", "沪深300"), ("1.000688", "科创50"),
                    ("0.399006", "创业板指"), ("1.000016", "上证50"),
                    ("0.399005", "中小板指"), ("1.000852", "中证1000"),
                    ("100.HSI", "恒生指数"), ("100.HSCEI", "国企指数"),
                    ("105.NDX", "纳斯达克100"), ("100.DJIA", "道琼斯"),
                ];
                let secids = indices.iter().map(|(s, _)| *s).collect::<Vec<_>>().join(",");
                let resp = self.fetch_quotes(&secids).await?;
                match resp.data {
                    Some(data) => {
                        let items: Vec<String> = data.diff.iter().map(|item| {
                            format!(
                                "{}\t{}\t{:.2}%",
                                item.f58.as_deref().unwrap_or("-"),
                                item.f43.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "-".into()),
                                item.f170.unwrap_or(0.0),
                            )
                        }).collect();
                        let summary = format!("指数\t点位\t涨跌幅\n{}\n{}", "-".repeat(40), items.join("\n"));
                        let data = serde_json::json!({
                            "items": data.diff.iter().map(|r| serde_json::json!({
                                "name": r.f58, "price": r.f43, "pct_chg": r.f170,
                                "high": r.f44, "low": r.f45, "volume": r.f47,
                            })).collect::<Vec<_>>()
                        });
                        Ok(FinancialResult::ok(summary, Some(data)))
                    }
                    None => Ok(FinancialResult::err("东方财富指数数据为空")),
                }
            }

            FinancialQuery::HkStockQuote { symbols } => {
                if symbols.is_empty() {
                    return Ok(FinancialResult::err("请提供港股代码"));
                }
                let secids: Vec<String> = symbols
                    .iter()
                    .map(|s| EastMoneyProvider::to_secid(s.trim(), 116))
                    .collect();
                let secids_str = secids.join(",");
                let resp = self.fetch_quotes(&secids_str).await?;
                match resp.data {
                    Some(data) => {
                        let items: Vec<String> = data.diff.iter().map(|r| r.to_string()).collect();
                        let header = "代码\t名称\t现价(HKD)\t最高\t最低\t成交量\t涨跌幅";
                        let summary = format!("{header}\n{}\n{}", "-".repeat(70), items.join("\n"));
                        Ok(FinancialResult::ok(summary, Some(serde_json::json!({"items": data.diff.iter().map(|r| serde_json::json!({
                            "code": r.f57, "name": r.f58,
                            "price": r.f43, "pct_chg": r.f170, "volume": r.f47,
                            "pe_ttm": r.f162,
                        })).collect::<Vec<_>>()
                        }))))
                    }
                    None => Ok(FinancialResult::err("港股行情数据为空")),
                }
            }

            _ => Ok(FinancialResult::err(
                "东方财富数据源支持: stock_quote, stock_list, stock_history, index_list, hk_stock_quote"
            )),
        }
    }

    fn provider_name(&self) -> &str {
        "eastmoney"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_market_shanghai() {
        assert_eq!(EastMoneyProvider::infer_market("600519"), Some(1));
        assert_eq!(EastMoneyProvider::infer_market("900901"), Some(1));
    }

    #[test]
    fn infer_market_shenzhen() {
        assert_eq!(EastMoneyProvider::infer_market("000001"), Some(0));
        assert_eq!(EastMoneyProvider::infer_market("300750"), Some(0));
    }

    #[test]
    fn infer_market_unknown() {
        assert_eq!(EastMoneyProvider::infer_market("ABC"), None);
    }

    #[test]
    fn test_to_secid() {
        assert_eq!(EastMoneyProvider::to_secid("600519", 1), "1.600519");
        assert_eq!(EastMoneyProvider::to_secid("000001", 0), "0.000001");
        assert_eq!(EastMoneyProvider::to_secid("00700", 116), "116.00700");
    }

    #[test]
    fn test_is_hk() {
        assert!(EastMoneyProvider::is_hk("00700"));
        assert!(EastMoneyProvider::is_hk("09988"));
        assert!(!EastMoneyProvider::is_hk("600519"));
        assert!(!EastMoneyProvider::is_hk("000001"));
    }
}
