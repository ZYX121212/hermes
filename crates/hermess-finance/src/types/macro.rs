// Macroeconomic indicator types
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroIndicator {
    pub date: String,
    pub value: f64,
    pub unit: Option<String>,
    pub indicator: Option<String>,
}
