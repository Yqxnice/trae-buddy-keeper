//! 账号级并发控制：同一账号在同一时刻只允许一个上游请求（签到或积分查询）。
//!
//! 签到优先：签到侧等待账号锁后立即执行（最多等一个正在进行中的积分请求）；
//! 积分批次在检测到签到批量运行时主动让位，不再占用批次级互斥锁。

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

fn in_flight() -> &'static Mutex<HashSet<String>> {
    static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

fn lock_in_flight() -> std::sync::MutexGuard<'static, HashSet<String>> {
    in_flight().lock().unwrap_or_else(|e| e.into_inner())
}

/// 账号级互斥持有凭证：drop 时自动释放
pub struct AccountGuard {
    id: String,
}

impl Drop for AccountGuard {
    fn drop(&mut self) {
        lock_in_flight().remove(&self.id);
    }
}

/// 尝试获取账号级互斥；已被占用时返回 None（不等待）
pub fn try_acquire_account(id: &str) -> Option<AccountGuard> {
    let mut set = lock_in_flight();
    if !set.insert(id.to_string()) {
        return None;
    }
    Some(AccountGuard { id: id.to_string() })
}

/// 等待获取账号级互斥（签到侧使用，保证签到不被积分查询跳过）
pub async fn acquire_account(id: &str) -> AccountGuard {
    loop {
        if let Some(guard) = try_acquire_account(id) {
            return guard;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

