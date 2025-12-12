use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tracing::{info, warn};

/// Supabase client for storing signals and updating status/notifications
#[derive(Clone)]
pub struct SupabaseClient {
    client: Client,
    url: String,
    service_key: String,
    fcm_server_key: Option<String>,
}

#[derive(Serialize)]
struct NewSignalLegacy {
    signal_id: String,
    pair: String,
    signal_type: String,
    level: i32,
    pattern_sequence: Vec<i32>,
    entry: f64,
    stop_loss: f64,
    target: f64,
    risk_reward_ratio: Option<f64>,
    status: String,
    timestamp: String,
    subscribers: JsonValue,
}

#[derive(Serialize)]
struct UpdateSignalSettlement {
    status: String,
    settled_price: f64,
    settled_at: String,
}

#[derive(Deserialize)]
struct SignalRecipientsRow {
    subscribers: Option<JsonValue>,
}

#[derive(Deserialize)]
struct UserProfileRow {
    user_id: String,
    // Supports either a single token or an array/json list
    fcm_token: Option<String>,
    device_tokens: Option<JsonValue>,
}

impl SupabaseClient {
    pub fn new(url: &str, service_key: &str) -> Self {
        Self {
            client: Client::new(),
            url: url.to_string(),
            service_key: service_key.to_string(),
            fcm_server_key: std::env::var("FCM_SERVER_KEY").ok(),
        }
    }

    /// Insert a new signal row. subscribers MUST be null. status MUST be "active".
    pub async fn insert_active_signal(
        &self,
        signal: &crate::tracker::ActiveSignal,
    ) -> Result<(), reqwest::Error> {
        let created_at_dt =
            DateTime::from_timestamp_millis(signal.created_at).unwrap_or_else(Utc::now);

        let payload = NewSignalLegacy {
            signal_id: signal.signal_id.clone(),
            pair: signal.pair.clone(),
            signal_type: signal.signal_type.to_string(),
            level: signal.level as i32,
            pattern_sequence: signal.pattern_sequence.clone(),
            entry: signal.entry,
            stop_loss: signal.stop_loss,
            target: signal.target,
            risk_reward_ratio: signal.risk_reward_ratio,
            status: "active".to_string(),
            timestamp: created_at_dt.to_rfc3339(),
            subscribers: JsonValue::Null,
        };

        let response = self
            .client
            .post(&format!("{}/rest/v1/signals", self.url))
            .header("apikey", &self.service_key)
            .header("Authorization", format!("Bearer {}", self.service_key))
            .header("Content-Type", "application/json")
            .header("Prefer", "return=minimal")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status_code = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!(
                "[Supabase] Failed to insert signal {}: {} - {}",
                payload.signal_id, status_code, body
            );
            return Ok(());
        }

        info!(
            "[Supabase] Inserted signal: {} {} L{}",
            payload.pair, payload.signal_type, payload.level
        );
        Ok(())
    }

    /// Update signal status with settlement details
    pub async fn update_signal_status(
        &self,
        signal_id: &str,
        status: &str,
        settled_price: f64,
    ) -> Result<(), reqwest::Error> {
        let update = UpdateSignalSettlement {
            status: status.to_string(),
            settled_price,
            settled_at: Utc::now().to_rfc3339(),
        };

        let response = self
            .client
            .patch(&format!("{}/rest/v1/signals", self.url))
            .header("apikey", &self.service_key)
            .header("Authorization", format!("Bearer {}", self.service_key))
            .header("Content-Type", "application/json")
            .header("Prefer", "return=minimal")
            .query(&[("signal_id", format!("eq.{}", signal_id))])
            .json(&update)
            .send()
            .await?;

        if response.status().is_success() {
            info!(
                "[Supabase] Settled signal {} -> {} @ {:.5}",
                signal_id, status, settled_price
            );
        } else {
            let status_code = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!(
                "[Supabase] Failed to settle signal {}: {} - {}",
                signal_id, status_code, body
            );
        }

        Ok(())
    }

    async fn get_signal_subscribers(
        &self,
        signal_id: &str,
    ) -> Result<Vec<String>, reqwest::Error> {
        let response = self
            .client
            .get(&format!("{}/rest/v1/signals", self.url))
            .header("apikey", &self.service_key)
            .header("Authorization", format!("Bearer {}", self.service_key))
            .query(&[
                ("select", "subscribers"),
                ("signal_id", &format!("eq.{}", signal_id)),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            warn!(
                "[Supabase] Failed to fetch subscribers for {}: {}",
                signal_id,
                response.status()
            );
            return Ok(vec![]);
        }

        let rows: Vec<SignalRecipientsRow> = response.json().await.unwrap_or_default();
        let Some(first) = rows.first() else { return Ok(vec![]) };

        let Some(json) = &first.subscribers else { return Ok(vec![]) };
        let JsonValue::Array(arr) = json else { return Ok(vec![]) };

        Ok(arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect())
    }

    async fn get_user_profiles(
        &self,
        user_ids: &[String],
    ) -> Result<Vec<UserProfileRow>, reqwest::Error> {
        if user_ids.is_empty() {
            return Ok(vec![]);
        }

        // PostgREST "in" filter: user_id=in.(a,b,c)
        let joined = user_ids.join(",");
        let response = self
            .client
            .get(&format!("{}/rest/v1/user_profiles", self.url))
            .header("apikey", &self.service_key)
            .header("Authorization", format!("Bearer {}", self.service_key))
            .query(&[
                ("select", "user_id,fcm_token,device_tokens"),
                ("user_id", &format!("in.({})", joined)),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            warn!(
                "[Supabase] Failed to fetch user_profiles: {}",
                response.status()
            );
            return Ok(vec![]);
        }

        Ok(response.json().await.unwrap_or_default())
    }

    fn extract_fcm_tokens(profile: &UserProfileRow) -> Vec<String> {
        if let Some(t) = &profile.fcm_token {
            if !t.is_empty() {
                return vec![t.clone()];
            }
        }

        match &profile.device_tokens {
            Some(JsonValue::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => vec![],
        }
    }

    async fn send_fcm(
        &self,
        token: &str,
        title: &str,
        body: &str,
    ) -> Result<(), reqwest::Error> {
        let Some(server_key) = &self.fcm_server_key else {
            return Ok(());
        };

        let payload = serde_json::json!({
            "to": token,
            "notification": {
                "title": title,
                "body": body
            }
        });

        let response = self
            .client
            .post("https://fcm.googleapis.com/fcm/send")
            .header("Authorization", format!("key={}", server_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            warn!(
                "[FCM] Failed to send notification: {}",
                response.status()
            );
        }

        Ok(())
    }

    /// When a signal closes, notify all subscribers using device tokens in user_profiles.
    pub async fn push_signal_closed(
        &self,
        signal_id: &str,
        pair: &str,
        status: &str,
    ) -> Result<(), reqwest::Error> {
        let subscribers = self.get_signal_subscribers(signal_id).await?;
        if subscribers.is_empty() {
            return Ok(());
        }

        let profiles = self.get_user_profiles(&subscribers).await?;
        if profiles.is_empty() {
            return Ok(());
        }

        let title = "Signal Closed";
        let body = format!("Your {} signal hit {}.", pair, status);

        for profile in profiles {
            for token in Self::extract_fcm_tokens(&profile) {
                // Best-effort, per-token
                let _ = self.send_fcm(&token, title, &body).await;
            }
        }

        Ok(())
    }
}
