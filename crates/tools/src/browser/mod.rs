// crates/tools/src/browser/mod.rs
// Browser 工具集：基于 Chrome DevTools Protocol 的浏览器自动化。
use async_trait::async_trait;
use headless_chrome::{Browser, LaunchOptions, Tab};
use std::sync::Arc;
use std::time::Duration;

use crate::{Tool, ToolOutput};

/// Browser 工具共享状态：管理单个浏览器实例和 Tab。
pub struct BrowserState {
    #[allow(dead_code)]
    browser: Browser,
    tab: Arc<Tab>,
}

impl BrowserState {
    fn launch() -> anyhow::Result<Self> {
        let sandbox_enabled = std::env::var("HERMESS_BROWSER_SANDBOX")
            .map(|v| v != "0" && v.to_lowercase() != "false")
            .unwrap_or(true);
        if !sandbox_enabled {
            tracing::warn!("浏览器沙箱已禁用（HERMESS_BROWSER_SANDBOX=false），仅在 Docker 环境使用");
        }
        let opts = LaunchOptions::default_builder()
            .headless(true)
            .sandbox(sandbox_enabled)
            .window_size(Some((1920, 1080)))
            .build()?;

        let browser = Browser::new(opts)?;
        let tab = browser.new_tab()?;
        Ok(Self { browser, tab })
    }

    fn navigate(&self, url: &str) -> anyhow::Result<()> {
        self.tab.navigate_to(url)?;
        self.tab.wait_until_navigated()?;
        Ok(())
    }

    fn screenshot(&self, path: &str) -> anyhow::Result<Vec<u8>> {
        use headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption;
        let data = self.tab.capture_screenshot(
            CaptureScreenshotFormatOption::Png,
            None,
            None,
            true,
        )?;
        // 如果指定了路径，保存到文件
        if path != "-" {
            std::fs::write(path, &data)?;
        }
        Ok(data)
    }

    #[allow(dead_code)]
    fn get_content(&self) -> anyhow::Result<String> {
        let html = self.tab.get_content()?;
        // HTML → text
        Ok(html2text::from_read(html.as_bytes(), 80))
    }

    fn click(&self, selector: &str) -> anyhow::Result<()> {
        let elem = self.tab.wait_for_element(selector)?;
        elem.click()?;
        Ok(())
    }

    fn fill(&self, selector: &str, value: &str) -> anyhow::Result<()> {
        let elem = self.tab.wait_for_element(selector)?;
        elem.click()?;
        // Clear existing value by selecting all and typing
        elem.type_into(value)?;
        Ok(())
    }

    fn execute_js(&self, code: &str) -> anyhow::Result<String> {
        let result = self.tab.evaluate(code, false)?;
        Ok(format!("{result:?}"))
    }
}

// ===== Browser Navigate =====
pub struct BrowserNavigateTool {
    state: Arc<std::sync::Mutex<Option<BrowserState>>>,
    #[allow(dead_code)]
    timeout: Duration,
}

impl BrowserNavigateTool {
    pub fn new(timeout: Duration) -> Self {
        Self {
            state: Arc::new(std::sync::Mutex::new(None)),
            timeout,
        }
    }

    fn ensure_browser(&self) -> anyhow::Result<std::sync::MutexGuard<'_, Option<BrowserState>>> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(BrowserState::launch()?);
        }
        Ok(guard)
    }
}

#[async_trait]
impl Tool for BrowserNavigateTool {
    fn name(&self) -> &str {
        "browser_navigate"
    }

    fn description(&self) -> &str {
        "Open a URL in the headless browser. After navigating, you can use browser_screenshot or browser_get_content to see the page."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "The URL to navigate to"}
            },
            "required": ["url"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let url = args["url"].as_str()
            .ok_or_else(|| anyhow::anyhow!("browser_navigate: 'url' is required"))?;

        let guard = self.ensure_browser()?;
        let state = guard.as_ref().unwrap();
        state.navigate(url)?;
        let title = state.tab.get_title()?;
        Ok(ToolOutput::text(format!("Navigated to: {url}\nPage title: {title}")))
    }
}

// ===== Browser Screenshot =====
pub struct BrowserScreenshotTool {
    state: Arc<std::sync::Mutex<Option<BrowserState>>>,
}

impl BrowserScreenshotTool {
    pub fn new(browser_state: Arc<std::sync::Mutex<Option<BrowserState>>>) -> Self {
        Self { state: browser_state }
    }
}

#[async_trait]
impl Tool for BrowserScreenshotTool {
    fn name(&self) -> &str {
        "browser_screenshot"
    }

    fn description(&self) -> &str {
        "Take a screenshot of the current browser page. Specify an output file path. The screenshot is saved as PNG."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Output file path for the screenshot (PNG), or '-' for in-memory only"}
            },
            "required": ["path"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let path = args["path"].as_str().unwrap_or("screenshot.png");
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let state = guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("browser_screenshot: browser not launched. Use browser_navigate first."))?;

        let data = state.screenshot(path)?;
        Ok(ToolOutput {
            success: true,
            content: format!("Screenshot saved to {path} ({:.1} KB)", data.len() as f64 / 1024.0),
            metadata: serde_json::json!({"path": path, "size_bytes": data.len()}),
        })
    }
}

// ===== Browser Click =====
pub struct BrowserClickTool {
    state: Arc<std::sync::Mutex<Option<BrowserState>>>,
}

impl BrowserClickTool {
    pub fn new(browser_state: Arc<std::sync::Mutex<Option<BrowserState>>>) -> Self {
        Self { state: browser_state }
    }
}

#[async_trait]
impl Tool for BrowserClickTool {
    fn name(&self) -> &str {
        "browser_click"
    }

    fn description(&self) -> &str {
        "Click an element on the current page by CSS selector."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string", "description": "CSS selector of the element to click"}
            },
            "required": ["selector"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let selector = args["selector"].as_str()
            .ok_or_else(|| anyhow::anyhow!("browser_click: 'selector' is required"))?;
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let state = guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("browser_click: browser not launched."))?;
        state.click(selector)?;
        Ok(ToolOutput::text(format!("Clicked element: {selector}")))
    }
}

// ===== Browser Fill =====
pub struct BrowserFillTool {
    state: Arc<std::sync::Mutex<Option<BrowserState>>>,
}

impl BrowserFillTool {
    pub fn new(browser_state: Arc<std::sync::Mutex<Option<BrowserState>>>) -> Self {
        Self { state: browser_state }
    }
}

#[async_trait]
impl Tool for BrowserFillTool {
    fn name(&self) -> &str {
        "browser_fill"
    }

    fn description(&self) -> &str {
        "Fill a text input field on the current page by CSS selector."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string", "description": "CSS selector of the input field"},
                "value": {"type": "string", "description": "Text to type into the field"}
            },
            "required": ["selector", "value"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let selector = args["selector"].as_str()
            .ok_or_else(|| anyhow::anyhow!("browser_fill: 'selector' is required"))?;
        let value = args["value"].as_str()
            .ok_or_else(|| anyhow::anyhow!("browser_fill: 'value' is required"))?;
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let state = guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("browser_fill: browser not launched."))?;
        state.fill(selector, value)?;
        Ok(ToolOutput::text(format!("Filled {selector} with: {value}")))
    }
}

// ===== Browser Execute JS =====
pub struct BrowserExecuteTool {
    state: Arc<std::sync::Mutex<Option<BrowserState>>>,
}

impl BrowserExecuteTool {
    pub fn new(browser_state: Arc<std::sync::Mutex<Option<BrowserState>>>) -> Self {
        Self { state: browser_state }
    }
}

#[async_trait]
impl Tool for BrowserExecuteTool {
    fn name(&self) -> &str {
        "browser_execute"
    }

    fn description(&self) -> &str {
        "Execute JavaScript code on the current page and return the result."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "code": {"type": "string", "description": "JavaScript code to execute in the page context"}
            },
            "required": ["code"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let code = args["code"].as_str()
            .ok_or_else(|| anyhow::anyhow!("browser_execute: 'code' is required"))?;
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let state = guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("browser_execute: browser not launched."))?;
        let result = state.execute_js(code)?;
        Ok(ToolOutput::text(result))
    }
}

// Factory - creates all browser tools sharing one BrowserState
pub fn browser_toolset() -> Vec<Box<dyn Tool>> {
    let state: Arc<std::sync::Mutex<Option<BrowserState>>> = Arc::new(std::sync::Mutex::new(None));
    vec![
        Box::new(BrowserNavigateTool {
            state: state.clone(),
            timeout: Duration::from_secs(30),
        }),
        Box::new(BrowserScreenshotTool::new(state.clone())),
        Box::new(BrowserClickTool::new(state.clone())),
        Box::new(BrowserFillTool::new(state.clone())),
        Box::new(BrowserExecuteTool::new(state.clone())),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_navigate() {
        let tool = BrowserNavigateTool::new(Duration::from_secs(30));
        let s = tool.schema();
        assert!(s["required"].as_array().unwrap().contains(&serde_json::json!("url")));
    }

    #[test]
    fn test_schema_click() {
        let state = Arc::new(std::sync::Mutex::new(None));
        let tool = BrowserClickTool::new(state);
        assert_eq!(tool.name(), "browser_click");
    }

    #[test]
    fn test_schema_fill() {
        let state = Arc::new(std::sync::Mutex::new(None));
        let tool = BrowserFillTool::new(state);
        let s = tool.schema();
        assert!(s["required"].as_array().unwrap().contains(&serde_json::json!("selector")));
        assert!(s["required"].as_array().unwrap().contains(&serde_json::json!("value")));
    }
}
