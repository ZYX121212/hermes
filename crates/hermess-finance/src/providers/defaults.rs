// Shared default financial data provider chain.

use std::sync::Arc;

use crate::FinancialDataProvider;

use super::{
    composite::CompositeProvider, eastmoney::EastMoneyProvider, ftshare::FtShareProvider,
    null::NullProvider, sina::SinaFinanceProvider, tencent::TencentFinanceProvider,
    tushare::TuShareProvider,
};

#[derive(Debug, Clone, Default)]
pub struct FinanceProviderOptions {
    pub provider: Option<String>,
    pub ftshare_url: Option<String>,
    pub tushare_token: Option<String>,
    pub allow_disable: bool,
}

pub fn build_finance_provider(options: FinanceProviderOptions) -> Arc<dyn FinancialDataProvider> {
    let provider = options
        .provider
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    let is_explicit = provider.is_some();
    let mut providers: Vec<Box<dyn FinancialDataProvider>> = Vec::new();

    match provider.as_deref() {
        Some("ftshare") => {
            let url = options
                .ftshare_url
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| "https://market.ft.tech".into());
            tracing::info!(%url, "Primary financial provider: FtShare");
            providers.push(Box::new(FtShareProvider::new(url)));
        }
        Some("tushare") => {
            let token = options
                .tushare_token
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .unwrap_or_default();
            if token.is_empty() {
                tracing::warn!("TuShare configured but no token set; using free providers");
            } else {
                tracing::info!("Primary financial provider: TuShare");
                providers.push(Box::new(TuShareProvider::new(token)));
            }
        }
        Some("eastmoney") => {
            tracing::info!("Primary financial provider: EastMoney");
            providers.push(Box::new(EastMoneyProvider::new()));
        }
        Some("tencent") => {
            tracing::info!("Primary financial provider: Tencent Finance");
            providers.push(Box::new(TencentFinanceProvider::new()));
        }
        Some("sina") => {
            tracing::info!("Primary financial provider: Sina Finance");
            providers.push(Box::new(SinaFinanceProvider::new()));
        }
        Some("none") | Some("null") if options.allow_disable => {
            tracing::info!("Financial data provider disabled by user");
            return Arc::new(NullProvider);
        }
        None | Some("none") | Some("null") => {}
        Some(other) => {
            tracing::warn!(
                provider = other,
                "Unknown financial provider; using free provider chain"
            );
        }
    }

    if providers.is_empty() {
        add_default_fallbacks(&options, &mut providers);
    } else if is_explicit {
        add_missing_default_fallbacks(&options, &mut providers);
    }

    if providers.len() == 1 {
        Arc::from(
            providers
                .into_iter()
                .next()
                .expect("providers has exactly one element"),
        )
    } else {
        let names: Vec<&str> = providers.iter().map(|p| p.provider_name()).collect();
        tracing::info!(
            ?names,
            "Composite financial provider with automatic failover"
        );
        Arc::new(CompositeProvider::new(providers).with_circuit_breaker(3, 30))
    }
}

fn add_default_fallbacks(
    options: &FinanceProviderOptions,
    providers: &mut Vec<Box<dyn FinancialDataProvider>>,
) {
    tracing::info!(
        "Using default financial providers: TuShare(token if set) -> FtShare -> EastMoney -> Tencent -> Sina"
    );
    add_missing_default_fallbacks(options, providers);
}

fn add_missing_default_fallbacks(
    options: &FinanceProviderOptions,
    providers: &mut Vec<Box<dyn FinancialDataProvider>>,
) {
    let has_tushare = providers.iter().any(|p| p.provider_name() == "tushare");
    let has_ftshare = providers.iter().any(|p| p.provider_name() == "ftshare");
    let has_eastmoney = providers.iter().any(|p| p.provider_name() == "eastmoney");
    let has_tencent = providers.iter().any(|p| p.provider_name() == "tencent");
    let has_sina = providers.iter().any(|p| p.provider_name() == "sina");

    if !has_tushare {
        if let Some(token) = options
            .tushare_token
            .as_ref()
            .filter(|s| !s.trim().is_empty())
        {
            providers.push(Box::new(TuShareProvider::new(token.clone())));
        } else {
            tracing::debug!("TuShare default provider skipped because no token is configured");
        }
    }
    if !has_ftshare {
        let url = options
            .ftshare_url
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "https://market.ft.tech".into());
        providers.push(Box::new(FtShareProvider::new(url)));
    }
    if !has_eastmoney {
        providers.push(Box::new(EastMoneyProvider::new()));
    }
    if !has_tencent {
        providers.push(Box::new(TencentFinanceProvider::new()));
    }
    if !has_sina {
        providers.push(Box::new(SinaFinanceProvider::new()));
    }
}
