use crate::types::{BoxDetail, PatternMatch};
use std::collections::{HashMap, HashSet};
use tokio::sync::RwLock;

const TOLERANCE: f64 = 0.00001;
const BOX0_CHANGE_TOLERANCE: f64 = 0.00001;

#[derive(Debug, Clone)]
struct L1Signal {
    box1_high: f64,
    box1_low: f64,
}

pub struct Deduplicator {
    active_l1_signals: RwLock<HashMap<String, L1Signal>>,
    box1_states: RwLock<HashMap<String, (f64, f64)>>,
    structural_boxes: RwLock<HashMap<String, HashMap<i32, (f64, f64)>>>,
}

impl Deduplicator {
    pub fn new() -> Self {
        Self {
            active_l1_signals: RwLock::new(HashMap::new()),
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
        let Some(box1) = pattern.box_details.first() else {
            return true;
        };

        let mut active_l1 = self.active_l1_signals.write().await;
        let mut box1_states = self.box1_states.write().await;

        let current_box1_state = (box1.high, box1.low);
        let box1_changed = if let Some(existing_state) = box1_states.get(pair) {
            (existing_state.0 - box1.high).abs() >= BOX0_CHANGE_TOLERANCE
                || (existing_state.1 - box1.low).abs() >= BOX0_CHANGE_TOLERANCE
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
        box_details: &[BoxDetail],
        signal_type: crate::types::SignalType,
        level: u32,
    ) -> bool {

        let is_long = matches!(signal_type, crate::types::SignalType::LONG);
        let mut structural: Vec<&BoxDetail> = box_details
            .iter()
            .filter(|b| (is_long && b.integer_value > 0) || (!is_long && b.integer_value < 0))
            .collect();
        
        structural.sort_by(|a, b| b.integer_value.abs().cmp(&a.integer_value.abs()));

        let tracked_structural: Vec<&BoxDetail> = structural
            .iter()
            .take(level as usize)
            .copied()
            .collect();

        if tracked_structural.is_empty() {
            return false;
        }

        // Create pattern key from tracked structural boxes only (not full pattern sequence)
        // This ensures patterns with same structural boxes share the same key, even if they have different levels
        // For example, L5 and L6 with same structural boxes (up to L5's entry) will share tracking
        let structural_key: String = tracked_structural
            .iter()
            .map(|b| b.integer_value.to_string())
            .collect::<Vec<_>>()
            .join("_");
        
        // Include signal type in key to separate LONG and SHORT
        let tracking_key = format!("{}:{}:{}", pair, signal_type, structural_key);

        let mut tracked = self.structural_boxes.write().await;
        let pattern_tracked = tracked.entry(tracking_key.clone()).or_insert_with(HashMap::new);

        let mut all_match = !pattern_tracked.is_empty();
        let mut any_changed = false;

        for box_detail in &tracked_structural {
            let integer_value = box_detail.integer_value;
            let current = (box_detail.high, box_detail.low);

            if let Some(&tracked) = pattern_tracked.get(&integer_value) {
                let changed = (tracked.0 - current.0).abs() >= TOLERANCE || (tracked.1 - current.1).abs() >= TOLERANCE;
                if changed {
                    any_changed = true;
                    all_match = false;
                    pattern_tracked.insert(integer_value, current);
                }
            } else {
                all_match = false;
                pattern_tracked.insert(integer_value, current);
            }
        }

        !any_changed && all_match
    }

    fn should_filter_l1(
        &self,
        pair: &str,
        pattern: &PatternMatch,
        box1: &BoxDetail,
        active_l1: &mut HashMap<String, L1Signal>,
        _timestamp: i64,
    ) -> bool {
        let key = format!("{}:{}", pair, pattern.traversal_path.signal_type());

        if let Some(existing) = active_l1.get(&key) {
            let box1_unchanged = (existing.box1_high - box1.high).abs() < BOX0_CHANGE_TOLERANCE
                && (existing.box1_low - box1.low).abs() < BOX0_CHANGE_TOLERANCE;

            if box1_unchanged {
                return true;
            }
        }

        active_l1.insert(key, L1Signal { box1_high: box1.high, box1_low: box1.low });

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
            let pattern_signal_type = pattern.traversal_path.signal_type();
            
            let is_duplicate = unique_patterns.iter().any(|existing: &PatternMatch| {
                if existing.traversal_path.signal_type() != pattern_signal_type {
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

