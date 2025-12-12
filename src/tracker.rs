use crate::supabase::SupabaseClient;
use crate::types::SignalType;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::info;

/// Represents an active signal being tracked
#[derive(Clone, Debug)]
pub struct ActiveSignal {
    pub signal_id: String,
    pub pair: String,
    pub signal_type: SignalType,
    pub level: u32,
    pub entry: f64,
    pub stop_loss: f64,
    pub target: f64,
    pub risk_reward_ratio: Option<f64>,
    pub pattern_sequence: Vec<i32>,
    pub created_at: i64,
}

/// Settlement result for a signal
#[derive(Debug)]
pub struct Settlement {
    pub signal: ActiveSignal,
    pub status: &'static str,
    pub settled_price: f64,
}

/// Tracks active signals and checks for settlements on each price tick
pub struct SignalTracker {
    /// Map of pair -> list of active signals for that pair
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

    /// Add a new active signal - writes to Supabase and tracks in memory
    pub async fn add_signal(&self, signal: ActiveSignal) {
        let pair = signal.pair.clone();
        let signal_type = signal.signal_type.to_string();
        let level = signal.level;

        // Write to Supabase (subscribers explicitly null; server matches later)
        if let Err(e) = self
            .supabase
            .insert_active_signal(&signal)
            .await
        {
            tracing::warn!("[Tracker] Failed to write signal to Supabase: {}", e);
        }

        // Add to in-memory tracker
        let mut active = self.active.write().await;
        active
            .entry(pair.clone())
            .or_insert_with(Vec::new)
            .push(signal);

        info!(
            "[Tracker] Added active signal: {} {} L{} (total: {})",
            pair,
            signal_type,
            level,
            active.values().map(|v| v.len()).sum::<usize>()
        );

        drop(active);
    }

    /// Check price against all active signals for a pair
    /// Returns list of settlements that occurred
    pub async fn check_price(&self, pair: &str, current_price: f64) -> Vec<Settlement> {
        let mut settlements = Vec::new();

        // First, collect settlements while holding read lock
        let to_settle: Vec<(usize, &'static str, f64)>;
        {
            let active = self.active.read().await;
            let Some(signals) = active.get(pair) else {
                return vec![];
            };

            to_settle = signals
                .iter()
                .enumerate()
                .filter_map(|(idx, signal)| {
                    let settlement = match signal.signal_type {
                        SignalType::LONG => {
                            if current_price <= signal.stop_loss {
                                Some(("failed", current_price))
                            } else if current_price >= signal.target {
                                Some(("success", current_price))
                            } else {
                                None
                            }
                        }
                        SignalType::SHORT => {
                            if current_price >= signal.stop_loss {
                                Some(("failed", current_price))
                            } else if current_price <= signal.target {
                                Some(("success", current_price))
                            } else {
                                None
                            }
                        }
                    };
                    settlement.map(|(status, price)| (idx, status, price))
                })
                .collect();
        }

        if to_settle.is_empty() {
            return vec![];
        }

        // Now process settlements with write lock
        {
            let mut active = self.active.write().await;
            let Some(signals) = active.get_mut(pair) else {
                return vec![];
            };

            // Process in reverse order to preserve indices
            for (idx, status, settled_price) in to_settle.into_iter().rev() {
                if idx < signals.len() {
                    let signal = signals.remove(idx);

                    info!(
                        "[Tracker] SETTLED: {} {} L{} → {} @ {:.5}",
                        signal.pair, signal.signal_type, signal.level, status, settled_price
                    );

                    settlements.push(Settlement {
                        signal,
                        status,
                        settled_price,
                    });
                }
            }
        }

        // Process each settlement (update Supabase only)
        for settlement in &settlements {
            // Update Supabase status with settlement price and timestamp
            if let Err(e) = self
                .supabase
                .update_signal_status(
                    &settlement.signal.signal_id,
                    settlement.status,
                    settlement.settled_price,
                )
                .await
            {
                tracing::warn!("[Tracker] Failed to settle signal in Supabase: {}", e);
            }
        }

        settlements
    }

    /// Get total count of active signals
    pub async fn get_active_count(&self) -> usize {
        self.active.read().await.values().map(|v| v.len()).sum()
    }

    /// Get count of active signals per pair
    pub async fn get_active_by_pair(&self) -> HashMap<String, usize> {
        self.active
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect()
    }
}
