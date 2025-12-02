use crate::instruments::get_instrument_config;
use crate::patterns::{BOXES, STARTING_POINTS};
use crate::types::{Box, BoxDetail, PatternMatch, SignalType, TraversalPath};
use std::collections::HashSet;

/// MarketScanner - Detects trading patterns from live box data
pub struct MarketScanner {
    all_paths: Vec<TraversalPath>,
    last_hashes: std::collections::HashMap<String, String>,
}

impl MarketScanner {
    pub fn new() -> Self {
        Self {
            all_paths: Vec::new(),
            last_hashes: std::collections::HashMap::new(),
        }
    }

    /// Pre-calculate all possible traversal paths at startup
    pub fn initialize(&mut self) {
        self.all_paths.clear();
        
        // Generate all starting points (positive and negative)
        let mut all_starting: Vec<i32> = Vec::new();
        for &sp in STARTING_POINTS {
            all_starting.push(sp);
            all_starting.push(-sp);
        }

        // Calculate all traversal paths
        for &start in &all_starting {
            self.traverse_all_paths(start, vec![start], start);
        }
    }

    fn traverse_all_paths(&mut self, current_key: i32, current_path: Vec<i32>, original_start: i32) {
        let abs_key = current_key.abs();
        let patterns = BOXES.get(&abs_key);

        if patterns.is_none() || patterns.unwrap().is_empty() {
            // Terminal node - save the complete path
            let path = TraversalPath {
                path: current_path.clone(),
                length: current_path.len(),
                starting_point: original_start,
                signal_type: if original_start > 0 { SignalType::LONG } else { SignalType::SHORT },
            };
            self.all_paths.push(path);
            return;
        }

        for pattern in patterns.unwrap() {
            // Adjust pattern based on current key sign
            let adjusted: Vec<i32> = if current_key > 0 {
                pattern.clone()
            } else {
                pattern.iter().map(|&v| -v).collect()
            };

            let mut full_path = current_path.clone();
            full_path.extend(&adjusted);
            let last_value = *adjusted.last().unwrap();

            // Check for cycle (same absolute value)
            if last_value.abs() == current_key.abs() {
                let path = TraversalPath {
                    path: full_path.clone(),
                    length: full_path.len(),
                    starting_point: original_start,
                    signal_type: if original_start > 0 { SignalType::LONG } else { SignalType::SHORT },
                };
                self.all_paths.push(path);
                continue;
            }

            // Continue traversal
            self.traverse_all_paths(last_value, full_path, original_start);
        }
    }

    pub fn path_count(&self) -> usize {
        self.all_paths.len()
    }

    /// Detect patterns in live box data
    pub fn detect_patterns(&self, pair: &str, boxes: &[Box]) -> Vec<PatternMatch> {
        if boxes.is_empty() {
            return Vec::new();
        }

        // Get instrument config for point value
        let (point, _digits) = get_instrument_config(pair);
        
        // Convert box values to integers
        let integer_values: Vec<i32> = boxes
            .iter()
            .map(|b| (b.value / point).round() as i32)
            .collect();
        let value_set: HashSet<i32> = integer_values.iter().copied().collect();

        // Check hash to avoid reprocessing same data
        let hash = integer_values.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",");
        // Note: We can't use mutable self here, so deduplication should be done at caller level

        let mut matches = Vec::new();

        for path in &self.all_paths {
            // Quick check: first value must exist in live data
            let first = path.path[0].abs();
            if !value_set.contains(&first) && !value_set.contains(&(-first)) {
                continue;
            }

            // Check if all path values exist in live data
            if self.is_path_present(&path.path, &value_set) {
                let pattern_match = self.create_pattern_match(pair, path, boxes, &integer_values, point);
                matches.push(pattern_match);
            }
        }

        matches
    }

    fn is_path_present(&self, path: &[i32], value_set: &HashSet<i32>) -> bool {
        for &value in path {
            if !value_set.contains(&value) {
                return false;
            }
        }
        true
    }

    fn create_pattern_match(
        &self,
        pair: &str,
        traversal: &TraversalPath,
        boxes: &[Box],
        integer_values: &[i32],
        point: f64,
    ) -> PatternMatch {
        let level = self.calculate_level(&traversal.path);
        
        // Get box details for each path value
        let mut box_details = Vec::new();
        for &path_value in &traversal.path {
            // Find the box with this integer value
            for (i, &int_val) in integer_values.iter().enumerate() {
                if int_val == path_value {
                    box_details.push(BoxDetail {
                        integer_value: int_val,
                        high: boxes[i].high,
                        low: boxes[i].low,
                        value: boxes[i].value,
                    });
                    break;
                }
            }
        }

        PatternMatch {
            pair: pair.to_string(),
            level,
            traversal_path: traversal.clone(),
            full_pattern: traversal.path.clone(),
            box_details,
        }
    }

    fn calculate_level(&self, path: &[i32]) -> u32 {
        if path.len() <= 1 {
            return 1;
        }

        let mut level = 0;
        let mut current_index = 0;
        let mut current_key = path[0];

        while current_index < path.len() - 1 {
            let abs_key = current_key.abs();
            let patterns = BOXES.get(&abs_key);

            if patterns.is_none() || patterns.unwrap().is_empty() {
                break;
            }

            let mut pattern_found = false;
            for pattern in patterns.unwrap() {
                let adjusted: Vec<i32> = if current_key > 0 {
                    pattern.clone()
                } else {
                    pattern.iter().map(|&v| -v).collect()
                };

                let pattern_start_index = current_index + 1;
                let pattern_end_index = pattern_start_index + adjusted.len();

                if pattern_end_index <= path.len() {
                    let path_segment: Vec<i32> = path[pattern_start_index..pattern_end_index].to_vec();
                    if adjusted == path_segment {
                        level += 1;
                        current_index = pattern_end_index - 1;
                        current_key = *adjusted.last().unwrap();
                        pattern_found = true;
                        break;
                    }
                }
            }

            if !pattern_found {
                break;
            }
        }

        level.max(1)
    }
}
