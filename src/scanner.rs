use crate::instruments::get_instrument_config;
use crate::patterns::{BOXES, STARTING_POINTS};
use crate::types::{Box, BoxDetail, PatternMatch, SignalType, TraversalPath};
use std::collections::HashSet;

#[derive(Default)]
pub struct MarketScanner {
    all_paths: Vec<TraversalPath>,
}

impl MarketScanner {
    pub fn initialize(&mut self) {
        self.all_paths.clear();
        for &sp in STARTING_POINTS {
            self.traverse_all_paths(sp, vec![sp], sp);
            self.traverse_all_paths(-sp, vec![-sp], -sp);
        }
    }

    fn make_path(&self, path: Vec<i32>, start: i32) -> TraversalPath {
        TraversalPath {
            length: path.len(),
            signal_type: if start > 0 { SignalType::LONG } else { SignalType::SHORT },
            path,
            starting_point: start,
        }
    }

    fn traverse_all_paths(&mut self, current_key: i32, current_path: Vec<i32>, original_start: i32) {
        let Some(patterns) = BOXES.get(&current_key.abs()).filter(|p| !p.is_empty()) else {
            self.all_paths.push(self.make_path(current_path, original_start));
            return;
        };

        for pattern in patterns {
            let adjusted: Vec<i32> = if current_key > 0 {
                pattern.clone()
            } else {
                pattern.iter().map(|&v| -v).collect()
            };
            let last = *adjusted.last().unwrap();

            // Self-terminating pattern
            if adjusted.len() == 1 && last.abs() == current_key.abs() {
                self.all_paths.push(self.make_path(current_path.clone(), original_start));
                continue;
            }

            let mut full_path = current_path.clone();
            full_path.extend(&adjusted);

            // Cycle detection
            if last.abs() == current_key.abs() {
                self.all_paths.push(self.make_path(full_path, original_start));
            } else {
                self.traverse_all_paths(last, full_path, original_start);
            }
        }
    }

    pub fn path_count(&self) -> usize {
        self.all_paths.len()
    }

    pub fn detect_patterns(&self, pair: &str, boxes: &[Box]) -> Vec<PatternMatch> {
        if boxes.is_empty() { return vec![]; }

        let (point, _) = get_instrument_config(pair);
        let integer_values: Vec<i32> = boxes.iter().map(|b| (b.value / point).round() as i32).collect();
        let value_set: HashSet<i32> = integer_values.iter().copied().collect();

        self.all_paths.iter()
            .filter(|path| {
                let first = path.path[0].abs();
                (value_set.contains(&first) || value_set.contains(&(-first)))
                    && path.path.iter().all(|v| value_set.contains(v))
            })
            .map(|path| self.create_pattern_match(pair, path, boxes, &integer_values))
            .collect()
    }

    fn create_pattern_match(&self, pair: &str, traversal: &TraversalPath, boxes: &[Box], integer_values: &[i32]) -> PatternMatch {
        let box_details: Vec<BoxDetail> = traversal.path.iter()
            .filter_map(|&path_value| {
                integer_values.iter().position(|&v| v == path_value).map(|i| BoxDetail {
                    integer_value: path_value,
                    high: boxes[i].high,
                    low: boxes[i].low,
                    value: boxes[i].value,
                })
            })
            .collect();

        PatternMatch {
            pair: pair.to_string(),
            level: self.calculate_level(&traversal.path),
            traversal_path: traversal.clone(),
            full_pattern: traversal.path.clone(),
            box_details,
        }
    }

    fn calculate_level(&self, path: &[i32]) -> u32 {
        if path.len() <= 1 { return 1; }

        let mut level = 0u32;
        let mut idx = 0;
        let mut key = path[0];

        while idx < path.len() - 1 {
            let Some(patterns) = BOXES.get(&key.abs()).filter(|p| !p.is_empty()) else { break };

            let found = patterns.iter().find_map(|pattern| {
                let adjusted: Vec<i32> = if key > 0 { pattern.clone() } else { pattern.iter().map(|&v| -v).collect() };
                let end = idx + 1 + adjusted.len();
                (end <= path.len() && path[idx + 1..end] == adjusted).then(|| (end - 1, *adjusted.last().unwrap()))
            });

            if let Some((new_idx, new_key)) = found {
                level += 1;
                idx = new_idx;
                key = new_key;
            } else {
                break;
            }
        }
        level.max(1)
    }
}
