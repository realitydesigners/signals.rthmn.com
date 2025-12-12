use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value as JsonValue;
use tracing::{info, warn};

/// Supabase client for storing signals and updating status
#[derive(Clone)]
pub struct SupabaseClient {
    client: Client,
    url: String,
    service_key: String,
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

impl SupabaseClient {
    pub fn new(url: &str, service_key: &str) -> Self {
        Self {
            client: Client::new(),
            url: url.to_string(),
            service_key: service_key.to_string(),
        }
    }

    /// Insert a new signal row
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
}
