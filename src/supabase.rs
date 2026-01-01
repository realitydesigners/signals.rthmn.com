use reqwest::Client;
use serde::Serialize;
use serde_json::Value as JsonValue;
use tracing::{info, warn};

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

    pub async fn insert_active_signal(
        &self,
        signal: &crate::tracker::ActiveSignal,
    ) -> Result<i64, reqwest::Error> {
        let target_hits_json: JsonValue = JsonValue::Array(
            signal.target_hits
                .iter()
                .map(|hit| {
                    hit.map(|(ts, price)| {
                        serde_json::json!({
                            "timestamp": ts,
                            "price": price
                        })
                    })
                    .unwrap_or(JsonValue::Null)
                })
                .collect()
        );
        
        let stop_loss_hit_json: JsonValue = signal.stop_loss_hit
            .map(|(ts, price)| {
                serde_json::json!({
                    "timestamp": ts,
                    "price": price
                })
            })
            .unwrap_or(JsonValue::Null);
        
        let payload = serde_json::json!({
            "pair": signal.pair,
            "signal_type": signal.signal_type.to_string(),
            "level": signal.level as i32,
            "pattern_sequence": signal.pattern_sequence,
            "box_details": signal.box_details,
            "entry": signal.entry,
            "stop_losses": signal.stop_losses,
            "targets": signal.targets,
            "target_hits": target_hits_json,
            "stop_loss_hit": stop_loss_hit_json,
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

    pub async fn update_target_hits(
        &self,
        signal_id: i64,
        target_hits: &[Option<(i64, f64)>],
        stop_loss_hit: Option<(i64, f64)>,
    ) -> Result<(), reqwest::Error> {
        let target_hits_json: JsonValue = JsonValue::Array(
            target_hits
                .iter()
                .map(|hit| {
                    hit.map(|(ts, price)| {
                        serde_json::json!({
                            "timestamp": ts,
                            "price": price
                        })
                    })
                    .unwrap_or(JsonValue::Null)
                })
                .collect()
        );
        
        let stop_loss_hit_json: JsonValue = stop_loss_hit
            .map(|(ts, price)| {
                serde_json::json!({
                    "timestamp": ts,
                    "price": price
                })
            })
            .unwrap_or(JsonValue::Null);
        
        let update = serde_json::json!({
            "target_hits": target_hits_json,
            "stop_loss_hit": stop_loss_hit_json,
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
            let hits_count = target_hits.iter().filter(|h| h.is_some()).count();
            info!(
                "[Supabase] Updated target hits for signal {}: {}/{} targets hit",
                signal_id, hits_count, target_hits.len()
            );
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!("[Supabase] Failed to update target hits for signal {}: {} - {}", signal_id, status, body);
        }

        Ok(())
    }
}
