// Fund types
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundInfo {
    pub code: String,
    pub name: String,
    pub nav: Option<f64>,
    pub acc_nav: Option<f64>,
    pub fund_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundNavPoint {
    pub date: String,
    pub nav: f64,
    pub acc_nav: Option<f64>,
    pub pct_chg: Option<f64>,
}
