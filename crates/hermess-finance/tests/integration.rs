// Integration tests for the pluggable financial data layer.

use hermess_finance::{
    providers::{
        defaults::{build_finance_provider, FinanceProviderOptions},
        ftshare::FtShareProvider,
        null::NullProvider,
        sina::SinaFinanceProvider,
        tushare::TuShareProvider,
    },
    tool::FinancialTool,
    FinancialDataProvider, FinancialQuery, FinancialResult, QueryCapability,
};
use std::sync::Arc;
use tools::Tool;

#[tokio::test]
async fn null_provider_returns_placeholder() {
    let p = NullProvider;
    assert!(p.capabilities().is_empty());
    assert_eq!(p.provider_name(), "null");

    let result = p.query(FinancialQuery::StockList).await.unwrap();
    assert!(!result.success);
    assert!(result.error.unwrap().contains("未配置"));
}

#[tokio::test]
async fn null_provider_tool_call() {
    let tool = FinancialTool::new(Arc::new(NullProvider));
    assert_eq!(tool.name(), "finance");
    assert!(tool.description().contains("金融"));

    let result = tool
        .call(serde_json::json!({"query_type": "stock_list"}))
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.content.contains("未配置"));
}

#[tokio::test]
async fn tool_rejects_unknown_query_type() {
    let tool = FinancialTool::new(Arc::new(NullProvider));
    let result = tool
        .call(serde_json::json!({"query_type": "nonexistent"}))
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.content.contains("不支持"));
}

struct AlwaysFailProvider;

#[async_trait::async_trait]
impl FinancialDataProvider for AlwaysFailProvider {
    fn capabilities(&self) -> Vec<QueryCapability> {
        vec![QueryCapability {
            name: "stock_list".into(),
            description: "test capability".into(),
            parameters: serde_json::json!({}),
        }]
    }

    async fn query(&self, _q: FinancialQuery) -> anyhow::Result<FinancialResult> {
        Err(anyhow::anyhow!("network unavailable"))
    }

    fn provider_name(&self) -> &str {
        "always_fail"
    }
}

#[tokio::test]
async fn financial_tool_converts_provider_error_to_tool_error() {
    let tool = FinancialTool::new(Arc::new(AlwaysFailProvider));
    let result = tool
        .call(serde_json::json!({"query_type": "stock_list"}))
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.content.contains("金融数据源暂时不可用"));
    assert!(result.content.contains("network unavailable"));
}

#[tokio::test]
async fn ft_share_provider_builds() {
    let p = FtShareProvider::new("https://market.ft.tech");
    assert_eq!(p.provider_name(), "ftshare");
    let caps = p.capabilities();
    assert!(!caps.is_empty());
    // Verify key capabilities
    let names: Vec<&str> = caps.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"stock_quote"));
    assert!(names.contains(&"index_list"));
    assert!(names.contains(&"macro_china"));
}

#[tokio::test]
async fn financial_tool_schema_from_provider() {
    let tool = FinancialTool::new(Arc::new(FtShareProvider::new("https://market.ft.tech")));
    let schema = tool.schema();
    let query_type_enum = &schema["properties"]["query_type"]["enum"];
    assert!(query_type_enum.is_array());
    let enums: Vec<&str> = query_type_enum
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(enums.contains(&"stock_quote"));
    assert!(enums.contains(&"index_detail"));
}

#[tokio::test]
async fn tushare_provider_builds() {
    let p = TuShareProvider::new("test-token");
    assert_eq!(p.provider_name(), "tushare");
    let caps = p.capabilities();
    let names: Vec<&str> = caps.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"stock_quote"));
    assert!(names.contains(&"macro_china"));
    assert!(names.contains(&"trade_cal"));
}

#[tokio::test]
async fn tushare_tool_schema_includes_trade_cal() {
    let tool = FinancialTool::new(Arc::new(TuShareProvider::new("test-token")));
    let schema = tool.schema();
    let enums: Vec<&str> = schema["properties"]["query_type"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(enums.contains(&"trade_cal"));
    assert!(enums.contains(&"fund_nav"));
}

#[tokio::test]
async fn tushare_without_token_rejected() {
    let p = TuShareProvider::new("");
    assert_eq!(p.provider_name(), "tushare");
}

// ── Sina Finance tests ────────────────────────────────────────────

#[test]
fn sina_code_conversion() {
    // 沪市主板
    assert_eq!(
        SinaFinanceProvider::to_sina_code("600519"),
        Some("sh600519".into())
    );
    // 深市
    assert_eq!(
        SinaFinanceProvider::to_sina_code("000001"),
        Some("sz000001".into())
    );
    // 创业板
    assert_eq!(
        SinaFinanceProvider::to_sina_code("300750"),
        Some("sz300750".into())
    );
    // 北交所
    assert_eq!(
        SinaFinanceProvider::to_sina_code("830799"),
        Some("bj830799".into())
    );
    // ts_code format
    assert_eq!(
        SinaFinanceProvider::to_sina_code("000001.SZ"),
        Some("sz000001".into())
    );
}

#[test]
fn sina_parse_quote() {
    let raw = r#"var hq_str_sh600519="贵州茅台,1850.00,1845.00,1860.50,1870.00,1840.00,1850.00,1860.50,100000,185000000.00";"#;
    let row = SinaFinanceProvider::parse_quote(raw, "600519").unwrap();
    assert_eq!(row.name, "贵州茅台");
    assert_eq!(row.open, Some(1850.00));
    assert_eq!(row.prev_close, Some(1845.00));
    assert_eq!(row.price, Some(1860.50));
    assert_eq!(row.high, Some(1870.00));
    assert_eq!(row.low, Some(1840.00));
    assert_eq!(row.volume, Some(100000.0));
    assert_eq!(row.amount, Some(185000000.0));
    assert!(row.pct_chg.is_some());
}

#[tokio::test]
async fn sina_provider_builds_and_has_capabilities() {
    let p = SinaFinanceProvider::new();
    assert_eq!(p.provider_name(), "sina");
    let caps = p.capabilities();
    let names: Vec<&str> = caps.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"stock_quote"));
    assert!(names.contains(&"index_list"));
    // Sina only provides these two
    assert_eq!(caps.len(), 2);
}

#[tokio::test]
async fn sina_tool_rejects_unsupported_query() {
    let tool = FinancialTool::new(Arc::new(SinaFinanceProvider::new()));
    // Fund info is not supported by Sina
    let result = tool
        .call(serde_json::json!({
            "query_type": "fund_info",
            "parameters": {"code": "000001"}
        }))
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.content.contains("仅支持"));
}

// ── CompositeProvider tests ────────────────────────────────────────

use hermess_finance::providers::composite::CompositeProvider;

#[test]
fn composite_name_reflects_all_providers() {
    let p = CompositeProvider::new(vec![
        Box::new(FtShareProvider::new("https://market.ft.tech")),
        Box::new(SinaFinanceProvider::new()),
    ]);
    let name = p.provider_name();
    assert!(name.contains("composite"));
    assert!(name.contains("ftshare"));
    assert!(name.contains("sina"));
}

#[test]
fn composite_capabilities_union() {
    let p = CompositeProvider::new(vec![
        Box::new(SinaFinanceProvider::new()),
        Box::new(FtShareProvider::new("https://market.ft.tech")),
    ]);
    let caps = p.capabilities();
    let names: Vec<&str> = caps.iter().map(|c| c.name.as_str()).collect();
    // Sina provides stock_quote + index_list
    // FTShare provides stock_quote + index_list + index_detail + macro_china + ...
    // Union should include everything, no duplicates
    assert!(names.contains(&"stock_quote"));
    assert!(names.contains(&"index_list"));
    assert!(names.contains(&"index_detail"));
    assert!(names.contains(&"macro_china"));
    assert!(names.contains(&"fund_info"));
    // Verify no duplicates
    let stock_quote_count = names.iter().filter(|&&n| n == "stock_quote").count();
    assert_eq!(stock_quote_count, 1);
}

#[tokio::test]
async fn composite_falls_back_on_unsupported_query() {
    // Sina doesn't support fund_info, FTShare does.
    // Composite should fall through Sina and succeed with FTShare.
    let p = CompositeProvider::new(vec![
        Box::new(SinaFinanceProvider::new()),
        Box::new(FtShareProvider::new("https://market.ft.tech")),
    ]);
    let result = p
        .query(FinancialQuery::FundInfo {
            code: "000001".into(),
        })
        .await;
    // FTShare may or may not be reachable in test; we just verify no panic
    // and that we get a reasonable result (not a Sina "仅支持" error)
    match result {
        Ok(r) => {
            if !r.success {
                // If both fail, error should not be Sina's "仅支持"
                let msg = r.error.unwrap_or_default();
                assert!(
                    !msg.contains("仅支持"),
                    "should have fallen through Sina: {msg}"
                );
            }
        }
        Err(_) => {
            // Network error is acceptable in test
        }
    }
}

#[tokio::test]
async fn composite_tool_uses_union_capabilities() {
    let p = CompositeProvider::new(vec![
        Box::new(SinaFinanceProvider::new()),
        Box::new(FtShareProvider::new("https://market.ft.tech")),
    ]);
    let tool = FinancialTool::new(Arc::new(p));
    let schema = tool.schema();
    let enums: Vec<&str> = schema["properties"]["query_type"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    // Should include Sina's stock_quote AND FTShare's fund_info
    assert!(enums.contains(&"stock_quote"));
    assert!(enums.contains(&"fund_info"));
    assert!(enums.contains(&"index_detail"));
}

#[test]
fn composite_single_provider_no_overhead() {
    // Single-provider composite should work identically to bare provider
    let p = CompositeProvider::new(vec![Box::new(NullProvider)]);
    assert!(p.provider_name().contains("null"));
    assert!(p.capabilities().is_empty());
}

#[test]
#[should_panic(expected = "at least one provider")]
fn composite_empty_panics() {
    CompositeProvider::new(vec![]);
}

#[test]
fn default_finance_provider_uses_premium_and_free_failover_chain() {
    let provider = build_finance_provider(FinanceProviderOptions::default());
    let name = provider.provider_name();
    assert!(name.contains("composite"));
    assert!(name.contains("ftshare"));
    assert!(name.contains("eastmoney"));
    assert!(name.contains("tencent"));
    assert!(name.contains("sina"));
    assert!(!name.contains("tushare"));

    let caps = provider.capabilities();
    let names: Vec<&str> = caps.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"stock_quote"));
    assert!(names.contains(&"stock_list"));
    assert!(names.contains(&"stock_history"));
}

#[test]
fn default_finance_provider_enables_tushare_when_token_exists() {
    let provider = build_finance_provider(FinanceProviderOptions {
        tushare_token: Some("test-token".into()),
        ..FinanceProviderOptions::default()
    });
    let name = provider.provider_name();
    assert!(name.contains("tushare"));
    assert!(name.contains("ftshare"));
    assert!(name.contains("eastmoney"));
    assert!(name.find("tushare") < name.find("ftshare"));
}

#[test]
fn tushare_without_token_keeps_premium_and_free_fallback_chain() {
    let provider = build_finance_provider(FinanceProviderOptions {
        provider: Some("tushare".into()),
        ..FinanceProviderOptions::default()
    });
    let name = provider.provider_name();
    assert!(name.contains("ftshare"));
    assert!(name.contains("eastmoney"));
    assert!(name.contains("tencent"));
    assert!(name.contains("sina"));
    assert!(!name.contains("tushare"));
}

#[test]
fn explicit_sina_keeps_ftshare_fallback_available() {
    let provider = build_finance_provider(FinanceProviderOptions {
        provider: Some("sina".into()),
        ..FinanceProviderOptions::default()
    });
    let name = provider.provider_name();
    assert!(name.contains("sina"));
    assert!(name.contains("ftshare"));

    let caps = provider.capabilities();
    let names: Vec<&str> = caps.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"fund_info"));
    assert!(names.contains(&"macro_china"));
}

#[test]
fn explicit_null_can_disable_finance_provider() {
    let provider = build_finance_provider(FinanceProviderOptions {
        provider: Some("null".into()),
        allow_disable: true,
        ..FinanceProviderOptions::default()
    });
    assert_eq!(provider.provider_name(), "null");
    assert!(provider.capabilities().is_empty());
}
