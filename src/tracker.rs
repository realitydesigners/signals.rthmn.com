use crate::supabase::SupabaseClient;
use crate::types::{BoxDetail, SignalType};
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
    pub stop_losses: Vec<f64>,
    pub targets: Vec<f64>,
    pub target_hits: Vec<Option<(i64, f64)>>,
    pub stop_loss_hit: Option<(i64, f64)>,
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
        let pair = signal.pair.clone();
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
            .entry(pair.clone())
            .or_insert_with(Vec::new)
            .push(signal);

        let total = active.values().map(|v| v.len()).sum::<usize>();
        drop(active);
        info!("[Tracker] Added active signal: {} {} L{} (id: {}, total: {})", pair, signal_type, level, id, total);
        id
    }

    pub async fn check_price(&self, pair: &str, current_price: f64) -> Vec<Settlement> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut hit_updates: Vec<(i64, Vec<Option<(i64, f64)>>, Option<(i64, f64)>)> = Vec::new();
        
        let to_settle: Vec<(usize, &'static str)> = {
            let mut active = self.active.write().await;
            let Some(signals) = active.get_mut(pair) else {
                return vec![];
            };

            signals
                .iter_mut()
                .enumerate()
                .filter_map(|(idx, signal)| {
                    // Check if stop loss was hit
                    let stop_loss_hit = self.check_stop_loss_hit(signal, current_price, now);
                    let hit_stop = stop_loss_hit.is_some();
                    
                    // Check if any targets were hit
                    let any_new_target_hit = self.check_target_hits(signal, current_price, now);
                    
                    // Collect updates for Supabase if any hits detected
                    if any_new_target_hit || hit_stop {
                        hit_updates.push((signal.id, signal.target_hits.clone(), signal.stop_loss_hit));
                    }
                    
                    // Determine if signal should be settled
                    let hit_final_target = signal.targets.last().map_or(false, |&final_target| {
                        match signal.signal_type {
                            SignalType::LONG => current_price >= final_target,
                            SignalType::SHORT => current_price <= final_target,
                        }
                    });
                    
                    if hit_stop {
                        Some((idx, "failed"))
                    } else if hit_final_target {
                        Some((idx, "success"))
                    } else {
                        None
                    }
                })
                .collect()
        };

        // Update Supabase with target hits and stop loss hits
        for (signal_id, target_hits, stop_loss_hit) in hit_updates {
            if let Err(e) = self
                .supabase
                .update_target_hits(signal_id, &target_hits, stop_loss_hit)
                .await
            {
                tracing::warn!("[Tracker] Failed to update target hits in Supabase: {}", e);
            }
        }

        if to_settle.is_empty() {
            return vec![];
        }

        let mut settlements = Vec::new();
        let mut active = self.active.write().await;
        let Some(signals) = active.get_mut(pair) else {
            return vec![];
        };

        for (idx, status) in to_settle.into_iter().rev() {
            if idx < signals.len() {
                let signal = signals.remove(idx);
                
                let settled_price = if status == "failed" {
                    signal.stop_loss_hit.map(|(_, price)| price).unwrap_or(0.0)
                } else {
                    signal.target_hits.last().and_then(|h| h.map(|(_, price)| price)).unwrap_or(0.0)
                };
                
                info!(
                    "[Tracker] SETTLED: {} {} L{} → {} @ {:.5} (targets hit: {}/{})",
                    signal.pair, signal.signal_type, signal.level, status, settled_price,
                    signal.target_hits.iter().filter(|h| h.is_some()).count(),
                    signal.target_hits.len()
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

    fn check_stop_loss_hit(&self, signal: &mut ActiveSignal, current_price: f64, now: i64) -> Option<(i64, f64)> {
        if signal.stop_loss_hit.is_some() {
            return None; // Already hit
        }

        let stop_loss = signal.stop_losses.first().copied()?;
        let hit = match signal.signal_type {
            SignalType::LONG => current_price <= stop_loss,
            SignalType::SHORT => current_price >= stop_loss,
        };

        if hit {
            signal.stop_loss_hit = Some((now, current_price));
            info!(
                "[Tracker] Stop loss hit: {} {} L{} stop = {:.5} @ {:.5}",
                signal.pair, signal.signal_type, signal.level, stop_loss, current_price
            );
            Some((now, current_price))
        } else {
            None
        }
    }

    fn check_target_hits(&self, signal: &mut ActiveSignal, current_price: f64, now: i64) -> bool {
        let mut any_new_hit = false;

        for (target_idx, &target_price) in signal.targets.iter().enumerate() {
            if signal.target_hits[target_idx].is_some() {
                continue; // Already hit
            }

            let hit = match signal.signal_type {
                SignalType::LONG => current_price >= target_price,
                SignalType::SHORT => current_price <= target_price,
            };

            if hit {
                signal.target_hits[target_idx] = Some((now, current_price));
                any_new_hit = true;
                info!(
                    "[Tracker] Target {} hit: {} {} L{} target[{}] = {:.5} @ {:.5}",
                    target_idx + 1, signal.pair, signal.signal_type, signal.level, target_idx, target_price, current_price
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
