//! Tauri 命令按业务域分组；lib.rs 只负责运行时装配与命令注册。

pub mod accounts;
pub mod checkin;
pub mod credits;
pub mod settings;
pub mod switch;
pub mod token;
