// 发布版隐藏 Windows 控制台窗口（debug 运行时保留控制台以便查看日志）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    trae_buddy_keeper_lib::run();
}
