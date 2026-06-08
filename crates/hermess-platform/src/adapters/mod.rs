// crates/hermess-platform/src/adapters/mod.rs
pub mod discord;
pub mod feishu;
pub mod slack;
pub mod telegram;
pub mod wechat;

pub use discord::{DiscordAdapter, DiscordConfig};
pub use feishu::{FeishuAdapter, FeishuConfig};
pub use slack::{SlackAdapter, SlackConfig};
pub use telegram::{TelegramAdapter, TelegramConfig};
pub use wechat::{WechatAdapter, WechatConfig};
