use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::{Mutex, OnceLock};

use crate::data::models::*;

pub struct Database {
    conn: Mutex<Connection>,
    /// 前端事件发射器（setup 阶段注入），日志变更时广播 logs-updated 驱动 UI 刷新
    emitter: OnceLock<tauri::AppHandle>,
}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        // 外部进程短暂锁库（备份工具/杀软扫描等）时等待重试而非立即失败，
        // 避免日志等写入被静默丢弃
        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
        let db = Database {
            conn: Mutex::new(conn),
            emitter: OnceLock::new(),
        };
        db.init()?;
        Ok(db)
    }

    /// 注入应用句柄（仅 setup 阶段调用一次）
    pub fn set_emitter(&self, app: tauri::AppHandle) {
        let _ = self.emitter.set(app);
    }

    /// 广播日志变更信号（日志页监听后静默刷新）
    fn notify_logs_changed(&self) {
        if let Some(app) = self.emitter.get() {
            use tauri::Emitter;
            let _ = app.emit("logs-updated", ());
        }
    }

    /// 将当前数据库一致性快照写入目标路径（backup API，可在连接打开时安全执行）
    pub fn backup_to(&self, dest: &std::path::Path) -> Result<()> {
        let src = self.conn.lock().unwrap();
        let mut dest_conn = Connection::open(dest)?;
        let backup = rusqlite::backup::Backup::new(&src, &mut dest_conn)?;
        backup.run_to_completion(64, std::time::Duration::from_millis(5), None)?;
        Ok(())
    }

    /// 从备份文件恢复到当前连接（运行中安全执行，覆盖全部表数据）
    pub fn restore_from(&self, src: &std::path::Path) -> Result<()> {
        let src_conn = Connection::open(src)?;
        let mut dst = self.conn.lock().unwrap();
        let backup = rusqlite::backup::Backup::new(&src_conn, &mut dst)?;
        backup.run_to_completion(64, std::time::Duration::from_millis(5), None)?;
        Ok(())
    }

    fn init(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY,
                platform TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT '',
                email TEXT NOT NULL DEFAULT '',
                access_token TEXT NOT NULL DEFAULT '',
                refresh_token TEXT NOT NULL DEFAULT '',
                cookies TEXT NOT NULL DEFAULT '',
                token_expires_at INTEGER NOT NULL DEFAULT 0,
                refresh_token_expires_at INTEGER NOT NULL DEFAULT 0,
                last_checkin_time INTEGER NOT NULL DEFAULT 0,
                checkin_status TEXT NOT NULL DEFAULT 'idle',
                fail_count INTEGER NOT NULL DEFAULT 0,
                cooldown_until INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS checkin_records (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                platform TEXT NOT NULL,
                status TEXT NOT NULL,
                points INTEGER NOT NULL DEFAULT 0,
                message TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS points_history (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                date TEXT NOT NULL,
                points INTEGER NOT NULL DEFAULT 0,
                source TEXT NOT NULL DEFAULT 'checkin',
                expires_at INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS app_logs (
                id TEXT PRIMARY KEY,
                level TEXT NOT NULL DEFAULT 'info',
                action TEXT NOT NULL DEFAULT '',
                message TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_checkin_account ON checkin_records(account_id);
            CREATE INDEX IF NOT EXISTS idx_points_account ON points_history(account_id);
            CREATE INDEX IF NOT EXISTS idx_points_date ON points_history(date);
            CREATE INDEX IF NOT EXISTS idx_logs_created ON app_logs(created_at);
            "#,
        )?;
        // 旧库迁移：补积分列（CREATE TABLE 只对新库生效）
        for (col, ddl) in [
            (
                "credits_balance",
                "ALTER TABLE accounts ADD COLUMN credits_balance REAL",
            ),
            (
                "credits_expiries",
                "ALTER TABLE accounts ADD COLUMN credits_expiries TEXT NOT NULL DEFAULT ''",
            ),
            (
                "credits_fetched_at",
                "ALTER TABLE accounts ADD COLUMN credits_fetched_at INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "avatar",
                "ALTER TABLE accounts ADD COLUMN avatar TEXT NOT NULL DEFAULT ''",
            ),
            (
                "uid",
                "ALTER TABLE accounts ADD COLUMN uid TEXT NOT NULL DEFAULT ''",
            ),
            (
                "domain",
                "ALTER TABLE accounts ADD COLUMN domain TEXT NOT NULL DEFAULT ''",
            ),
            (
                "enterprise_id",
                "ALTER TABLE accounts ADD COLUMN enterprise_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "account_type",
                "ALTER TABLE accounts ADD COLUMN account_type TEXT NOT NULL DEFAULT ''",
            ),
            (
                "device_id",
                "ALTER TABLE accounts ADD COLUMN device_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "device_key",
                "ALTER TABLE accounts ADD COLUMN device_key TEXT NOT NULL DEFAULT ''",
            ),
            (
                "machine_id",
                "ALTER TABLE accounts ADD COLUMN machine_id TEXT NOT NULL DEFAULT ''",
            ),
        ] {
            let exists: bool = conn
                .prepare("SELECT 1 FROM pragma_table_info('accounts') WHERE name=?1")?
                .exists(params![col])?;
            if !exists {
                conn.execute_batch(ddl)?;
            }
        }
        Ok(())
    }

    // ===== Accounts =====
    pub fn insert_account(&self, acc: &Account) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"INSERT INTO accounts (id, platform, name, email, avatar, uid, domain, enterprise_id, account_type, device_id,
                device_key, machine_id,
                access_token, refresh_token, cookies,
                token_expires_at, refresh_token_expires_at, last_checkin_time, checkin_status,
                fail_count, cooldown_until, created_at, updated_at)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)"#,
            params![
                acc.id, acc.platform, acc.name, acc.email, acc.avatar, acc.uid, acc.domain,
                acc.enterprise_id, acc.account_type, acc.device_id, acc.device_key, acc.machine_id,
                acc.access_token, acc.refresh_token, acc.cookies, acc.token_expires_at,
                acc.refresh_token_expires_at, acc.last_checkin_time, acc.checkin_status,
                acc.fail_count, acc.cooldown_until, acc.created_at, acc.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_account_tokens(
        &self,
        id: &str,
        access_token: &str,
        refresh_token: &str,
        token_expires_at: i64,
        refresh_token_expires_at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"UPDATE accounts SET access_token=?1, refresh_token=?2, token_expires_at=?3,
                refresh_token_expires_at=?4, updated_at=?5 WHERE id=?6"#,
            params![
                access_token,
                refresh_token,
                token_expires_at,
                refresh_token_expires_at,
                chrono::Utc::now().timestamp(),
                id
            ],
        )?;
        Ok(())
    }

    pub fn update_account_info(
        &self,
        id: &str,
        name: &str,
        email: &str,
        access_token: &str,
        refresh_token: &str,
        cookies: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"UPDATE accounts SET name=?1, email=?2, access_token=?3, refresh_token=?4,
                cookies=?5, updated_at=?6 WHERE id=?7"#,
            params![
                name,
                email,
                access_token,
                refresh_token,
                cookies,
                chrono::Utc::now().timestamp(),
                id
            ],
        )?;
        Ok(())
    }

    pub fn update_account_checkin(
        &self,
        id: &str,
        status: &str,
        last_checkin_time: i64,
        fail_count: i32,
        cooldown_until: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"UPDATE accounts SET checkin_status=?1, last_checkin_time=?2, fail_count=?3,
                cooldown_until=?4, updated_at=?5 WHERE id=?6"#,
            params![
                status,
                last_checkin_time,
                fail_count,
                cooldown_until,
                chrono::Utc::now().timestamp(),
                id
            ],
        )?;
        Ok(())
    }

    pub fn delete_account(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM accounts WHERE id=?1", params![id])?;
        conn.execute(
            "DELETE FROM checkin_records WHERE account_id=?1",
            params![id],
        )?;
        conn.execute(
            "DELETE FROM points_history WHERE account_id=?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn get_account(&self, id: &str) -> Result<Option<Account>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM accounts WHERE id=?1")?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_account(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM accounts ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], row_to_account)?;
        let mut accounts = Vec::new();
        for r in rows {
            accounts.push(r?);
        }
        Ok(accounts)
    }

    // ===== Checkin records =====
    pub fn insert_checkin_record(&self, rec: &CheckinRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"INSERT INTO checkin_records (id, account_id, platform, status, points, message, created_at)
                VALUES (?1,?2,?3,?4,?5,?6,?7)"#,
            params![rec.id, rec.account_id, rec.platform, rec.status, rec.points, rec.message, rec.created_at],
        )?;
        Ok(())
    }

    pub fn list_checkin_records(
        &self,
        account_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<CheckinRecord>> {
        let conn = self.conn.lock().unwrap();
        let sql = match account_id {
            Some(_) => "SELECT * FROM checkin_records WHERE account_id=?1 ORDER BY created_at DESC LIMIT ?2",
            None => "SELECT * FROM checkin_records ORDER BY created_at DESC LIMIT ?1",
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = match account_id {
            Some(aid) => stmt.query_map(params![aid, limit], row_to_record)?,
            None => stmt.query_map(params![limit], row_to_record)?,
        };
        let mut recs = Vec::new();
        for r in rows {
            recs.push(r?);
        }
        Ok(recs)
    }

    // ===== Points history =====
    pub fn insert_points_history(&self, p: &PointsHistory) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"INSERT OR REPLACE INTO points_history (id, account_id, date, points, source, expires_at, created_at)
                VALUES (?1,?2,?3,?4,?5,?6,?7)"#,
            params![p.id, p.account_id, p.date, p.points, p.source, p.expires_at, p.created_at],
        )?;
        Ok(())
    }

    pub fn get_total_points(&self, platform: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(ph.points),0) FROM points_history ph
             JOIN accounts a ON ph.account_id=a.id WHERE a.platform=?1",
            params![platform],
            |r| r.get(0),
        )?;
        Ok(total)
    }

    pub fn get_today_points(&self, today: &str, platform: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(ph.points),0) FROM points_history ph
             JOIN accounts a ON ph.account_id=a.id
             WHERE ph.date=?1 AND a.platform=?2",
            params![today, platform],
            |r| r.get(0),
        )?;
        Ok(total)
    }

    pub fn get_trend(&self, days: i64, platform: &str) -> Result<Vec<DailyPoints>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT ph.date, COALESCE(SUM(ph.points),0) as points FROM points_history ph
             JOIN accounts a ON ph.account_id=a.id
             WHERE ph.date >= date('now', ?1) AND a.platform=?2
             GROUP BY ph.date ORDER BY ph.date ASC",
        )?;
        let offset = format!("-{} days", days - 1);
        let rows = stmt.query_map(params![offset, platform], |r| {
            Ok(DailyPoints {
                date: r.get(0)?,
                points: r.get(1)?,
            })
        })?;
        let mut trend = Vec::new();
        for r in rows {
            trend.push(r?);
        }
        Ok(trend)
    }

    pub fn get_expiring_packages(
        &self,
        within_days: i64,
        platform: &str,
    ) -> Result<Vec<ExpiringPackage>> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        let limit = now + within_days * 86400;
        let mut stmt = conn.prepare(
            r#"SELECT a.name, a.platform, ph.points, ph.expires_at,
                CAST((ph.expires_at - ?1)/86400 AS INTEGER) as days_left
                FROM points_history ph JOIN accounts a ON ph.account_id=a.id
                WHERE ph.expires_at > 0 AND ph.expires_at <= ?2 AND ph.expires_at > ?1
                AND a.platform = ?3
                ORDER BY ph.expires_at ASC"#,
        )?;
        let rows = stmt.query_map(params![now, limit, platform], |r| {
            Ok(ExpiringPackage {
                account_name: r.get(0)?,
                platform: r.get(1)?,
                points: r.get(2)?,
                expires_at: r.get(3)?,
                days_left: r.get(4)?,
            })
        })?;
        let mut pkgs = Vec::new();
        for r in rows {
            pkgs.push(r?);
        }
        Ok(pkgs)
    }

    // ===== 真实积分 =====
    pub fn update_account_credits(
        &self,
        id: &str,
        balance: Option<f64>,
        expiries_json: &str,
        fetched_at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"UPDATE accounts SET credits_balance=?1, credits_expiries=?2,
                credits_fetched_at=?3, updated_at=?4 WHERE id=?5"#,
            params![
                balance,
                expiries_json,
                fetched_at,
                chrono::Utc::now().timestamp(),
                id
            ],
        )?;
        Ok(())
    }

    /// 更新 TRAE 设备号（仅用于补齐/规范化，不随签到重试轮换）
    pub fn update_account_device_id(&self, id: &str, device_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET device_id=?1, updated_at=?2 WHERE id=?3",
            params![device_id, chrono::Utc::now().timestamp(), id],
        )?;
        Ok(())
    }

    /// 重新授权时同步设备身份（设备号/设备密钥/机器码），并刷新凭据。
    /// 换证签发的 JWT 绑定该设备，所以重新登录必须把设备身份一起覆盖回库。
    pub fn update_account_device_identity(
        &self,
        id: &str,
        device_id: &str,
        device_key: &str,
        machine_id: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET device_id=?1, device_key=?2, machine_id=?3, updated_at=?4 WHERE id=?5",
            params![
                device_id,
                device_key,
                machine_id,
                chrono::Utc::now().timestamp(),
                id
            ],
        )?;
        Ok(())
    }

    /// 企业 ID 补拉落库（企业积分分流用）
    pub fn set_enterprise_id(&self, id: &str, enterprise_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET enterprise_id=?1, updated_at=?2 WHERE id=?3",
            params![enterprise_id, chrono::Utc::now().timestamp(), id],
        )?;
        Ok(())
    }

    /// 平台真实积分合计（仅含已查询过的账号）
    pub fn get_real_total_credits(&self, platform: &str) -> Result<f64> {
        let conn = self.conn.lock().unwrap();
        let total: f64 = conn.query_row(
            "SELECT COALESCE(SUM(credits_balance),0) FROM accounts
             WHERE platform=?1 AND credits_balance IS NOT NULL",
            params![platform],
            |r| r.get(0),
        )?;
        Ok(total)
    }

    /// 平台真实到期积分包（读 accounts.credits_expiries JSON）
    pub fn get_real_expiring_packages(
        &self,
        within_days: i64,
        platform: &str,
    ) -> Result<Vec<ExpiringPackage>> {
        let now = chrono::Utc::now().timestamp();
        let limit = now + within_days * 86400;
        let accounts = self.list_accounts()?;
        let mut pkgs = Vec::new();
        for a in accounts.into_iter().filter(|a| a.platform == platform) {
            let expiries: Vec<CreditExpiry> =
                serde_json::from_str(&a.credits_expiries).unwrap_or_default();
            for e in expiries {
                if e.at > now && e.at <= limit && e.amount > 0.0 {
                    pkgs.push(ExpiringPackage {
                        account_name: a.name.clone(),
                        platform: a.platform.clone(),
                        points: e.amount,
                        expires_at: e.at,
                        days_left: (e.at - now) / 86400,
                    });
                }
            }
        }
        pkgs.sort_by_key(|p| p.expires_at);
        Ok(pkgs)
    }

    // ===== 通用设置 =====
    pub fn load_app_settings(&self) -> AppSettings {
        match self.get_setting("app_settings") {
            Ok(Some(s)) => serde_json::from_str(&s).unwrap_or_default(),
            _ => AppSettings::default(),
        }
    }

    // ===== 系统日志 =====
    /// 写一条日志（顺带按设置清理过期旧日志；写入成功后广播 logs-updated）
    pub fn log(&self, level: &str, action: &str, message: &str) {
        let now = chrono::Utc::now().timestamp();
        let retention = self.load_app_settings().log_retention_days.max(1);
        let insert = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO app_logs (id, level, action, message, created_at) VALUES (?1,?2,?3,?4,?5)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    level,
                    action,
                    message,
                    now
                ],
            )
        };
        if let Err(e) = insert {
            // 写入失败不能无声吞掉：落 stderr 便于诊断日志丢失
            eprintln!("[db] 日志写入失败（{action} - {message}）: {e}");
            return;
        }
        {
            let conn = self.conn.lock().unwrap();
            let _ = conn.execute(
                "DELETE FROM app_logs WHERE created_at < ?1",
                params![now - retention * 86400],
            );
        }
        self.notify_logs_changed();
    }

    pub fn list_logs(&self, limit: i64) -> Result<Vec<AppLog>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, level, action, message, created_at FROM app_logs
             ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(AppLog {
                id: r.get(0)?,
                level: r.get(1)?,
                action: r.get(2)?,
                message: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        let mut logs = Vec::new();
        for r in rows {
            logs.push(r?);
        }
        Ok(logs)
    }

    pub fn clear_logs(&self) -> Result<()> {
        {
            let conn = self.conn.lock().unwrap();
            conn.execute("DELETE FROM app_logs", [])?;
        }
        self.notify_logs_changed();
        Ok(())
    }

    // ===== Settings =====
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key=?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }
}

fn row_to_account(r: &rusqlite::Row) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get("id")?,
        platform: r.get("platform")?,
        name: r.get("name")?,
        email: r.get("email")?,
        uid: r.get("uid").unwrap_or_default(),
        domain: r.get("domain").unwrap_or_default(),
        enterprise_id: r.get("enterprise_id").unwrap_or_default(),
        account_type: r.get("account_type").unwrap_or_default(),
        access_token: r.get("access_token")?,
        refresh_token: r.get("refresh_token")?,
        cookies: r.get("cookies")?,
        token_expires_at: r.get("token_expires_at")?,
        refresh_token_expires_at: r.get("refresh_token_expires_at")?,
        last_checkin_time: r.get("last_checkin_time")?,
        checkin_status: r.get("checkin_status")?,
        fail_count: r.get("fail_count")?,
        cooldown_until: r.get("cooldown_until")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        credits_balance: r.get("credits_balance")?,
        credits_expiries: r
            .get::<_, Option<String>>("credits_expiries")?
            .unwrap_or_default(),
        credits_fetched_at: r.get("credits_fetched_at")?,
        device_id: r.get("device_id").unwrap_or_default(),
        device_key: r.get("device_key").unwrap_or_default(),
        machine_id: r.get("machine_id").unwrap_or_default(),
        avatar: r.get("avatar")?,
    })
}

fn row_to_record(r: &rusqlite::Row) -> rusqlite::Result<CheckinRecord> {
    Ok(CheckinRecord {
        id: r.get(0)?,
        account_id: r.get(1)?,
        platform: r.get(2)?,
        status: r.get(3)?,
        points: r.get(4)?,
        message: r.get(5)?,
        created_at: r.get(6)?,
    })
}
