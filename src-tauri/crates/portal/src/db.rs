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

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
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
            1..=5 => (),
            _ => return Err(Error::Internal("Unsupported portal schema version".into())),
        }
        if version < 2 {
            tx.execute_batch(include_str!("model_settings.sql"))?;
        }
        if version < 3 {
            tx.execute_batch(include_str!("native_tokens.sql"))?;
        }
        if version < 4 {
            tx.execute_batch(include_str!("billing.sql"))?;
        }
        if version < 5 {
            tx.execute_batch(include_str!("account_services.sql"))?;
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
    pub(crate) fn connect(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        Ok(conn)
    }
    pub fn register(&self, email: &str, password: &str, time: i64) -> Result<User> {
        let email = email.trim().to_lowercase();
        if email.len() > 254
            || !email.contains('@')
            || email.starts_with('@')
            || email.ends_with('@')
            || email.chars().any(char::is_whitespace)
            || !(6..=128).contains(&password.len())
        {
            return Err(Error::Invalid(
                "请输入有效邮箱，密码长度为 6–128 字节".into(),
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
        tx.execute("INSERT INTO wallets(user_id) VALUES(?1)", [&user.id])?;
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
    pub fn users(&self) -> Result<Vec<User>> {
        let conn = self.connect()?;
        let mut stmt =
            conn.prepare("SELECT id,email,enabled,created_at FROM users ORDER BY created_at")?;
        let users = stmt
            .query_map([], |r| {
                Ok(User {
                    id: r.get(0)?,
                    email: r.get(1)?,
                    enabled: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(users)
    }
    pub fn native_token_id(&self, user: &str) -> Result<Option<String>> {
        Ok(self
            .connect()?
            .query_row(
                "SELECT token_id FROM native_token_links WHERE user_id=?1",
                [user],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn link_native_token(&self, user: &str, token: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO native_token_links(user_id,token_id) VALUES(?1,?2)
             ON CONFLICT(user_id) DO UPDATE SET token_id=excluded.token_id",
            params![user, token],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO billing_token_owners(token_id,user_id) VALUES(?1,?2)",
            params![token, user],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn legacy_dashboard(&self, user: &str) -> Result<Value> {
        let conn = self.connect()?;
        let (gift, paid, reserved): (i64, i64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(CASE WHEN kind='signup' THEN remaining-reserved ELSE 0 END),0),
            COALESCE(SUM(CASE WHEN kind='purchase' THEN remaining-reserved ELSE 0 END),0),
            COALESCE(SUM(reserved),0) FROM grants WHERE user_id=?1",
            params![user],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
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
            "available":(gift+paid-debt).max(0),"gift_expires_at":expiry,"usage":usage,"orders":orders}),
        )
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
        Ok(json!({"users":users,"billing_enabled":true}))
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
