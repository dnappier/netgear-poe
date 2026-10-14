use crate::config::Config;
use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::{Client, RequestBuilder};
use serde_json::{Value, json};
use std::time::Duration;

pub struct Switch {
    client: Client,
    base: String,
    session: String,
}

fn checked(value: Value) -> Result<Value> {
    match value.pointer("/resp/respCode").and_then(Value::as_i64) {
        Some(0) => Ok(value),
        Some(code) => bail!("switch rejected request (respCode {code})"),
        None => bail!("invalid switch response: missing resp.respCode"),
    }
}

impl Switch {
    pub fn login(config: &Config) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()?;
        let base = format!("{}/api/v1", config.url.trim_end_matches('/'));
        let response = checked(
            client
                .post(format!("{base}/login"))
                .json(&json!({"user": {"name": config.username, "password": config.password}}))
                .send()
                .context("switch login failed")?
                .error_for_status()?
                .json()?,
        )?;
        let session = response
            .pointer("/user/session")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .context("login response missing session")?
            .to_owned();
        Ok(Self {
            client,
            base,
            session,
        })
    }

    fn request(&self, request: RequestBuilder) -> Result<Value> {
        checked(
            request
                .header("session", &self.session)
                .header("Cache-Control", "no-cache")
                .send()
                .context("switch request failed; mutations are not retried")?
                .error_for_status()?
                .json()
                .context("invalid JSON response")?,
        )
    }

    pub fn get(&self, endpoint: &str, params: &[(&str, &str)]) -> Result<Value> {
        self.request(
            self.client
                .get(format!("{}/{endpoint}", self.base))
                .query(params),
        )
    }

    pub fn neighbors(&self) -> Result<Vec<Value>> {
        let mut result = Vec::new();
        for page in 1..=1000 {
            let page = page.to_string();
            let data = self.get("neighbor", &[("indexPage", &page), ("pageSize", "100")])?;
            let rows = data
                .pointer("/lldpRemoteDevice/rows")
                .and_then(Value::as_array)
                .context("neighbor response missing rows")?;
            let total = data
                .pointer("/lldpRemoteDevice/total")
                .context("neighbor response missing total")?;
            let total = total
                .as_u64()
                .or_else(|| total.as_str()?.parse().ok())
                .context("invalid neighbor total")? as usize;
            ensure!(
                !rows.is_empty() || result.len() >= total,
                "neighbor pagination ended before total"
            );
            result.extend(rows.iter().cloned());
            if result.len() >= total {
                return Ok(result);
            }
        }
        bail!("neighbor pagination exceeded safety limit")
    }

    pub fn reset(&self, ports: &[u64]) -> Result<Value> {
        self.request(
            self.client
                .post(format!("{}/swcfg_poe_reset", self.base))
                .json(&reset_payload(ports)),
        )
        .context("PoE reset result uncertain on transport failure; inspect ports before retrying")
    }
}

pub fn reset_payload(ports: &[u64]) -> Value {
    json!({"poePortConfig": {"portId": ports}})
}

impl Drop for Switch {
    fn drop(&mut self) {
        // Best effort, bounded by the same timeout; never leak the session to disk/stdout.
        let _ = self
            .client
            .get(format!("{}/logout", self.base))
            .header("session", &self.session)
            .send();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn response_codes_are_not_http_statuses() {
        assert!(checked(json!({"resp":{"respCode":0}})).is_ok());
        assert!(checked(json!({"resp":{"respCode":12001}})).is_err());
        assert!(checked(json!({"ok":true})).is_err());
    }
    #[test]
    fn reset_matches_ui_contract() {
        assert_eq!(
            reset_payload(&[1, 3]),
            json!({"poePortConfig":{"portId":[1,3]}})
        );
    }
}
