use crate::supabase::SupabaseClient;
use crate::types::{BoxDetail, SignalType, Target, StopLoss};
use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::info;

#[derive(Clone, Debug)]
pub struct ActiveSignal {
    pub id: i64,
    pub pair: String,
    pub signal_type: SignalType,
    pub level: u32,
    pub entry: f64,
    pub stop_losses: Vec<StopLoss>,
    pub targets: Vec<Target>,
    pub risk_reward: Vec<f64>,
    pub pattern_sequence: Vec<i32>,
    pub box_details: Vec<BoxDetail>,
    pub created_at: i64,
}

#[derive(Debug)]
pub struct Settlement {
    pub signal: ActiveSignal,
    pub status: &'static str,
}

pub struct SignalTracker {
    active: RwLock<HashMap<String, Vec<ActiveSignal>>>,
    supabase: SupabaseClient,
}

impl SignalTracker {
    pub fn new(supabase: SupabaseClient) -> Self {
        Self {
            active: RwLock::new(HashMap::new()),
            supabase,
        }
    }

    pub async fn add_signal(&self, mut signal: ActiveSignal) -> i64 {
        let pair_upper = signal.pair.to_uppercase();
        signal.pair = pair_upper.clone();
        let signal_type = signal.signal_type.to_string();
        let level = signal.level;

        let id = match self
            .supabase
            .insert_active_signal(&signal)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("[Tracker] Failed to write signal to Supabase: {}", e);
                return 0; // Return 0 on error - caller should handle
            }
        };

        signal.id = id;

        let mut active = self.active.write().await;
        active
            .entry(pair_upper.clone())
            .or_insert_with(Vec::new)
            .push(signal);

        let total = active.values().map(|v| v.len()).sum::<usize>();
        drop(active);
        info!("[Tracker] Added active signal: {} {} L{} (id: {}, total: {})", pair_upper, signal_type, level, id, total);
        id
    }

    pub async fn check_price(&self, pair: &str, current_price: f64) -> Vec<Settlement> {
        let now_iso = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        let pair_upper = pair.to_uppercase();
        let mut signals_to_update: Vec<i64> = Vec::new();
        
        if current_price <= 0.0 {
            tracing::warn!("[Tracker] Invalid price for {}: {}", pair, current_price);
            return vec![];
        }
        
        let to_settle: Vec<(usize, &'static str)> = {
            let mut active = self.active.write().await;
            let Some(signals) = active.get_mut(&pair_upper) else {
                tracing::debug!("[Tracker] No active signals found for pair: {} (checked: {})", pair, pair_upper);
                return vec![];
            };
            
            if signals.is_empty() {
                tracing::debug!("[Tracker] Pair {} has empty signals vector", pair_upper);
                return vec![];
            }
            
            tracing::debug!("[Tracker] Checking {} signals for {} @ {:.5}", signals.len(), pair_upper, current_price);

            signals
                .iter_mut()
                .enumerate()
                .filter_map(|(idx, signal)| {
                    // Check if stop loss was hit
                    let stop_loss_hit = self.check_stop_loss_hit(signal, current_price, &now_iso);
                    let hit_stop = stop_loss_hit;
                    
                    // Check if any targets were hit
                    let any_new_target_hit = self.check_target_hits(signal, current_price, &now_iso);
                    
                    // Collect signal IDs that need updating
                    if any_new_target_hit || stop_loss_hit {
                        signals_to_update.push(signal.id);
                    }
                    
                    // Determine if signal should be settled
                    let hit_final_target = signal.targets.last().map_or(false, |target| {
                        match signal.signal_type {
                            SignalType::LONG => current_price >= target.price,
                            SignalType::SHORT => current_price <= target.price,
                        }
                    });
                    
                    let targets_hit_count = signal.targets.iter().filter(|t| t.timestamp.is_some()).count();
                    let has_partial_targets = targets_hit_count > 0 && targets_hit_count < signal.targets.len();
                    
                    if hit_stop {
                        if has_partial_targets {
                            Some((idx, "partial"))
                        } else {
                            Some((idx, "failed"))
                        }
                    } else if hit_final_target {
                        Some((idx, "success"))
                    } else {
                        None
                    }
                })
                .collect()
        };

        // Update Supabase with target hits and stop loss hits
        {
            let active = self.active.read().await;
            for signal_id in signals_to_update {
                // Find the signal in active tracking
                if let Some(signal) = active.values()
                    .flatten()
                    .find(|s| s.id == signal_id)
                {
                    if let Err(e) = self
                        .supabase
                        .update_signal_targets_and_stops(signal_id, &signal.targets, &signal.stop_losses)
                        .await
                    {
                        tracing::warn!("[Tracker] Failed to update signal hits in Supabase: {}", e);
                    }
                }
            }
        }

        if to_settle.is_empty() {
            return vec![];
        }

        let mut settlements = Vec::new();
        let mut active = self.active.write().await;
        let Some(signals) = active.get_mut(&pair_upper) else {
            tracing::warn!("[Tracker] Signals removed before settlement for pair: {}", pair_upper);
            return vec![];
        };

        for (idx, status) in to_settle.into_iter().rev() {
            if idx < signals.len() {
                let signal = signals.remove(idx);
                
                let settled_price = if status == "failed" {
                    signal.stop_losses.first()
                        .and_then(|sl| sl.timestamp.as_ref().map(|_| sl.price))
                        .unwrap_or(0.0)
                } else {
                    signal.targets.last()
                        .and_then(|t| t.timestamp.as_ref().map(|_| t.price))
                        .unwrap_or(0.0)
                };
                
                let targets_hit = signal.targets.iter().filter(|t| t.timestamp.is_some()).count();
                info!(
                    "[Tracker] SETTLED: {} {} L{} → {} @ {:.5} (targets hit: {}/{})",
                    signal.pair, signal.signal_type, signal.level, status, settled_price,
                    targets_hit,
                    signal.targets.len()
                );
                settlements.push(Settlement { signal, status });
            }
        }

        drop(active);

        for settlement in &settlements {
            if let Err(e) = self
                .supabase
                .update_signal_status(settlement.signal.id, settlement.status)
                .await
            {
                tracing::warn!("[Tracker] Failed to update signal status in Supabase: {}", e);
            }
        }

        settlements
    }

    fn check_stop_loss_hit(&self, signal: &mut ActiveSignal, current_price: f64, now_iso: &str) -> bool {
        if let Some(stop_loss) = signal.stop_losses.first_mut() {
            if stop_loss.timestamp.is_some() {
                return false; // Already hit
            }

            let hit = match signal.signal_type {
                SignalType::LONG => current_price <= stop_loss.price,
                SignalType::SHORT => current_price >= stop_loss.price,
            };

            if hit {
                stop_loss.timestamp = Some(now_iso.to_string());
                info!(
                    "[Tracker] Stop loss hit: {} {} L{} (id: {}) stop = {:.5} @ {:.5}",
                    signal.pair, signal.signal_type, signal.level, signal.id, stop_loss.price, current_price
                );
                return true;
            } else {
                tracing::debug!(
                    "[Tracker] Stop loss not hit: {} {} L{} (id: {}) stop = {:.5} @ {:.5} (diff: {:.5})",
                    signal.pair, signal.signal_type, signal.level, signal.id, stop_loss.price, current_price,
                    if signal.signal_type == SignalType::LONG {
                        stop_loss.price - current_price
                    } else {
                        current_price - stop_loss.price
                    }
                );
            }
        } else {
            tracing::warn!("[Tracker] Signal {} has no stop loss", signal.id);
        }
        false
    }

    fn check_target_hits(&self, signal: &mut ActiveSignal, current_price: f64, now_iso: &str) -> bool {
        let mut any_new_hit = false;

        for (target_idx, target) in signal.targets.iter_mut().enumerate() {
            if target.timestamp.is_some() {
                continue; // Already hit
            }

            let hit = match signal.signal_type {
                SignalType::LONG => current_price >= target.price,
                SignalType::SHORT => current_price <= target.price,
            };

            if hit {
                target.timestamp = Some(now_iso.to_string());
                any_new_hit = true;
                info!(
                    "[Tracker] Target {} hit: {} {} L{} target[{}] = {:.5} @ {:.5}",
                    target_idx + 1, signal.pair, signal.signal_type, signal.level, target_idx, target.price, current_price
                );
            }
        }

        any_new_hit
    }

    pub async fn get_active_count(&self) -> usize {
        self.active.read().await.values().map(|v| v.len()).sum()
    }

    pub async fn get_active_by_pair(&self) -> HashMap<String, usize> {
        self.active
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect()
    }
}
