use crate::{
    db::{digest, secret, Store},
    Error, Result,
};
use argon2::{
    password_hash::{PasswordHasher, SaltString},
    Argon2,
};
use rand::rngs::OsRng;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};

impl Store {
    pub fn require_verification(&self, user: &str) -> Result<()> {
        self.connect()?
            .execute("UPDATE users SET email_verified=0 WHERE id=?1", [user])?;
        Ok(())
    }
    pub fn email_verified(&self, user: &str) -> Result<bool> {
        Ok(self.connect()?.query_row(
            "SELECT email_verified FROM users WHERE id=?1",
            [user],
            |r| r.get(0),
        )?)
    }
    pub fn issue_challenge(
        &self,
        email: &str,
        purpose: &str,
        time: i64,
    ) -> Result<Option<(String, String)>> {
        if !matches!(purpose, "verify" | "reset") {
            return Err(Error::Invalid("无效的邮件用途".into()));
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let user = tx
            .query_row(
                "SELECT id,email FROM users WHERE email=?1 AND enabled=1",
                [email.trim().to_lowercase()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        tx.execute(
            "DELETE FROM account_challenges WHERE expires_at<=?1",
            [time],
        )?;
        let Some((user, email)) = user else {
            tx.commit()?;
            return Ok(None);
        };
        let token = secret("mail_");
        tx.execute(
            "DELETE FROM account_challenges WHERE user_id=?1 AND purpose=?2",
            params![user, purpose],
        )?;
        tx.execute(
            "INSERT INTO account_challenges VALUES(?1,?2,?3,?4)",
            params![digest(&token), user, purpose, time + 1200],
        )?;
        tx.commit()?;
        Ok(Some((email, token)))
    }
    pub fn invalidate_challenge(&self, token: &str) -> Result<()> {
        self.connect()?.execute(
            "DELETE FROM account_challenges WHERE digest=?1",
            [digest(token)],
        )?;
        Ok(())
    }
    pub fn consume_challenge(
        &self,
        token: &str,
        purpose: &str,
        bound_user: Option<&str>,
        password: Option<&str>,
        time: i64,
    ) -> Result<()> {
        if token.len() != 69
            || !token.starts_with("mail_")
            || !matches!(purpose, "verify" | "reset")
        {
            return Err(Error::Invalid("验证码无效或已过期".into()));
        }
        let hash = if purpose == "reset" {
            let password = password
                .filter(|p| (6..=128).contains(&p.len()))
                .ok_or_else(|| Error::Invalid("密码长度为 6–128 字节".into()))?;
            Some(
                Argon2::default()
                    .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
                    .map_err(|e| Error::Internal(e.to_string()))?
                    .to_string(),
            )
        } else {
            None
        };
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let user=tx.query_row("SELECT c.user_id FROM account_challenges c JOIN users u ON u.id=c.user_id WHERE c.digest=?1 AND c.purpose=?2 AND c.expires_at>?3 AND u.enabled=1",params![digest(token),purpose,time],|r|r.get::<_,String>(0)).optional()?.ok_or_else(||Error::Invalid("验证码无效或已过期".into()))?;
        if purpose == "verify" && bound_user != Some(user.as_str()) {
            return Err(Error::Forbidden);
        }
        if let Some(hash) = hash {
            tx.execute(
                "UPDATE users SET password_hash=?1,email_verified=1 WHERE id=?2",
                params![hash, user],
            )?;
            tx.execute("DELETE FROM sessions WHERE user_id=?1", [&user])?;
            tx.execute("DELETE FROM account_challenges WHERE user_id=?1", [&user])?;
        } else {
            tx.execute("UPDATE users SET email_verified=1 WHERE id=?1", [&user])?;
            tx.execute(
                "DELETE FROM account_challenges WHERE user_id=?1 AND purpose='verify'",
                [&user],
            )?;
        }
        tx.execute(
            "INSERT INTO audit(action,subject,details,created_at) VALUES(?1,?2,'{}',?3)",
            params![format!("email_{purpose}"), user, time],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// A request does not move money. Approval reserves funds by deducting the wallet atomically.
    pub fn request_refund(
        &self,
        user: &str,
        credit: &str,
        cents: i64,
        reason: &str,
        time: i64,
    ) -> Result<String> {
        if !(1..=100_000_000).contains(&cents) || reason.trim().is_empty() || reason.len() > 1000 {
            return Err(Error::Invalid("退款金额或原因无效".into()));
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let amount = cents * 10_000_000;
        let credited = tx
            .query_row(
                "SELECT amount_nano FROM wallet_credits WHERE reference=?1 AND user_id=?2",
                params![credit, user],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .ok_or(Error::Forbidden)?;
        let committed:i64=tx.query_row("SELECT COALESCE(SUM(amount_nano),0) FROM wallet_refunds WHERE credit_reference=?1 AND state!='cancelled'",[credit],|r|r.get(0))?;
        if amount > credited - committed {
            return Err(Error::Invalid("退款超过该笔充值的剩余可退金额".into()));
        }
        let id = secret("refund_");
        tx.execute("INSERT INTO wallet_refunds(id,user_id,credit_reference,amount_nano,state,reason,created_at,updated_at) VALUES(?1,?2,?3,?4,'requested',?5,?6,?6)",params![id,user,credit,amount,reason.trim(),time])?;
        tx.execute("INSERT INTO audit(action,subject,details,created_at) VALUES('refund_request',?1,?2,?3)",params![user,json!({"refund":id,"credit":credit,"amount_cents":cents}).to_string(),time])?;
        tx.commit()?;
        Ok(id)
    }
    pub fn transition_refund(
        &self,
        id: &str,
        action: &str,
        payment_reference: Option<&str>,
        time: i64,
    ) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (user,amount,state,previous):(String,i64,String,Option<String>)=tx.query_row("SELECT user_id,amount_nano,state,payment_reference FROM wallet_refunds WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.ok_or(Error::Forbidden)?;
        let target = match action {
            "approve" => "approved",
            "pay" => "paid",
            "cancel" => "cancelled",
            _ => return Err(Error::Invalid("无效的退款操作".into())),
        };
        let payment = if action == "pay" {
            Some(
                payment_reference
                    .filter(|s| !s.trim().is_empty() && s.len() <= 128)
                    .ok_or_else(|| Error::Invalid("填写已完成的线下退款流水号".into()))?
                    .trim(),
            )
        } else {
            None
        };
        if state == target {
            return if action != "pay" || previous.as_deref() == payment {
                Ok(())
            } else {
                Err(Error::Conflict)
            };
        }
        match (state.as_str(), action) {
            ("requested", "approve") => {
                let unresolved:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM billing_requests WHERE user_id=?1 AND state IN ('pending','review'))",[&user],|r|r.get(0))?;
                if unresolved {
                    return Err(Error::Invalid(
                        "先完成在途调用和待核对费用，再审核退款".into(),
                    ));
                }
                if tx.execute("UPDATE wallets SET balance_nano=balance_nano-?1 WHERE user_id=?2 AND balance_nano>=?1",params![amount,user])?!=1 {return Err(Error::InsufficientBalance);}
            }
            ("requested", "cancel") => (),
            ("approved", "cancel") => {
                tx.execute(
                    "UPDATE wallets SET balance_nano=balance_nano+?1 WHERE user_id=?2",
                    params![amount, user],
                )?;
            }
            ("approved", "pay") => {
                let used: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM wallet_refunds WHERE payment_reference=?1)",
                    [payment],
                    |r| r.get(0),
                )?;
                if used {
                    return Err(Error::Conflict);
                }
            }
            _ => return Err(Error::Conflict),
        }
        tx.execute(
            "UPDATE wallet_refunds SET state=?1,payment_reference=?2,updated_at=?3 WHERE id=?4",
            params![target, payment, time, id],
        )?;
        tx.execute(
            "INSERT INTO audit(action,subject,details,created_at) VALUES(?1,?2,?3,?4)",
            params![
                format!("refund_{action}"),
                user,
                json!({"refund":id,"amount_nano":amount,"payment_reference":payment}).to_string(),
                time
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn wallet_history(&self, user: &str) -> Result<Value> {
        let conn = self.connect()?;
        let mut credits=conn.prepare("SELECT c.reference,c.amount_nano,c.created_at,c.amount_nano-COALESCE((SELECT SUM(r.amount_nano) FROM wallet_refunds r WHERE r.credit_reference=c.reference AND r.state!='cancelled'),0) FROM wallet_credits c WHERE c.user_id=?1 ORDER BY c.created_at DESC,c.rowid DESC LIMIT 100")?;
        let credits=credits.query_map([user],|r|Ok(json!({"reference":r.get::<_,String>(0)?,"amount_nano":r.get::<_,i64>(1)?,"created_at":r.get::<_,i64>(2)?,"refundable_nano":r.get::<_,i64>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut refunds=conn.prepare("SELECT id,credit_reference,amount_nano,state,reason,payment_reference,created_at,updated_at FROM wallet_refunds WHERE user_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 100")?;
        let refunds=refunds.query_map([user],|r|Ok(json!({"id":r.get::<_,String>(0)?,"credit_reference":r.get::<_,String>(1)?,"amount_nano":r.get::<_,i64>(2)?,"state":r.get::<_,String>(3)?,"reason":r.get::<_,String>(4)?,"payment_reference":r.get::<_,Option<String>>(5)?,"created_at":r.get::<_,i64>(6)?,"updated_at":r.get::<_,i64>(7)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"credits":credits,"refunds":refunds}))
    }
}
