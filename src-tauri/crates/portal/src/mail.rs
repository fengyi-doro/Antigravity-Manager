use crate::{Error, Result};
use lettre::{
    message::Mailbox, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    AsyncTransport, Message, Tokio1Executor,
};
use std::time::Duration;

#[derive(Clone)]
pub struct MailConfig {
    pub host: String,
    pub port: u16,
    pub from: String,
    pub username: String,
    pub password: String,
    pub security: String,
}
impl MailConfig {
    pub fn from_env() -> Result<Option<Self>> {
        let Ok(host) = std::env::var("ABV_SMTP_HOST") else {
            return Ok(None);
        };
        let security = std::env::var("ABV_SMTP_SECURITY").unwrap_or_else(|_| "starttls".into());
        let config = Self {
            host,
            security: security.clone(),
            port: std::env::var("ABV_SMTP_PORT")
                .unwrap_or_else(|_| if security == "tls" { "465" } else { "587" }.into())
                .parse()
                .map_err(|_| Error::Invalid("SMTP 端口无效".into()))?,
            from: std::env::var("ABV_SMTP_FROM").unwrap_or_default(),
            username: std::env::var("ABV_SMTP_USERNAME").unwrap_or_default(),
            password: std::env::var("ABV_SMTP_PASSWORD").unwrap_or_default(),
        };
        config.validate()?;
        Ok(Some(config))
    }
    fn validate(&self) -> Result<()> {
        self.from
            .parse::<Mailbox>()
            .map_err(|_| Error::Invalid("SMTP 发件地址无效".into()))?;
        if self.host.is_empty()
            || !matches!(self.security.as_str(), "starttls" | "tls" | "local")
            || (self.security == "local" && !matches!(self.host.as_str(), "127.0.0.1" | "::1"))
            || self.username.is_empty() != self.password.is_empty()
        {
            return Err(Error::Invalid(
                "SMTP 配置无效；明文仅允许回环地址本地验收".into(),
            ));
        }
        Ok(())
    }
    pub async fn send(&self, to: &str, purpose: &str, token: &str) -> Result<()> {
        self.validate()?;
        let title = if purpose == "verify" {
            "邮箱验证"
        } else {
            "重置密码"
        };
        let message=Message::builder().from(self.from.parse().map_err(|_|Error::Invalid("发件地址无效".into()))?).to(to.parse().map_err(|_|Error::Invalid("收件地址无效".into()))?).subject(format!("Antigravity API {title}")).body(format!("{title}验证码（20 分钟有效，仅可使用一次）：\n\n{token}\n\n请回到用户中心粘贴验证码。若非本人操作，请忽略此邮件。")).map_err(|_|Error::Internal("构建邮件失败".into()))?;
        let builder = match self.security.as_str() {
            "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&self.host),
            "starttls" => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.host),
            _ => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
                &self.host,
            )),
        }
        .map_err(|_| Error::Invalid("SMTP 主机无效".into()))?
        .port(self.port)
        .timeout(Some(Duration::from_secs(15)));
        let builder = if self.username.is_empty() {
            builder
        } else {
            builder.credentials(Credentials::new(
                self.username.clone(),
                self.password.clone(),
            ))
        };
        tokio::time::timeout(Duration::from_secs(20), builder.build().send(message))
            .await
            .map_err(|_| Error::Unavailable("邮件发送超时，请稍后重试".into()))?
            .map_err(|_| Error::Unavailable("邮件发送失败，请联系管理员检查 SMTP".into()))?;
        Ok(())
    }
}
