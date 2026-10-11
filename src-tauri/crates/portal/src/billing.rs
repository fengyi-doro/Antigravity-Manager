//! Prepaid CNY accounting for native user keys. Legacy token grants stay separate.
use crate::{
    db::{now, secret, Store},
    Error, Result,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};

pub const NANO_PER_YUAN: i64 = 1_000_000_000;
pub const INPUT_NANO: i64 = 600;
pub const CACHED_NANO: i64 = 60;
pub const OUTPUT_NANO: i64 = 3000;

pub fn pricing() -> Value {
    json!({"currency":"CNY","nano_per_yuan":NANO_PER_YUAN,
        "input_per_million":0.60,"cached_per_million":0.06,"output_per_million":3.00,
        "output_includes_reasoning":true})
}

impl Store {
    /// An idempotent administrator credit also opts an existing account into billing.
    pub fn credit_wallet(&self, user: &str, amount_cents: i64, reference: &str) -> Result<()> {
        if !(1..=100_000_000).contains(&amount_cents)
            || reference.is_empty()
            || reference.len() > 128
        {
            return Err(Error::Invalid("请输入有效充值金额和唯一流水号".into()));
        }
        let amount = amount_cents * (NANO_PER_YUAN / 100);
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1)",
            [user],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(Error::Invalid("用户不存在".into()));
        }
        if let Some(previous) = tx
            .query_row(
                "SELECT user_id,amount_nano FROM wallet_credits WHERE reference=?1",
                [reference],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        {
            return if previous == (user.into(), amount) {
                Ok(())
            } else {
                Err(Error::Conflict)
            };
        }
        tx.execute("INSERT OR IGNORE INTO wallets(user_id) VALUES(?1)", [user])?;
        tx.execute("INSERT INTO wallet_credits(reference,user_id,amount_nano,created_at) VALUES(?1,?2,?3,?4)",params![reference,user,amount,now()])?;
        tx.execute(
            "UPDATE wallets SET balance_nano=balance_nano+?1 WHERE user_id=?2",
            params![amount, user],
        )?;
        tx.execute(
            "INSERT INTO audit(action,subject,details,created_at) VALUES('wallet_credit',?1,?2,?3)",
            params![
                user,
                json!({"amount_cents":amount_cents,"reference":reference}).to_string(),
                now()
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Record admission before the upstream call. No wallet means a legacy unbilled key.
    pub fn begin_billing(&self, token: &str, model: &str) -> Result<Option<String>> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let account = tx.query_row("SELECT w.user_id,w.balance_nano,u.enabled FROM billing_token_owners n JOIN wallets w ON w.user_id=n.user_id JOIN users u ON u.id=n.user_id WHERE n.token_id=?1",[token],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,bool>(2)?))).optional()?;
        let Some((user, balance, enabled)) = account else {
            return Ok(None);
        };
        if !enabled {
            return Err(Error::Forbidden);
        }
        if balance <= 0 {
            return Err(Error::InsufficientBalance);
        }
        let review: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM billing_requests WHERE user_id=?1 AND state='review')",
            [&user],
            |r| r.get(0),
        )?;
        if review {
            return Err(Error::Unavailable(
                "存在待核对的调用费用，请联系管理员".into(),
            ));
        }
        let id = secret("bill_");
        tx.execute("INSERT INTO billing_requests(id,user_id,token_id,model,state,created_at) VALUES(?1,?2,?3,?4,'pending',?5)",params![id,user,token,model,now()])?;
        tx.commit()?;
        Ok(Some(id))
    }

    /// Usage is authoritative upstream usage: total input includes cached input;
    /// output already includes reasoning. A failed request never incurs a charge.
    pub fn settle_billing(
        &self,
        id: &str,
        status: u16,
        usage: Option<(u32, u32, u32)>,
    ) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (user, state): (String, String) = tx.query_row(
            "SELECT user_id,state FROM billing_requests WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if state == "settled" || state == "released" {
            return Ok(());
        }
        if !(200..300).contains(&status) {
            tx.execute("UPDATE billing_requests SET state='released',charged_nano=0,status=?1,settled_at=?2 WHERE id=?3",params![status,now(),id])?;
        } else if let Some((input, cached, output)) = usage {
            let cached = cached.min(input);
            let charge = i64::from(input - cached) * INPUT_NANO
                + i64::from(cached) * CACHED_NANO
                + i64::from(output) * OUTPUT_NANO;
            tx.execute(
                "UPDATE wallets SET balance_nano=balance_nano-?1 WHERE user_id=?2",
                params![charge, user],
            )?;
            tx.execute("UPDATE billing_requests SET state='settled',input_tokens=?1,cached_tokens=?2,output_tokens=?3,charged_nano=?4,status=?5,settled_at=?6 WHERE id=?7",params![input,cached,output,charge,status,now(),id])?;
        } else {
            // Never silently bill estimates or grant free successful calls with missing usage.
            tx.execute(
                "UPDATE billing_requests SET state='review',status=?1 WHERE id=?2",
                params![status, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn recover_billing(&self) -> Result<()> {
        self.connect()?.execute(
            "UPDATE billing_requests SET state='review' WHERE state='pending'",
            [],
        )?;
        Ok(())
    }

    pub fn wallet_dashboard(&self, user: &str) -> Result<Value> {
        let conn = self.connect()?;
        let balance = conn
            .query_row(
                "SELECT balance_nano FROM wallets WHERE user_id=?1",
                [user],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        let spent: i64 = conn.query_row(
            "SELECT COALESCE(SUM(charged_nano),0) FROM billing_requests WHERE user_id=?1",
            [user],
            |r| r.get(0),
        )?;
        let mut stmt=conn.prepare("SELECT id,model,input_tokens,cached_tokens,output_tokens,charged_nano,status,state,created_at FROM billing_requests WHERE user_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 30")?;
        let usage=stmt.query_map([user],|r|Ok(json!({"id":r.get::<_,String>(0)?,"model":r.get::<_,String>(1)?,"input_tokens":r.get::<_,Option<i64>>(2)?,"cached_tokens":r.get::<_,Option<i64>>(3)?,"output_tokens":r.get::<_,Option<i64>>(4)?,"charged_nano":r.get::<_,Option<i64>>(5)?,"status":r.get::<_,Option<u16>>(6)?,"state":r.get::<_,String>(7)?,"created_at":r.get::<_,i64>(8)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(
            json!({"enabled":balance.is_some(),"balance_nano":balance.unwrap_or(0),"spent_nano":spent,"pricing":pricing(),"usage":usage}),
        )
    }
}
