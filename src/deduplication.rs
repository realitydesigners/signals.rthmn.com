use crate::types::{BoxDetail, PatternMatch};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
struct BoxValueHistory {
    last_value: Option<i32>,
    flip_count: usize,
    box1_high_when_tracked: f64,
    box1_low_when_tracked: f64,
}

#[derive(Debug, Clone)]
struct PatternHistory {
    levels: Vec<u32>,
    box1_high: f64,
    box1_low: f64,
    last_seen: i64,
}

#[derive(Debug, Clone)]
struct L1Signal {
    #[allow(dead_code)]
    pattern_sequence: Vec<i32>,
    box1_high: f64,
    box1_low: f64,
    #[allow(dead_code)]
    created_at: i64,
}

#[derive(Debug, Clone)]
struct RecentSignal {
    pattern_key: String,
    level: u32,
    entry: f64,
    stop_loss: f64,
    target: f64,
    sent_at: i64,
}

pub struct Deduplicator {
    box_histories: RwLock<HashMap<String, HashMap<i32, BoxValueHistory>>>,
    pattern_histories: RwLock<HashMap<String, HashMap<String, PatternHistory>>>,
    active_l1_signals: RwLock<HashMap<String, L1Signal>>,
    recent_signals: RwLock<HashMap<String, Vec<RecentSignal>>>,
    box1_states: RwLock<HashMap<String, (f64, f64)>>,
}

impl Deduplicator {
    pub fn new() -> Self {
        Self {
            box_histories: RwLock::new(HashMap::new()),
            pattern_histories: RwLock::new(HashMap::new()),
            active_l1_signals: RwLock::new(HashMap::new()),
            recent_signals: RwLock::new(HashMap::new()),
            box1_states: RwLock::new(HashMap::new()),
        }
    }

    pub async fn should_filter_pattern(
        &self,
        pair: &str,
        pattern: &PatternMatch,
        _boxes: &[crate::types::Box],
        timestamp: i64,
    ) -> bool {
        let integer_values: Vec<i32> = pattern
            .box_details
            .iter()
            .map(|b| b.integer_value)
            .collect();

        if integer_values.is_empty() {
            return true;
        }

        let box1 = pattern.box_details.first().unwrap();
        let pattern_key = self.get_pattern_key(&pattern.traversal_path.path);

        let mut box_histories = self.box_histories.write().await;
        let mut pattern_histories = self.pattern_histories.write().await;
        let mut active_l1 = self.active_l1_signals.write().await;
        let mut box1_states = self.box1_states.write().await;

        let pair_box_history = box_histories.entry(pair.to_string()).or_insert_with(HashMap::new);
        let pair_pattern_history = pattern_histories
            .entry(pair.to_string())
            .or_insert_with(HashMap::new);

        let current_box1_state = (box1.high, box1.low);
        let box1_changed = if let Some(existing_state) = box1_states.get(pair) {
            (existing_state.0 - box1.high).abs() >= 0.00001
                || (existing_state.1 - box1.low).abs() >= 0.00001
        } else {
            false
        };

        if box1_changed {
            pair_box_history.clear();
            pair_pattern_history.clear();
            active_l1.retain(|k, _| !k.starts_with(&format!("{}:", pair)));
        }

        box1_states.insert(pair.to_string(), current_box1_state);

        if pattern.level == 1 {
            if self.should_filter_l1(pair, pattern, box1, &mut *active_l1, timestamp) {
                return true;
            }
        }

        if self.should_filter_box_flips(
            pattern,
            &integer_values,
            pair_box_history,
            box1,
        ) {
            return true;
        }

        self.update_box_histories(&integer_values, pair_box_history, box1);

        if self.should_prefer_higher_level(
            pattern,
            &pattern_key,
            pair_pattern_history,
            box1,
            timestamp,
        ) {
            return true;
        }

        false
    }

    pub async fn should_filter_recent_signal(
        &self,
        pair: &str,
        pattern_sequence: &[i32],
        level: u32,
        entry: f64,
        stop_loss: f64,
        target: f64,
        timestamp: i64,
    ) -> bool {
        const TIME_WINDOW_MS: i64 = 5 * 60 * 1000;
        const PRICE_TOLERANCE: f64 = 0.0001;

        let pattern_key = self.get_pattern_key(pattern_sequence);

        let mut recent_signals = self.recent_signals.write().await;
        let pair_signals = recent_signals
            .entry(pair.to_string())
            .or_insert_with(Vec::new);

        let now = timestamp;
        pair_signals.retain(|s| now - s.sent_at < TIME_WINDOW_MS);

        for signal in pair_signals.iter() {
            if signal.pattern_key == pattern_key
                && signal.level == level
            {
                let prices_match = (signal.entry - entry).abs() < PRICE_TOLERANCE
                    && (signal.stop_loss - stop_loss).abs() < PRICE_TOLERANCE
                    && (signal.target - target).abs() < PRICE_TOLERANCE;

                if prices_match {
                    return true;
                }
            }
        }

        pair_signals.push(RecentSignal {
            pattern_key,
            level,
            entry,
            stop_loss,
            target,
            sent_at: timestamp,
        });

        false
    }

    fn should_filter_l1(
        &self,
        pair: &str,
        pattern: &PatternMatch,
        box1: &BoxDetail,
        active_l1: &mut HashMap<String, L1Signal>,
        timestamp: i64,
    ) -> bool {
        let key = format!("{}:{}", pair, pattern.traversal_path.signal_type);

        if let Some(existing) = active_l1.get(&key) {
            let box1_unchanged = (existing.box1_high - box1.high).abs() < 0.00001
                && (existing.box1_low - box1.low).abs() < 0.00001;

            if box1_unchanged {
                return true;
            }
        }

        active_l1.insert(
            key,
            L1Signal {
                pattern_sequence: pattern.traversal_path.path.clone(),
                box1_high: box1.high,
                box1_low: box1.low,
                created_at: timestamp,
            },
        );

        false
    }

    fn should_filter_box_flips(
        &self,
        _pattern: &PatternMatch,
        integer_values: &[i32],
        pair_box_history: &mut HashMap<i32, BoxValueHistory>,
        box1: &BoxDetail,
    ) -> bool {
        for &value in integer_values {
            let abs_value = value.abs();
            let history = pair_box_history.entry(abs_value).or_insert_with(|| BoxValueHistory {
                last_value: None,
                flip_count: 0,
                box1_high_when_tracked: box1.high,
                box1_low_when_tracked: box1.low,
            });

            let box1_unchanged = (history.box1_high_when_tracked - box1.high).abs() < 0.00001
                && (history.box1_low_when_tracked - box1.low).abs() < 0.00001;

            if let Some(last_val) = history.last_value {
                let flipped = (last_val > 0 && value < 0) || (last_val < 0 && value > 0);
                
                if flipped {
                    if box1_unchanged {
                        history.flip_count += 1;
                        if history.flip_count > 3 {
                            return true;
                        }
                    } else {
                        history.flip_count = 0;
                        history.box1_high_when_tracked = box1.high;
                        history.box1_low_when_tracked = box1.low;
                    }
                }
            } else {
                history.box1_high_when_tracked = box1.high;
                history.box1_low_when_tracked = box1.low;
            }

            history.last_value = Some(value);
        }

        false
    }

    fn update_box_histories(
        &self,
        integer_values: &[i32],
        pair_box_history: &mut HashMap<i32, BoxValueHistory>,
        box1: &BoxDetail,
    ) {
        for &value in integer_values {
            let abs_value = value.abs();
            if let Some(history) = pair_box_history.get_mut(&abs_value) {
                let box1_changed = (history.box1_high_when_tracked - box1.high).abs() >= 0.00001
                    || (history.box1_low_when_tracked - box1.low).abs() >= 0.00001;
                
                if box1_changed {
                    history.flip_count = 0;
                    history.box1_high_when_tracked = box1.high;
                    history.box1_low_when_tracked = box1.low;
                }
            }
        }
    }

    fn should_prefer_higher_level(
        &self,
        pattern: &PatternMatch,
        pattern_key: &str,
        pair_pattern_history: &mut HashMap<String, PatternHistory>,
        box1: &BoxDetail,
        timestamp: i64,
    ) -> bool {
        if let Some(existing) = pair_pattern_history.get(pattern_key) {
            let box1_unchanged = (existing.box1_high - box1.high).abs() < 0.00001
                && (existing.box1_low - box1.low).abs() < 0.00001;

            if box1_unchanged {
                let max_existing_level = existing.levels.iter().max().copied().unwrap_or(0);
                if pattern.level <= max_existing_level {
                    return true;
                }

                let mut updated = existing.clone();
                updated.levels.push(pattern.level);
                updated.last_seen = timestamp;
                pair_pattern_history.insert(pattern_key.to_string(), updated);
                return false;
            }
        }

        pair_pattern_history.insert(
            pattern_key.to_string(),
            PatternHistory {
                levels: vec![pattern.level],
                box1_high: box1.high,
                box1_low: box1.low,
                last_seen: timestamp,
            },
        );

        false
    }

    fn get_pattern_key(&self, path: &[i32]) -> String {
        path.iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("_")
    }

    pub async fn cleanup_old_patterns(&self, max_age_ms: i64, current_time: i64) {
        let mut pattern_histories = self.pattern_histories.write().await;
        for pair_history in pattern_histories.values_mut() {
            pair_history.retain(|_, history| current_time - history.last_seen < max_age_ms);
        }
    }

    pub async fn remove_l1_signal(&self, pair: &str, signal_type: &str) {
        let mut active_l1 = self.active_l1_signals.write().await;
        let key = format!("{}:{}", pair, signal_type);
        active_l1.remove(&key);
    }
}

impl Default for Deduplicator {
    fn default() -> Self {
        Self::new()
    }
}

