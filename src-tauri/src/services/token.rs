use anyhow::{anyhow, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::data::db::Database;
use crate::platform::trae::api as trae_api;

pub struct TokenManager {
    db: Arc<Database>,
    last_refresh: Arc<Mutex<HashMap<String, i64>>>,
    min_interval_secs: i64,
}

impl TokenManager {
    pub fn new(db: Arc<Database>) -> Self {
        TokenManager {
            db,
            last_refresh: Arc::new(Mutex::new(HashMap::new())),
            min_interval_secs: 300,
        }
    }

    async fn can_refresh(&self, account_id: &str) -> bool {
        let map = self.last_refresh.lock().await;
        match map.get(account_id) {
            Some(t) => Utc::now().timestamp() - t >= self.min_interval_secs,
            None => true,
        }
    }

    async fn mark_refreshed(&self, account_id: &str) {
        let mut map = self.last_refresh.lock().await;
        map.insert(account_id.to_string(), Utc::now().timestamp());
    }

    pub async fn auto_renew_if_needed(&self, account_id: &str) -> Result<Option<String>> {
        let acc = self
            .db
            .get_account(account_id)?
            .ok_or_else(|| anyhow!("账户不存在"))?;
        let now = Utc::now().timestamp();
        let threshold = 24 * 3600;
        if acc.token_expires_at == 0 || acc.token_expires_at - now > threshold {
            return Ok(None);
        }
        self.renew(account_id).await
    }

    pub async fn renew(&self, account_id: &str) -> Result<Option<String>> {
        let acc = self
            .db
            .get_account(account_id)?
            .ok_or_else(|| anyhow!("账户不存在"))?;

        if !self.can_refresh(account_id).await {
            return Err(anyhow!("操作过于频繁，请稍后再试"));
        }

        match acc.platform.as_str() {
            "trae" => {
                if acc.refresh_token.is_empty() {
                    return Err(anyhow!("无刷新凭证，无法自动续期"));
                }
                let (new_access, new_refresh, expires_at, _device_bound) =
                    trae_api::refresh_access_token(&acc).await?;
                self.db.update_account_tokens(
                    account_id,
                    &new_access,
                    &new_refresh,
                    expires_at,
                    acc.refresh_token_expires_at,
                )?;
                self.mark_refreshed(account_id).await;
                Ok(Some("续期成功".to_string()))
            }
            "workbuddy" => {
                let (new_access, new_refresh, expires_at) =
                    crate::platform::workbuddy::api::refresh_access_token(&acc).await?;
                self.db.update_account_tokens(
                    account_id,
                    &new_access,
                    &new_refresh,
                    expires_at,
                    acc.refresh_token_expires_at,
                )?;
                self.mark_refreshed(account_id).await;
                Ok(Some("续期成功".to_string()))
            }
            _ => Err(anyhow!("未知平台")),
        }
    }
}
