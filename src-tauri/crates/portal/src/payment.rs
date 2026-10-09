//! EPay V1 compatibility adapter. The merchant selects their HTTPS gateway;
//! no default payment operator or client-supplied amount is trusted.
//! Protocol: https://pay.xr876.cn/doc/v1_legacy_api.html
use crate::{
    db::{Order, Store},
    Error, Result,
};
use md5::{Digest, Md5};
use serde_json::{json, Value};
use std::{collections::BTreeMap, time::Duration};
use subtle::ConstantTimeEq;
use url::Url;

#[derive(Clone)]
pub struct Payment {
    pub gateway: Url,
    pub merchant: String,
    key: String,
    public_url: String,
}
pub fn cents(value: &str) -> Result<i64> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.is_empty()
        || parts.len() > 2
        || parts[0].is_empty()
        || !parts[0].bytes().all(|b| b.is_ascii_digit())
        || (parts.len() == 2
            && (parts[1].is_empty()
                || parts[1].len() > 2
                || !parts[1].bytes().all(|b| b.is_ascii_digit())))
    {
        return Err(Error::Invalid("支付金额格式错误".into()));
    }
    let whole = parts[0]
        .parse::<i64>()
        .map_err(|_| Error::Invalid("支付金额超出范围".into()))?;
    let fractional = if parts.len() == 1 {
        0
    } else {
        parts[1].parse::<i64>().unwrap_or(0) * if parts[1].len() == 1 { 10 } else { 1 }
    };
    whole
        .checked_mul(100)
        .and_then(|v| v.checked_add(fractional))
        .ok_or_else(|| Error::Invalid("支付金额超出范围".into()))
}
pub fn signature(fields: &BTreeMap<String, String>, key: &str) -> String {
    let mut text = fields
        .iter()
        .filter(|(k, v)| k.as_str() != "sign" && k.as_str() != "sign_type" && !v.is_empty())
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    text.push_str(key);
    format!("{:x}", Md5::digest(text.as_bytes()))
}
impl Payment {
    pub fn new(gateway: &str, merchant: String, key: String, public_url: String) -> Result<Self> {
        let mut gateway =
            Url::parse(gateway).map_err(|_| Error::Invalid("支付网关地址无效".into()))?;
        if gateway.scheme() != "https"
            || !gateway.username().is_empty()
            || gateway.password().is_some()
            || gateway.query().is_some()
            || gateway.fragment().is_some()
            || merchant.is_empty()
            || key.len() < 16
        {
            return Err(Error::Invalid(
                "支付网关必须使用 HTTPS，并配置商户号和有效密钥".into(),
            ));
        }
        if !gateway.path().ends_with('/') {
            gateway.set_path(&format!("{}/", gateway.path()));
        }
        Ok(Self {
            gateway,
            merchant,
            key,
            public_url,
        })
    }
    pub fn checkout(&self, order: &Order) -> Result<Value> {
        if order.state != "pending" {
            return Err(Error::Conflict);
        }
        let mut fields = BTreeMap::from([
            ("pid".into(), self.merchant.clone()),
            ("type".into(), order.channel.clone()),
            ("out_trade_no".into(), order.id.clone()),
            (
                "notify_url".into(),
                format!("{}/portal/api/payments/notify", self.public_url),
            ),
            (
                "return_url".into(),
                format!("{}/console?order={}", self.public_url, order.id),
            ),
            ("name".into(), format!("API Token 充值 {}", order.tokens)),
            (
                "money".into(),
                format!(
                    "{}.{:02}",
                    order.amount_cents / 100,
                    order.amount_cents % 100
                ),
            ),
        ]);
        fields.insert("sign".into(), signature(&fields, &self.key));
        fields.insert("sign_type".into(), "MD5".into());
        let action = self
            .gateway
            .join("submit.php")
            .map_err(|_| Error::Internal("Invalid gateway".into()))?;
        Ok(json!({"action":action.as_str(),"fields":fields}))
    }
    pub fn notify(
        &self,
        store: &Store,
        fields: &BTreeMap<String, String>,
        time: i64,
    ) -> Result<()> {
        let value = |k: &str| fields.get(k).map(String::as_str).unwrap_or("");
        if value("pid") != self.merchant
            || value("sign_type") != "MD5"
            || value("trade_status") != "TRADE_SUCCESS"
            || !bool::from(
                signature(fields, &self.key)
                    .as_bytes()
                    .ct_eq(value("sign").as_bytes()),
            )
        {
            return Err(Error::Forbidden);
        }
        store.credit_order(
            value("out_trade_no"),
            value("trade_no"),
            cents(value("money"))?,
            value("type"),
            time,
        )
    }
    pub async fn query(&self, order: &Order) -> Result<Option<(String, i64, String)>> {
        if order.state == "paid" {
            return Ok(None);
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Internal("Payment HTTP initialization failed".into()))?;
        let endpoint = self
            .gateway
            .join("api.php")
            .map_err(|_| Error::Internal("Invalid gateway".into()))?;
        let mut response = client
            .get(endpoint)
            .query(&[
                ("act", "order"),
                ("pid", self.merchant.as_str()),
                ("key", self.key.as_str()),
                ("out_trade_no", order.id.as_str()),
            ])
            .send()
            .await
            .map_err(|_| Error::Unavailable("支付平台暂时不可用".into()))?;
        if !response.status().is_success() || response.content_length().unwrap_or(0) > 65_536 {
            return Err(Error::Unavailable("支付平台返回异常".into()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error::Unavailable("支付查询失败".into()))?
        {
            if bytes.len() + chunk.len() > 65_536 {
                return Err(Error::Unavailable("支付平台返回异常".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let v: Value = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Unavailable("支付平台返回格式错误".into()))?;
        if v["code"].as_i64() != Some(1) {
            return Err(Error::Unavailable("订单尚未同步，请稍后查询".into()));
        }
        if v["out_trade_no"].as_str() != Some(order.id.as_str()) {
            return Err(Error::Forbidden);
        }
        if !(v["status"] == 1 || v["status"] == "1") {
            return Ok(None);
        }
        let trade = v["trade_no"].as_str().ok_or(Error::Forbidden)?.to_owned();
        let amount = cents(v["money"].as_str().ok_or(Error::Forbidden)?)?;
        let channel = v["type"].as_str().ok_or(Error::Forbidden)?.to_owned();
        if amount != order.amount_cents || channel != order.channel {
            return Err(Error::Forbidden);
        }
        Ok(Some((trade, amount, channel)))
    }
}
