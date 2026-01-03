use reqwest::Client;
use serde::Serialize;
use serde_json::Value as JsonValue;
use tracing::{info, warn};
use chrono::{DateTime, Utc, TimeZone};

#[derive(Clone)]
pub struct SupabaseClient {
    client: Client,
    url: String,
    service_key: String,
}

#[derive(Serialize)]
struct UpdateSignalStatus {
    status: String,
}

impl SupabaseClient {
    pub fn new(url: &str, service_key: &str) -> Self {
        Self {
            client: Client::new(),
            url: url.to_string(),
            service_key: service_key.to_string(),
        }
    }

    fn timestamp_ms_to_iso_string(ts_ms: i64) -> String {
        let seconds = ts_ms / 1000;
        let nanos = ((ts_ms % 1000) * 1_000_000) as u32;
        let dt = Utc.timestamp_opt(seconds, nanos)
            .single()
            .unwrap_or_else(Utc::now);
        dt.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
    }

    pub async fn insert_active_signal(
        &self,
        signal: &crate::tracker::ActiveSignal,
    ) -> Result<i64, reqwest::Error> {
        let payload = serde_json::json!({
            "pair": signal.pair,
            "signal_type": signal.signal_type.to_string(),
            "level": signal.level as i32,
            "pattern_sequence": signal.pattern_sequence,
            "box_details": signal.box_details,
            "entry": signal.entry,
            "stop_losses": signal.stop_losses,
            "targets": signal.targets,
            "risk_reward": signal.risk_reward,
            "status": "active",
            "subscribers": JsonValue::Null,
        });

        let response = self
            .client
            .post(&format!("{}/rest/v1/signals", self.url))
            .header("apikey", &self.service_key)
            .header("Authorization", format!("Bearer {}", self.service_key))
            .header("Content-Type", "application/json")
            .header("Prefer", "return=representation")
            .json(&payload)
            .send()
            .await?;

        let response = response.error_for_status().map_err(|e| {
            let pair = payload.get("pair").and_then(|v| v.as_str()).unwrap_or("unknown");
            warn!("[Supabase] Failed to insert signal for {}: {}", pair, e);
            e
        })?;

        let inserted: serde_json::Value = response.json().await?;
        let id = match inserted
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|row| row.get("id"))
            .and_then(|v| v.as_i64())
        {
            Some(id) => id,
            None => {
                warn!("[Supabase] Insert succeeded but no id returned");
                // Create an error by making a request that will fail
                return Err(
                    self.client
                        .get("http://invalid-url-that-will-fail.example.com")
                        .send()
                        .await
                        .expect_err("This should always fail")
                );
            }
        };

        let pair = payload.get("pair").and_then(|v| v.as_str()).unwrap_or("unknown");
        let signal_type = payload.get("signal_type").and_then(|v| v.as_str()).unwrap_or("unknown");
        let level = payload.get("level").and_then(|v| v.as_i64()).unwrap_or(0);
        info!(
            "[Supabase] Inserted signal: {} {} L{} (id: {})",
            pair, signal_type, level, id
        );
        Ok(id)
    }

    pub async fn update_signal_status(
        &self,
        signal_id: i64,
        status: &str,
    ) -> Result<(), reqwest::Error> {
        let update = UpdateSignalStatus {
            status: status.to_string(),
        };

        let response = self
            .client
            .patch(&format!("{}/rest/v1/signals", self.url))
            .header("apikey", &self.service_key)
            .header("Authorization", format!("Bearer {}", self.service_key))
            .header("Content-Type", "application/json")
            .header("Prefer", "return=minimal")
            .query(&[("id", format!("eq.{}", signal_id))])
            .json(&update)
            .send()
            .await?;

        if response.status().is_success() {
            info!(
                "[Supabase] Updated signal {} status to {}",
                signal_id, status
            );
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!("[Supabase] Failed to update signal {} status: {} - {}", signal_id, status, body);
        }

        Ok(())
    }

    pub async fn update_signal_hits(
        &self,
        signal_id: i64,
    ) -> Result<(), reqwest::Error> {
        // This method is called after hits are updated in memory
        // We need to fetch the signal from active tracking and update it
        // For now, we'll update targets and stop_losses directly
        // The actual signal data will be updated via a separate method that fetches from active tracking
        // This is a placeholder - the real update happens in tracker.rs before calling this
        
        Ok(())
    }
    
    pub async fn update_signal_targets_and_stops(
        &self,
        signal_id: i64,
        targets: &[crate::types::Target],
        stop_losses: &[crate::types::StopLoss],
    ) -> Result<(), reqwest::Error> {
        let update = serde_json::json!({
            "targets": targets,
            "stop_losses": stop_losses,
        });

        let response = self
            .client
            .patch(&format!("{}/rest/v1/signals", self.url))
            .header("apikey", &self.service_key)
            .header("Authorization", format!("Bearer {}", self.service_key))
            .header("Content-Type", "application/json")
            .header("Prefer", "return=minimal")
            .query(&[("id", format!("eq.{}", signal_id))])
            .json(&update)
            .send()
            .await?;

        if response.status().is_success() {
            let targets_hit = targets.iter().filter(|t| t.timestamp.is_some()).count();
            let stop_hit = stop_losses.first().and_then(|sl| sl.timestamp.as_ref()).is_some();
            info!(
                "[Supabase] Updated signal {}: {}/{} targets hit, stop loss hit: {}",
                signal_id, targets_hit, targets.len(), stop_hit
            );
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!("[Supabase] Failed to update signal {}: {} - {}", signal_id, status, body);
        }

        Ok(())
    }
}
