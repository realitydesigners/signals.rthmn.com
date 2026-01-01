use crate::types::{BoxDetail, PatternMatch, SignalMessage, SignalType};
use lazy_static::lazy_static;

#[derive(Debug, Clone, Copy)]
pub enum PricePoint { HIGH, LOW, MID }

#[derive(Debug, Clone)]
pub struct TradeRule {
    pub id: &'static str,
    pub level: u32,
    pub entry_box: usize,
    pub entry_point: PricePoint,
    pub stop_boxes: Vec<usize>,
    pub stop_point: PricePoint,
    pub target_boxes: Vec<usize>,
    pub target_point: PricePoint,
}

// Trade rules configuration:
// - Levels represent pattern reversals: L1 = 1 reversal, L2 = 2 reversals, etc.
// - Boxes are 0-indexed and sorted by absolute value descending (Box 0 = largest).
// - LONG: Entry at entry_box HIGH, Stop at stop_box LOW, Targets cumulative from box 0 HIGH
// - SHORT: Entry at entry_box LOW, Stop at stop_box HIGH, Targets cumulative from box 0 LOW

lazy_static! {
    static ref LONG_RULES: Vec<TradeRule> = vec![
        TradeRule { 
            id: "L1_RULE_1", 
            level: 1, 
            entry_box: 1, 
            entry_point: PricePoint::HIGH, 
            stop_boxes: vec![0], 
            stop_point: PricePoint::LOW, 
            target_boxes: vec![0],
            target_point: PricePoint::HIGH,
        },
        TradeRule { 
            id: "L2_RULE_1", 
            level: 2, 
            entry_box: 2, 
            entry_point: PricePoint::HIGH, 
            stop_boxes: vec![1], 
            stop_point: PricePoint::LOW, 
            target_boxes: vec![0, 1], 
            target_point: PricePoint::HIGH,
        },
        TradeRule { 
            id: "L3_RULE_1", 
            level: 3, 
            entry_box: 3, 
            entry_point: PricePoint::HIGH, 
            stop_boxes: vec![2], 
            stop_point: PricePoint::LOW, 
            target_boxes: vec![0, 1, 2], 
            target_point: PricePoint::HIGH,
        },
        TradeRule { 
            id: "L4_RULE_1", 
            level: 4, 
            entry_box: 4, 
            entry_point: PricePoint::HIGH, 
            stop_boxes: vec![3], 
            stop_point: PricePoint::LOW, 
            target_boxes: vec![0, 1, 2, 3], 
            target_point: PricePoint::HIGH,
        },
        TradeRule { 
            id: "L5_RULE_1", 
            level: 5, 
            entry_box: 5, 
            entry_point: PricePoint::HIGH, 
            stop_boxes: vec![4], 
            stop_point: PricePoint::LOW, 
            target_boxes: vec![0, 1, 2, 3, 4], 
            target_point: PricePoint::HIGH,
        },
        TradeRule { 
            id: "L6_RULE_1", 
            level: 6, 
            entry_box: 6, 
            entry_point: PricePoint::HIGH, 
            stop_boxes: vec![5], 
            stop_point: PricePoint::LOW, 
            target_boxes: vec![0, 1, 2, 3, 4, 5], 
            target_point: PricePoint::HIGH,
        },
    ];

    static ref SHORT_RULES: Vec<TradeRule> = vec![
        TradeRule { 
            id: "L1_RULE_1", 
            level: 1, 
            entry_box: 1, 
            entry_point: PricePoint::LOW, 
            stop_boxes: vec![0], 
            stop_point: PricePoint::HIGH, 
            target_boxes: vec![0], 
            target_point: PricePoint::LOW,
        },
        TradeRule { 
            id: "L2_RULE_1", 
            level: 2, 
            entry_box: 2, 
            entry_point: PricePoint::LOW, 
            stop_boxes: vec![1], 
            stop_point: PricePoint::HIGH, 
            target_boxes: vec![0, 1], 
            target_point: PricePoint::LOW,
        },
        TradeRule { 
            id: "L3_RULE_1", 
            level: 3, 
            entry_box: 3, 
            entry_point: PricePoint::LOW, 
            stop_boxes: vec![2], 
            stop_point: PricePoint::HIGH, 
            target_boxes: vec![0, 1, 2], 
            target_point: PricePoint::LOW,
        },
        TradeRule { 
            id: "L4_RULE_1", 
            level: 4, 
            entry_box: 4, 
            entry_point: PricePoint::LOW, 
            stop_boxes: vec![3], 
            stop_point: PricePoint::HIGH, 
            target_boxes: vec![0, 1, 2, 3], 
            target_point: PricePoint::LOW,
        },
        TradeRule { 
            id: "L5_RULE_1", 
            level: 5, 
            entry_box: 5, 
            entry_point: PricePoint::LOW, 
            stop_boxes: vec![4], 
            stop_point: PricePoint::HIGH, 
            target_boxes: vec![0, 1, 2, 3, 4], 
            target_point: PricePoint::LOW,
        },
        TradeRule { 
            id: "L6_RULE_1", 
            level: 6, 
            entry_box: 6, 
            entry_point: PricePoint::LOW, 
            stop_boxes: vec![5], 
            stop_point: PricePoint::HIGH, 
            target_boxes: vec![0, 1, 2, 3, 4, 5], 
            target_point: PricePoint::LOW,
        },
    ];
}

fn get_rules(signal_type: SignalType) -> &'static [TradeRule] {
    match signal_type { 
        SignalType::LONG => &LONG_RULES, 
        SignalType::SHORT => &SHORT_RULES 
    }
}

#[derive(Default)]
pub struct SignalGenerator;

impl SignalGenerator {
    pub fn generate_signals(&self, pair: &str, patterns: &[PatternMatch], _boxes: &[crate::types::Box], _price: f64) -> Vec<SignalMessage> {
        patterns.iter()
            .filter(|p| get_rules(p.traversal_path.signal_type()).iter().any(|r| r.level == p.level))
            .map(|p| self.create_signal(pair, p))
            .collect()
    }

    fn create_signal(&self, pair: &str, pattern: &PatternMatch) -> SignalMessage {
        let _path_str = pattern.traversal_path.path.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("_");
        
        let sig_type = pattern.traversal_path.signal_type();
        let mut primary: Vec<&BoxDetail> = pattern.box_details.iter()
            .filter(|b| matches!(sig_type, SignalType::LONG if b.integer_value > 0) || matches!(sig_type, SignalType::SHORT if b.integer_value < 0))
            .collect();
        primary.sort_by(|a, b| b.integer_value.abs().cmp(&a.integer_value.abs()));

        let rule = get_rules(sig_type).iter()
            .find(|r| r.level == pattern.level);

        let (entry, stop_losses, targets, risk_reward) = if let Some(rule) = rule {
            let entry = get_price(&primary, rule.entry_box, rule.entry_point);
            
            let stop_losses: Vec<f64> = rule.stop_boxes.iter()
                .filter_map(|&box_idx| get_price(&primary, box_idx, rule.stop_point))
                .collect();
            
            let targets = rule.target_boxes.first().and_then(|&first_box_idx| {
                get_price(&primary, first_box_idx, rule.target_point).map(|base| {
                    let mut calculated_targets = Vec::new();
                    
                    // Get first box size for the last target calculation
                    let first_box_size = primary.get(first_box_idx)
                        .map(|b| b.high - b.low)
                        .unwrap_or(0.0);
                    
                    // All targets except the last: direct HIGH/LOW values of each box
                    for &box_idx in &rule.target_boxes {
                        if let Some(box_detail) = primary.get(box_idx) {
                            let target = match rule.target_point {
                                PricePoint::HIGH => box_detail.high,
                                PricePoint::LOW => box_detail.low,
                                PricePoint::MID => (box_detail.high + box_detail.low) / 2.0,
                            };
                            calculated_targets.push(target);
                        }
                    }
                    
                    // Last target (highest/furthest): base + first box size for LONG, base - first box size for SHORT
                    let last_target = match sig_type {
                        SignalType::LONG => base + first_box_size,
                        SignalType::SHORT => base - first_box_size,
                    };
                    calculated_targets.push(last_target);
                    
                    // Sort targets: closest to furthest
                    // LONG: ascending (smallest/closest first, largest/furthest last)
                    // SHORT: descending (largest/closest first, smallest/furthest last)
                    match sig_type {
                        SignalType::LONG => calculated_targets.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)),
                        SignalType::SHORT => calculated_targets.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal)),
                    }
                    
                    calculated_targets
                })
            }).unwrap_or_default();
            
            let risk_reward: Vec<f64> = entry.zip(stop_losses.first().copied()).map_or_else(
                Vec::new,
                |(e, s)| {
                    let risk = (e - s).abs();
                    if risk > 0.0 {
                        targets.iter()
                            .map(|&t| {
                                let reward = match sig_type {
                                    SignalType::LONG => (t - e).abs(),
                                    SignalType::SHORT => (e - t).abs(),
                                };
                                (reward / risk).round()
                            })
                            .collect()
                    } else {
                        vec![]
                    }
                }
            );
            
            (entry, stop_losses, targets, risk_reward)
        } else {
            (None, vec![], vec![], vec![])
        };
        
        SignalMessage {
            id: None, // Will be set after Supabase insert
            pair: pair.to_string(),
            signal_type: pattern.traversal_path.signal_type().to_string(),
            level: pattern.level,
            pattern_sequence: pattern.traversal_path.path.clone(),
            box_details: pattern.box_details.clone(),
            complete_box_snapshot: pattern.full_pattern.clone(),
            entry,
            stop_losses,
            targets,
            risk_reward,
        }
    }
}

fn get_price(boxes: &[&BoxDetail], idx: usize, point: PricePoint) -> Option<f64> {
    boxes.get(idx).map(|b| match point {
        PricePoint::HIGH => b.high,
        PricePoint::LOW => b.low,
        PricePoint::MID => (b.high + b.low) / 2.0,
    })
}

