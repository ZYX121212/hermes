// NullProvider — default no-op that returns "未配置金融数据源"

use async_trait::async_trait;

use crate::{FinancialDataProvider, FinancialQuery, FinancialResult, QueryCapability};

pub struct NullProvider;

#[async_trait]
impl FinancialDataProvider for NullProvider {
    fn capabilities(&self) -> Vec<QueryCapability> {
        vec![]
    }

    async fn query(&self, _q: FinancialQuery) -> anyhow::Result<FinancialResult> {
        Ok(FinancialResult::err(
            "未配置金融数据源。请在配置文件中指定 provider（如 ftshare）以启用金融数据查询。",
        ))
    }

    fn provider_name(&self) -> &str {
        "null"
    }
}
