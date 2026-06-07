// FinancialTool — bridges FinancialDataProvider into the Tool trait for agent use.

use std::sync::Arc;

use async_trait::async_trait;
use tools::{Tool, ToolOutput};

use crate::{FinancialDataProvider, FinancialQuery};

pub struct FinancialTool {
    provider: Arc<dyn FinancialDataProvider>,
}

impl FinancialTool {
    pub fn new(provider: Arc<dyn FinancialDataProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Tool for FinancialTool {
    fn name(&self) -> &str {
        "finance"
    }

    fn description(&self) -> &str {
        "查询金融数据，默认优先使用 FTShare/market.ft.tech，并自动降级到其他可用源。包括 A股行情、指数详情、基金信息、港股行情、宏观经济指标等。支持的查询类型需根据 schema 确定；如需 SkillHub FTShare 的完整子技能能力，优先调用 ftshare_market_data 工具。"
    }

    fn schema(&self) -> serde_json::Value {
        let caps = self.provider.capabilities();
        if caps.is_empty() {
            return serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "金融数据查询（当前未配置数据源，所有查询将返回未配置提示）"
                    }
                }
            });
        }

        let mut props = serde_json::Map::new();
        props.insert(
            "query_type".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "查询类型",
                "enum": caps.iter().map(|c| &c.name).collect::<Vec<_>>()
            }),
        );

        // Merge all capability parameters under a single "parameters" property
        // with per-type parameter descriptions in the enum descriptions
        let query_descriptions: Vec<String> = caps
            .iter()
            .map(|c| format!("{}: {}", c.name, c.description))
            .collect();

        props.insert(
            "parameters".to_string(),
            serde_json::json!({
                "type": "object",
                "description": format!(
                    "查询参数。可用查询类型: {}",
                    query_descriptions.join("; ")
                )
            }),
        );

        serde_json::json!({
            "type": "object",
            "properties": props,
            "required": ["query_type"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let query_type = args["query_type"].as_str().unwrap_or("").to_string();
        let params = args
            .get("parameters")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let query = match query_type.as_str() {
            "stock_list" => FinancialQuery::StockList,
            "stock_quote" => {
                let symbols = params["symbols"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let order_by = params["order_by"].as_str().unwrap_or("pct_chg").to_string();
                FinancialQuery::StockQuote { symbols, order_by }
            }
            "stock_ipos" => FinancialQuery::StockIpos {
                page: params["page"].as_u64().unwrap_or(1) as u32,
                page_size: params["page_size"].as_u64().unwrap_or(20) as u32,
            },
            "block_trades" => FinancialQuery::BlockTrades,
            "margin_trading" => FinancialQuery::MarginTrading {
                page: params["page"].as_u64().unwrap_or(1) as u32,
                page_size: params["page_size"].as_u64().unwrap_or(20) as u32,
            },
            "stock_security_info" => FinancialQuery::StockSecurityInfo {
                symbol: params["symbol"].as_str().unwrap_or("").to_string(),
            },
            "stock_history" => FinancialQuery::StockHistory {
                symbol: params["symbol"].as_str().unwrap_or("").to_string(),
                period: params["period"].as_str().unwrap_or("day").to_string(),
                limit: params["limit"].as_u64().unwrap_or(30) as u32,
            },
            "index_list" => FinancialQuery::IndexList {
                filter: params
                    .get("filter")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            },
            "index_detail" => FinancialQuery::IndexDetail {
                symbol: params["symbol"].as_str().unwrap_or("").to_string(),
            },
            "index_ohlc" => FinancialQuery::IndexOhlc {
                symbol: params["symbol"].as_str().unwrap_or("").to_string(),
                span: params["span"].as_str().unwrap_or("1m").to_string(),
                limit: params["limit"].as_u64().unwrap_or(30) as u32,
            },
            "fund_info" => FinancialQuery::FundInfo {
                code: params["code"].as_str().unwrap_or("").to_string(),
            },
            "fund_nav" => FinancialQuery::FundNav {
                code: params["code"].as_str().unwrap_or("").to_string(),
                page: params["page"].as_u64().unwrap_or(1) as u32,
            },
            "etf_info" => FinancialQuery::EtfInfo {
                code: params["code"].as_str().unwrap_or("").to_string(),
            },
            "hk_stock_quote" => {
                let symbols = params["symbols"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                FinancialQuery::HkStockQuote { symbols }
            }
            "hk_company" => FinancialQuery::HkCompany {
                symbol: params["symbol"].as_str().unwrap_or("").to_string(),
            },
            "hk_candlestick" => FinancialQuery::HkCandlestick {
                symbol: params["symbol"].as_str().unwrap_or("").to_string(),
                period: params["period"].as_str().unwrap_or("day").to_string(),
                limit: params["limit"].as_u64().unwrap_or(30) as u32,
            },
            "macro_china" => FinancialQuery::MacroChina {
                indicator: params["indicator"].as_str().unwrap_or("").to_string(),
            },
            "macro_us" => FinancialQuery::MacroUs {
                indicator_type: params["indicator_type"].as_str().unwrap_or("").to_string(),
            },
            _ => {
                return Ok(ToolOutput::error(format!(
                    "不支持的查询类型: {query_type}。可用类型请参考 finance 工具的 schema。"
                )));
            }
        };

        let result = match self.provider.query(query).await {
            Ok(result) => result,
            Err(err) => {
                return Ok(ToolOutput::error(format!(
                    "金融数据源暂时不可用，已尝试所有配置的数据源: {err:#}"
                )));
            }
        };

        if result.success {
            Ok(ToolOutput::text(result.to_string()))
        } else {
            Ok(ToolOutput::error(result.to_string()))
        }
    }
}
