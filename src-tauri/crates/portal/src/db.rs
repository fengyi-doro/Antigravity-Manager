use crate::{Error, Result};
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Duration};

pub const SIGNUP_TOKENS: i64 = 5_000_000;
pub const TOKENS_PER_CENT: i64 = 10_000;
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub fn midnight_after(time: i64) -> i64 {
    (time + 28_800).div_euclid(86_400) * 86_400 + 86_400 - 28_800
}
pub fn secret(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!(
        "{}{}",
        prefix,
        bytes.iter().map(|v| format!("{v:02x}")).collect::<String>()
    )
}
pub fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}
#[derive(Clone, Debug, Serialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub enabled: bool,
    pub created_at: i64,
}
#[derive(Clone, Debug)]
pub struct Identity {
    pub user_id: String,
    pub key_id: String,
}
#[derive(Clone, Serialize)]
pub struct Order {
    pub id: String,
    pub user_id: String,
    pub amount_cents: i64,
    pub tokens: i64,
    pub channel: String,
    pub state: String,
    pub created_at: i64,
    pub paid_at: Option<i64>,
}

impl Store {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Internal(e.to_string()))?;
        }
        let store = Self { path };
        let mut conn = store.connect()?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        match version {
            0 => tx.execute_batch(include_str!("schema.sql"))?,
            1 | 2 => (),
            _ => return Err(Error::Internal("Unsupported portal schema version".into())),
        }
        if version < 2 {
            tx.execute_batch(include_str!("model_settings.sql"))?;
        }
        tx.commit()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&store.path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| Error::Internal(e.to_string()))?;
        }
        Ok(store)
    }
    fn connect(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        Ok(conn)
    }
    /// Saved administrator settings take precedence over the deployment default.
    pub fn allowed_models(&self) -> Result<Vec<String>> {
        let saved: Option<String> = self
            .connect()?
            .query_row(
                "SELECT value FROM portal_settings WHERE name='allowed_models'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let models = match saved {
            Some(value) => serde_json::from_str(&value)
                .map_err(|e| Error::Internal(format!("Invalid saved model settings: {e}")))?,
            None => std::env::var("ABV_PORTAL_ALLOWED_MODELS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_owned)
                .collect(),
        };
        normalize_allowed_models(models)
    }
    pub fn set_allowed_models(&self, models: Vec<String>) -> Result<Vec<String>> {
        let models = normalize_allowed_models(models)?;
        let value = serde_json::to_string(&models).map_err(|e| Error::Internal(e.to_string()))?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO portal_settings(name,value,updated_at) VALUES('allowed_models',?1,?2)
             ON CONFLICT(name) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
            params![value, now()],
        )?;
        tx.execute(
            "INSERT INTO audit(action,subject,details,created_at)
             VALUES('set_allowed_models','portal',?1,?2)",
            params![value, now()],
        )?;
        tx.commit()?;
        Ok(models)
    }
    pub fn register(&self, email: &str, password: &str, time: i64) -> Result<User> {
        let email = email.trim().to_lowercase();
        if email.len() > 254
            || !email.contains('@')
            || email.starts_with('@')
            || email.ends_with('@')
            || email.chars().any(char::is_whitespace)
            || !(10..=128).contains(&password.len())
        {
            return Err(Error::Invalid(
                "请输入有效邮箱，密码长度为 10–128 字节".into(),
            ));
        }
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map_err(|e| Error::Internal(e.to_string()))?
            .to_string();
        let user = User {
            id: secret("usr_"),
            email,
            enabled: true,
            created_at: time,
        };
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE email=?1)",
            [&user.email],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::Conflict);
        }
        tx.execute(
            "INSERT INTO users(id,email,password_hash,created_at) VALUES(?1,?2,?3,?4)",
            params![user.id, user.email, hash, time],
        )?;
        tx.execute(
            "INSERT INTO grants(id,user_id,source,kind,amount,remaining,expires_at,created_at)
            VALUES(?1,?2,?3,'signup',?4,?4,?5,?6)",
            params![
                secret("gr_"),
                user.id,
                format!("signup:{}", user.id),
                SIGNUP_TOKENS,
                midnight_after(time),
                time
            ],
        )?;
        tx.execute("INSERT INTO ledger(user_id,event,kind,delta,reference,created_at) VALUES(?1,?2,'signup',?3,?1,?4)",
            params![user.id, format!("signup:{}",user.id), SIGNUP_TOKENS,time])?;
        tx.commit()?;
        Ok(user)
    }
    pub fn login(&self, email: &str, password: &str) -> Result<User> {
        if password.len() > 128 {
            return Err(Error::Unauthorized);
        }
        let conn = self.connect()?;
        let record = conn
            .query_row(
                "SELECT id,email,enabled,created_at,password_hash FROM users WHERE email=?1",
                [email.trim().to_lowercase()],
                |r| {
                    Ok((
                        User {
                            id: r.get(0)?,
                            email: r.get(1)?,
                            enabled: r.get(2)?,
                            created_at: r.get(3)?,
                        },
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        // Unknown accounts still perform a password hash to avoid a fast enumeration path.
        let Some((user, hash)) = record else {
            let _ = Argon2::default()
                .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng));
            return Err(Error::Unauthorized);
        };
        let parsed = PasswordHash::new(&hash).map_err(|_| Error::Unauthorized)?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| Error::Unauthorized)?;
        if !user.enabled {
            return Err(Error::Forbidden);
        }
        Ok(user)
    }
    pub fn create_session(&self, user: &str, time: i64) -> Result<(String, String)> {
        let token = secret("sess_");
        let csrf = secret("");
        let conn = self.connect()?;
        conn.execute("DELETE FROM sessions WHERE expires_at<=?1", [time])?;
        conn.execute(
            "INSERT INTO sessions(digest,user_id,csrf,expires_at) VALUES(?1,?2,?3,?4)",
            params![digest(&token), user, csrf, time + 7 * 86_400],
        )?;
        Ok((token, csrf))
    }
    pub fn session(&self, token: &str, time: i64) -> Result<(User, String)> {
        self.connect()?
            .query_row(
                "SELECT u.id,u.email,u.enabled,u.created_at,s.csrf FROM sessions s
            JOIN users u ON u.id=s.user_id WHERE s.digest=?1 AND s.expires_at>?2 AND u.enabled=1",
                params![digest(token), time],
                |r| {
                    Ok((
                        User {
                            id: r.get(0)?,
                            email: r.get(1)?,
                            enabled: r.get(2)?,
                            created_at: r.get(3)?,
                        },
                        r.get(4)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::Unauthorized)
    }
    pub fn logout(&self, token: &str) -> Result<()> {
        self.connect()?
            .execute("DELETE FROM sessions WHERE digest=?1", [digest(token)])?;
        Ok(())
    }
    pub fn rotate_key(&self, user: &str, time: i64) -> Result<String> {
        let token = secret("sk-portal-");
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE api_keys SET enabled=0 WHERE user_id=?1", [user])?;
        tx.execute(
            "INSERT INTO api_keys(id,user_id,digest,suffix,created_at) VALUES(?1,?2,?3,?4,?5)",
            params![
                secret("key_"),
                user,
                digest(&token),
                &token[token.len() - 8..],
                time
            ],
        )?;
        tx.commit()?;
        Ok(token)
    }
    pub fn revoke_keys(&self, user: &str) -> Result<()> {
        self.connect()?
            .execute("UPDATE api_keys SET enabled=0 WHERE user_id=?1", [user])?;
        Ok(())
    }
    pub fn key_identity(&self, token: &str) -> Result<Identity> {
        self.connect()?
            .query_row(
                "SELECT k.user_id,k.id FROM api_keys k JOIN users u ON u.id=k.user_id
            WHERE k.digest=?1 AND k.enabled=1 AND u.enabled=1",
                [digest(token)],
                |r| {
                    Ok(Identity {
                        user_id: r.get(0)?,
                        key_id: r.get(1)?,
                    })
                },
            )
            .optional()?
            .ok_or(Error::Unauthorized)
    }
    pub fn dashboard(&self, user: &str, time: i64) -> Result<Value> {
        self.expire(user, time)?;
        let conn = self.connect()?;
        let (gift,paid,reserved):(i64,i64,i64)=conn.query_row(
            "SELECT COALESCE(SUM(CASE WHEN kind='signup' AND expires_at>?2 THEN remaining-reserved ELSE 0 END),0),
            COALESCE(SUM(CASE WHEN kind='purchase' THEN remaining-reserved ELSE 0 END),0),
            COALESCE(SUM(reserved),0) FROM grants WHERE user_id=?1",params![user,time],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let debt: i64 = conn.query_row(
            "SELECT COALESCE(SUM(debt),0) FROM requests WHERE user_id=?1",
            [user],
            |r| r.get(0),
        )?;
        let expiry: Option<i64> = conn
            .query_row(
                "SELECT expires_at FROM grants WHERE user_id=?1 AND kind='signup'",
                [user],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let key=conn.query_row("SELECT suffix,created_at FROM api_keys WHERE user_id=?1 AND enabled=1 ORDER BY created_at DESC LIMIT 1",[user],
            |r|Ok(json!({"suffix":r.get::<_,String>(0)?,"created_at":r.get::<_,i64>(1)?}))).optional()?;
        let mut stmt=conn.prepare("SELECT id,model,input_tokens,output_tokens,charged,state,created_at FROM requests WHERE user_id=?1 ORDER BY created_at DESC LIMIT 30")?;
        let usage=stmt.query_map([user],|r|Ok(json!({"id":r.get::<_,String>(0)?,"model":r.get::<_,String>(1)?,
            "input_tokens":r.get::<_,Option<i64>>(2)?,"output_tokens":r.get::<_,Option<i64>>(3)?,
            "charged":r.get::<_,Option<i64>>(4)?,"state":r.get::<_,String>(5)?,"created_at":r.get::<_,i64>(6)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut stmt=conn.prepare("SELECT id,user_id,amount_cents,tokens,channel,state,created_at,paid_at FROM orders WHERE user_id=?1 ORDER BY created_at DESC LIMIT 30")?;
        let orders = stmt
            .query_map([user], read_order)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(
            json!({"gift_available":gift,"paid_available":paid,"reserved":reserved,"debt":debt,
            "available":(gift+paid-debt).max(0),"gift_expires_at":expiry,"key":key,"usage":usage,"orders":orders}),
        )
    }
    pub fn reserve(
        &self,
        identity: &Identity,
        model: &str,
        amount: i64,
        time: i64,
    ) -> Result<String> {
        if !(1..=20_000_000).contains(&amount) {
            return Err(Error::Invalid("请求预算超出范围".into()));
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_grants(&tx, &identity.user_id, time)?;
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM api_keys k JOIN users u ON k.user_id=u.id WHERE k.id=?1 AND k.user_id=?2 AND k.enabled=1 AND u.enabled=1)",
            params![identity.key_id,identity.user_id],|r|r.get(0))?;
        if !valid {
            return Err(Error::Unauthorized);
        }
        let debt: i64 = tx.query_row(
            "SELECT COALESCE(SUM(debt),0) FROM requests WHERE user_id=?1",
            [&identity.user_id],
            |r| r.get(0),
        )?;
        let mut stmt=tx.prepare("SELECT id,remaining-reserved FROM grants WHERE user_id=?1 AND (expires_at IS NULL OR expires_at>?2)
            AND remaining>reserved ORDER BY CASE WHEN expires_at IS NULL THEN 1 ELSE 0 END,expires_at,created_at")?;
        let grants = stmt
            .query_map(params![identity.user_id, time], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        if grants.iter().map(|g| g.1).sum::<i64>() - debt < amount {
            return Err(Error::Quota);
        }
        let id = secret("req_");
        tx.execute("INSERT INTO requests(id,user_id,key_id,model,reserved,state,created_at) VALUES(?1,?2,?3,?4,?5,'pending',?6)",
            params![id,identity.user_id,identity.key_id,model,amount,time])?;
        let mut remaining = amount;
        for (grant, available) in grants {
            let take = remaining.min(available);
            if take == 0 {
                break;
            }
            tx.execute(
                "UPDATE grants SET reserved=reserved+?1 WHERE id=?2",
                params![take, grant],
            )?;
            tx.execute(
                "INSERT INTO allocations(request_id,grant_id,amount) VALUES(?1,?2,?3)",
                params![id, grant, take],
            )?;
            remaining -= take;
        }
        tx.commit()?;
        Ok(id)
    }
    /// Exactly-once settlement. An overrun remains visible as debt and blocks new spending.
    pub fn settle(&self, id: &str, input: i64, output: i64, time: i64) -> Result<()> {
        self.settle_impl(id, input, output, time, None)
    }
    pub fn manual_settle(
        &self,
        id: &str,
        input: i64,
        output: i64,
        time: i64,
        reason: &str,
    ) -> Result<()> {
        self.settle_impl(id, input, output, time, Some(reason))
    }
    fn settle_impl(
        &self,
        id: &str,
        input: i64,
        output: i64,
        time: i64,
        reason: Option<&str>,
    ) -> Result<()> {
        if input < 0 || output < 0 {
            return Err(Error::Invalid("用量不能为负".into()));
        }
        let total = input
            .checked_add(output)
            .ok_or_else(|| Error::Invalid("用量超出范围".into()))?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (user, state): (String, String) = tx.query_row(
            "SELECT user_id,state FROM requests WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if reason.is_some() && state != "review" {
            return Err(Error::Conflict);
        }
        if state == "settled" || state == "released" {
            return Ok(());
        }
        if let Some(reason) = reason {
            tx.execute("INSERT INTO audit(action,subject,details,created_at) VALUES('manual_settlement',?1,?2,?3)",
                params![id,json!({"input":input,"output":output,"reason":reason}).to_string(),time])?;
        }
        let mut stmt = tx.prepare(
            "SELECT grant_id,amount FROM allocations WHERE request_id=?1 ORDER BY rowid",
        )?;
        let allocations = stmt
            .query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        let mut due = total;
        for (grant, reserved) in allocations {
            let take = due.min(reserved);
            tx.execute(
                "UPDATE grants SET remaining=remaining-?1,reserved=reserved-?2 WHERE id=?3",
                params![take, reserved, grant],
            )?;
            due -= take;
        }
        if due > 0 {
            let mut stmt=tx.prepare("SELECT id,remaining-reserved FROM grants WHERE user_id=?1 AND (expires_at IS NULL OR expires_at>?2) AND remaining>reserved ORDER BY expires_at IS NULL,expires_at")?;
            let extra = stmt
                .query_map(params![user, time], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            drop(stmt);
            for (grant, available) in extra {
                let take = due.min(available);
                tx.execute(
                    "UPDATE grants SET remaining=remaining-?1 WHERE id=?2",
                    params![take, grant],
                )?;
                due -= take;
                if due == 0 {
                    break;
                }
            }
        }
        tx.execute("UPDATE requests SET state=?1,input_tokens=?2,output_tokens=?3,charged=?4,debt=?5,settled_at=?6 WHERE id=?7",
            params![if total==0 {"released"}else{"settled"},input,output,total,due,time,id])?;
        tx.execute("INSERT INTO ledger(user_id,event,kind,delta,reference,created_at) VALUES(?1,?2,'usage',?3,?4,?5)",
            params![user,format!("usage:{id}"),-total,id,time])?;
        expire_grants(&tx, &user, time)?;
        tx.commit()?;
        Ok(())
    }
    pub fn expire(&self, user: &str, time: i64) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_grants(&tx, user, time)?;
        tx.commit()?;
        Ok(())
    }
    pub fn mark_review(&self, id: &str) -> Result<()> {
        self.connect()?.execute(
            "UPDATE requests SET state='review' WHERE id=?1 AND state='pending'",
            [id],
        )?;
        Ok(())
    }
    pub fn recover_pending(&self) -> Result<()> {
        self.connect()?.execute(
            "UPDATE requests SET state='review' WHERE state='pending'",
            [],
        )?;
        Ok(())
    }
    pub fn order(&self, id: &str, user: &str) -> Result<Order> {
        self.connect()?.query_row("SELECT id,user_id,amount_cents,tokens,channel,state,created_at,paid_at FROM orders WHERE id=?1 AND user_id=?2",
            params![id,user],read_order).optional()?.ok_or(Error::Forbidden)
    }
    pub fn create_order(
        &self,
        user: &str,
        cents: i64,
        channel: &str,
        idem: &str,
        time: i64,
    ) -> Result<Order> {
        if ![100, 1_000, 5_000, 10_000].contains(&cents)
            || !["wxpay", "alipay"].contains(&channel)
            || idem.len() < 16
            || idem.len() > 100
        {
            return Err(Error::Invalid("请选择有效充值套餐".into()));
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(order)=tx.query_row("SELECT id,user_id,amount_cents,tokens,channel,state,created_at,paid_at FROM orders WHERE user_id=?1 AND idempotency_key=?2",
            params![user,idem],read_order).optional()? {
            if order.amount_cents!=cents || order.channel!=channel {return Err(Error::Conflict);}
            return Ok(order);
        }
        let id = format!("P{}", &secret("")[..31]);
        tx.execute("INSERT INTO orders(id,user_id,idempotency_key,amount_cents,tokens,channel,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![id,user,idem,cents,cents*TOKENS_PER_CENT,channel,time])?;
        tx.commit()?;
        self.order(&id, user)
    }
    /// Called only with a verified provider response; order + credit + event are one transaction.
    pub fn credit_order(
        &self,
        id: &str,
        trade: &str,
        cents: i64,
        channel: &str,
        time: i64,
    ) -> Result<()> {
        if trade.is_empty() || trade.len() > 128 {
            return Err(Error::Invalid("无效支付流水号".into()));
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let order=tx.query_row("SELECT id,user_id,amount_cents,tokens,channel,state,created_at,paid_at FROM orders WHERE id=?1",
            [id],read_order).optional()?.ok_or(Error::Forbidden)?;
        if cents != order.amount_cents || channel != order.channel {
            return Err(Error::Forbidden);
        }
        if order.state == "paid" {
            let recorded: String =
                tx.query_row("SELECT trade_no FROM orders WHERE id=?1", [id], |r| {
                    r.get(0)
                })?;
            return if recorded == trade {
                Ok(())
            } else {
                Err(Error::Conflict)
            };
        }
        if order.state != "pending" {
            return Err(Error::Conflict);
        }
        tx.execute("INSERT INTO payment_events(trade_no,order_id,amount_cents,received_at) VALUES(?1,?2,?3,?4)",
            params![trade,id,cents,time])?;
        let mut left = order.tokens;
        let mut stmt = tx.prepare(
            "SELECT id,debt FROM requests WHERE user_id=?1 AND debt>0 ORDER BY created_at",
        )?;
        let debts = stmt
            .query_map([&order.user_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        for (request, debt) in debts {
            let paid = left.min(debt);
            tx.execute(
                "UPDATE requests SET debt=debt-?1 WHERE id=?2",
                params![paid, request],
            )?;
            left -= paid;
            if left == 0 {
                break;
            }
        }
        tx.execute("INSERT INTO grants(id,user_id,source,kind,amount,remaining,created_at) VALUES(?1,?2,?3,'purchase',?4,?5,?6)",
            params![secret("gr_"),order.user_id,format!("order:{id}"),order.tokens,left,time])?;
        tx.execute("INSERT INTO ledger(user_id,event,kind,delta,reference,created_at) VALUES(?1,?2,'purchase',?3,?4,?5)",
            params![order.user_id,format!("order:{id}"),order.tokens,id,time])?;
        tx.execute(
            "UPDATE orders SET state='paid',trade_no=?1,paid_at=?2 WHERE id=?3",
            params![trade, time, id],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn rate_limit(&self, bucket: &str, limit: i64, window: i64, time: i64) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM rate_limits WHERE expires_at<=?1", [time])?;
        tx.execute(
            "INSERT INTO rate_limits(bucket,count,expires_at) VALUES(?1,1,?2)
            ON CONFLICT(bucket) DO UPDATE SET count=count+1",
            params![bucket, time + window],
        )?;
        let count: i64 = tx.query_row(
            "SELECT count FROM rate_limits WHERE bucket=?1",
            [bucket],
            |r| r.get(0),
        )?;
        tx.commit()?;
        if count > limit {
            return Err(Error::Unavailable("操作过于频繁，请稍后再试".into()));
        }
        Ok(())
    }
    pub fn admin_users(&self) -> Result<Value> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id,email,enabled,created_at FROM users ORDER BY created_at DESC LIMIT 200",
        )?;
        let users=stmt.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"email":r.get::<_,String>(1)?,"enabled":r.get::<_,bool>(2)?,"created_at":r.get::<_,i64>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut stmt=conn.prepare("SELECT id,user_id,model,reserved,created_at FROM requests WHERE state='review' ORDER BY created_at LIMIT 200")?;
        let review=stmt.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"user_id":r.get::<_,String>(1)?,"model":r.get::<_,String>(2)?,"reserved":r.get::<_,i64>(3)?,"created_at":r.get::<_,i64>(4)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(json!({"users":users,"review":review}))
    }
    pub fn set_enabled(&self, user: &str, enabled: bool) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.execute(
            "UPDATE users SET enabled=?1 WHERE id=?2",
            params![enabled, user],
        )? != 1
        {
            return Err(Error::Invalid("用户不存在".into()));
        }
        if !enabled {
            tx.execute("DELETE FROM sessions WHERE user_id=?1", [user])?;
        }
        tx.execute(
            "INSERT INTO audit(action,subject,details,created_at) VALUES('set_enabled',?1,?2,?3)",
            params![user, enabled.to_string(), now()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn audit(&self, action: &str, subject: &str, details: &str) -> Result<()> {
        self.connect()?.execute(
            "INSERT INTO audit(action,subject,details,created_at) VALUES(?1,?2,?3,?4)",
            params![action, subject, details, now()],
        )?;
        Ok(())
    }
}
fn normalize_allowed_models(models: Vec<String>) -> Result<Vec<String>> {
    if models.len() > 200 {
        return Err(Error::Invalid("最多开放 200 个模型".into()));
    }
    let mut normalized = Vec::new();
    for model in models {
        let model = model.trim();
        if model.is_empty()
            || model.len() > 200
            || !model
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.:/".contains(&c))
        {
            return Err(Error::Invalid(
                "模型 ID 须为 1–200 个字符，仅支持英文字母、数字及 - _ . : /，不支持通配符".into(),
            ));
        }
        if !normalized.iter().any(|saved| saved == model) {
            normalized.push(model.to_owned());
        }
    }
    Ok(normalized)
}
fn read_order(r: &rusqlite::Row<'_>) -> rusqlite::Result<Order> {
    Ok(Order {
        id: r.get(0)?,
        user_id: r.get(1)?,
        amount_cents: r.get(2)?,
        tokens: r.get(3)?,
        channel: r.get(4)?,
        state: r.get(5)?,
        created_at: r.get(6)?,
        paid_at: r.get(7)?,
    })
}

fn expire_grants(tx: &rusqlite::Transaction<'_>, user: &str, time: i64) -> Result<()> {
    let mut stmt=tx.prepare("SELECT id,remaining,reserved FROM grants WHERE user_id=?1 AND expires_at<=?2 AND remaining>reserved")?;
    let expired = stmt
        .query_map(params![user, time], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(stmt);
    for (id, remaining, reserved) in expired {
        tx.execute("UPDATE grants SET remaining=reserved WHERE id=?1", [&id])?;
        tx.execute("INSERT INTO ledger(user_id,event,kind,delta,reference,created_at) VALUES(?1,?2,'expired',?3,?4,?5)",
            params![user,format!("expire:{id}:{remaining}"),reserved-remaining,id,time])?;
    }
    Ok(())
}
