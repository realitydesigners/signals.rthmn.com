use crate::types::{BoxDetail, PatternMatch};
use std::collections::{HashMap, HashSet};
use tokio::sync::RwLock;

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
    active_l1_signals: RwLock<HashMap<String, L1Signal>>,
    recent_signals: RwLock<HashMap<String, Vec<RecentSignal>>>,
    box1_states: RwLock<HashMap<String, (f64, f64)>>,
    structural_boxes: RwLock<HashMap<String, HashMap<i32, (f64, f64)>>>,
}

impl Deduplicator {
    pub fn new() -> Self {
        Self {
            active_l1_signals: RwLock::new(HashMap::new()),
            recent_signals: RwLock::new(HashMap::new()),
            box1_states: RwLock::new(HashMap::new()),
            structural_boxes: RwLock::new(HashMap::new()),
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

        let mut active_l1 = self.active_l1_signals.write().await;
        let mut box1_states = self.box1_states.write().await;

        let current_box1_state = (box1.high, box1.low);
        let box1_changed = if let Some(existing_state) = box1_states.get(pair) {
            (existing_state.0 - box1.high).abs() >= 0.00001
                || (existing_state.1 - box1.low).abs() >= 0.00001
        } else {
            false
        };

        if box1_changed {
            active_l1.retain(|k, _| !k.starts_with(&format!("{}:", pair)));
        }

        box1_states.insert(pair.to_string(), current_box1_state);

        if pattern.level == 1 {
            if self.should_filter_l1(pair, pattern, box1, &mut *active_l1, timestamp) {
                return true;
            }
        }

        false
    }

    pub async fn should_filter_structural_boxes(
        &self,
        pair: &str,
        pattern_sequence: &[i32],
        box_details: &[BoxDetail],
        signal_type: crate::types::SignalType,
        level: u32,
    ) -> bool {
        const TOLERANCE: f64 = 0.00001;

        let mut structural: Vec<&BoxDetail> = box_details
            .iter()
            .filter(|b| match signal_type {
                crate::types::SignalType::LONG => b.integer_value > 0,
                crate::types::SignalType::SHORT => b.integer_value < 0,
            })
            .collect();
        
        structural.sort_by(|a, b| b.integer_value.abs().cmp(&a.integer_value.abs()));

        let entry_box_index = level as usize;
        let structural: Vec<&BoxDetail> = structural
            .into_iter()
            .enumerate()
            .filter(|(idx, _)| *idx < entry_box_index)
            .map(|(_, b)| b)
            .collect();

        if structural.is_empty() {
            return false;
        }

        let pattern_key: String = pattern_sequence
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("_");
        
        let tracking_key = format!("{}:{}", pair, pattern_key);

        let mut tracked = self.structural_boxes.write().await;
        let pattern_tracked = tracked.entry(tracking_key.clone()).or_insert_with(HashMap::new);

        let mut all_match = true;
        let mut any_changed = false;

        for box_detail in &structural {
            let integer_value = box_detail.integer_value;
            let current_high = box_detail.high;
            let current_low = box_detail.low;

            if let Some(&(tracked_high, tracked_low)) = pattern_tracked.get(&integer_value) {
                let high_changed = (tracked_high - current_high).abs() >= TOLERANCE;
                let low_changed = (tracked_low - current_low).abs() >= TOLERANCE;

                if high_changed || low_changed {
                    any_changed = true;
                    all_match = false;
                    pattern_tracked.insert(integer_value, (current_high, current_low));
                }
            } else {
                all_match = false;
                pattern_tracked.insert(integer_value, (current_high, current_low));
            }
        }

        if any_changed {
            false
        } else if all_match && !pattern_tracked.is_empty() {
            true
        } else {
            false
        }
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

    pub async fn remove_l1_signal(&self, pair: &str, signal_type: &str) {
        let mut active_l1 = self.active_l1_signals.write().await;
        let key = format!("{}:{}", pair, signal_type);
        active_l1.remove(&key);
    }

    pub fn remove_subset_duplicates(&self, patterns: Vec<PatternMatch>) -> Vec<PatternMatch> {
        let mut unique_patterns = Vec::new();
        let mut sorted_patterns = patterns;
        sorted_patterns.sort_by(|a, b| b.level.cmp(&a.level));
        
        for pattern in sorted_patterns {
            let pattern_values: HashSet<i32> = pattern.traversal_path.path.iter().copied().collect();
            let pattern_signal_type = pattern.traversal_path.signal_type;
            
            let is_duplicate = unique_patterns.iter().any(|existing: &PatternMatch| {
                if existing.traversal_path.signal_type != pattern_signal_type {
                    return false;
                }
                if existing.level <= pattern.level {
                    return false;
                }
                let existing_values: HashSet<i32> = existing.traversal_path.path.iter().copied().collect();
                pattern_values.is_subset(&existing_values)
            });
            
            if !is_duplicate {
                unique_patterns.push(pattern);
            }
        }
        
        unique_patterns
    }
}

impl Default for Deduplicator {
    fn default() -> Self {
        Self::new()
    }
}

