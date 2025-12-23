use crate::types::{BoxDetail, PatternMatch, SignalData, SignalMessage, SignalType, TradeOpportunity};
use chrono::Utc;

#[derive(Debug, Clone, Copy)]
pub enum PricePoint { HIGH, LOW, MID }

#[derive(Debug, Clone)]
pub struct TradeRule {
    pub id: &'static str,
    pub level: u32,
    pub entry_box: usize,
    pub entry_point: PricePoint,
    pub stop_box: usize,
    pub stop_point: PricePoint,
    pub target_box: usize,
    pub target_point: PricePoint,
}

// ============================================================================
// TRADE RULES
// ============================================================================
// 
// LEVELS EXPLAINED:
// A "level" counts how many complete pattern reversals occur in the traversal.
// 
// - L1 = 1 reversal  (start key → pattern → end)
// - L2 = 2 reversals (start → pattern → new key → pattern → end)
// - L3 = 3 reversals (three complete pattern traversals)
// - L4 = 4 reversals (four complete pattern traversals)
// 
// Each reversal follows the BOXES map: given a starting key (e.g. 267), 
// look up valid patterns like [-231, 130]. If the live boxes contain that
// sequence, it's one complete reversal. The last value becomes the new key
// for the next potential reversal.
// new tech companies
// Higher levels = deeper fractal structure = stronger/rarer signals.
//
// ============================================================================
// BOX ORDERING:
// Boxes are sorted by absolute value descending:
//   Box 1 = largest (primary direction)
//   Box 2 = second largest
//   Box 3 = third largest
//   etc.
//
// ============================================================================
// LONG RULES (buy setups):
//   Entry = break above entry_box HIGH
//   Stop  = entry_box LOW
//   Target = box 1 HIGH + box 1 size (high + (high - low))
//
// SHORT RULES (sell setups):
//   Entry = break below entry_box LOW
//   Stop  = entry_box HIGH
//   Target = box 1 LOW - box 1 size (low - (high - low))
//
// ACTIVE LEVELS:
//   L1 → entry/stop at box 2
//   L2 → entry/stop at box 3
//   L3 → entry/stop at box 4
//   L4 → entry/stop at box 5
//   L5 → entry/stop at box 6
//   L6 → entry/stop at box 7
// ============================================================================

const LONG_RULES: &[TradeRule] = &[
    TradeRule { 
        id: "L1_RULE_1", 
        level: 1, 
        entry_box: 2, 
        entry_point: PricePoint::HIGH, 
        stop_box: 2, 
        stop_point: PricePoint::LOW, 
        target_box: 1, 
        target_point: PricePoint::HIGH,
    },
    TradeRule { 
        id: "L2_RULE_1", 
        level: 2, 
        entry_box: 3, 
        entry_point: PricePoint::HIGH, 
        stop_box: 3, 
        stop_point: PricePoint::LOW, 
        target_box: 1, 
        target_point: PricePoint::HIGH,
    },
    TradeRule { 
        id: "L3_RULE_1", 
        level: 3, 
        entry_box: 4, 
        entry_point: PricePoint::HIGH, 
        stop_box: 4, 
        stop_point: PricePoint::LOW, 
        target_box: 1, 
        target_point: PricePoint::HIGH,
    },
    TradeRule { 
        id: "L4_RULE_1", 
        level: 4, 
        entry_box: 5, 
        entry_point: PricePoint::HIGH, 
        stop_box: 5, 
        stop_point: PricePoint::LOW, 
        target_box: 1, 
        target_point: PricePoint::HIGH,
    },
    TradeRule { 
        id: "L5_RULE_1", 
        level: 5, 
        entry_box: 6, 
        entry_point: PricePoint::HIGH, 
        stop_box: 6, 
        stop_point: PricePoint::LOW, 
        target_box: 1, 
        target_point: PricePoint::HIGH,
    },
    TradeRule { 
        id: "L6_RULE_1", 
        level: 6, 
        entry_box: 7, 
        entry_point: PricePoint::HIGH, 
        stop_box: 7, 
        stop_point: PricePoint::LOW, 
        target_box: 1, 
        target_point: PricePoint::HIGH,
    },
];

const SHORT_RULES: &[TradeRule] = &[
    TradeRule { 
        id: "L1_RULE_1", 
        level: 1, 
        entry_box: 2, 
        entry_point: PricePoint::LOW, 
        stop_box: 2, 
        stop_point: PricePoint::HIGH, 
        target_box: 1, 
        target_point: PricePoint::LOW,
    },
    TradeRule { 
        id: "L2_RULE_1", 
        level: 2, 
        entry_box: 3, 
        entry_point: PricePoint::LOW, 
        stop_box: 3, 
        stop_point: PricePoint::HIGH, 
        target_box: 1, 
        target_point: PricePoint::LOW,
    },
    TradeRule { 
        id: "L3_RULE_1", 
        level: 3, 
        entry_box: 4, 
        entry_point: PricePoint::LOW, 
        stop_box: 4, 
        stop_point: PricePoint::HIGH, 
        target_box: 1, 
        target_point: PricePoint::LOW,
    },
    TradeRule { 
        id: "L4_RULE_1", 
        level: 4, 
        entry_box: 5, 
        entry_point: PricePoint::LOW, 
        stop_box: 5, 
        stop_point: PricePoint::HIGH, 
        target_box: 1, 
        target_point: PricePoint::LOW,
    },
    TradeRule { 
        id: "L5_RULE_1", 
        level: 5, 
        entry_box: 6, 
        entry_point: PricePoint::LOW, 
        stop_box: 6, 
        stop_point: PricePoint::HIGH, 
        target_box: 1, 
        target_point: PricePoint::LOW,
    },
    TradeRule { 
        id: "L6_RULE_1", 
        level: 6, 
        entry_box: 7, 
        entry_point: PricePoint::LOW, 
        stop_box: 7, 
        stop_point: PricePoint::HIGH, 
        target_box: 1, 
        target_point: PricePoint::LOW,
    },
];

fn get_rules(signal_type: SignalType) -> &'static [TradeRule] {
    match signal_type { SignalType::LONG => LONG_RULES, SignalType::SHORT => SHORT_RULES }
}

#[derive(Default)]
pub struct SignalGenerator;

impl SignalGenerator {
    pub fn generate_signals(&self, pair: &str, patterns: &[PatternMatch], _boxes: &[crate::types::Box], _price: f64) -> Vec<SignalMessage> {
        patterns.iter()
            .filter(|p| get_rules(p.traversal_path.signal_type).iter().any(|r| r.level == p.level))
            .map(|p| self.create_signal(pair, p))
            .collect()
    }

    fn create_signal(&self, pair: &str, pattern: &PatternMatch) -> SignalMessage {
        let now = Utc::now().timestamp_millis();
        let path_str = pattern.traversal_path.path.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("_");
        
        SignalMessage {
            signal_id: format!("{}_{}_{}_{}_{}", pair, pattern.traversal_path.signal_type, path_str, pattern.level, now),
            pair: pair.to_string(),
            signal_type: pattern.traversal_path.signal_type.to_string(),
            level: pattern.level,
            pattern_sequence: pattern.traversal_path.path.clone(),
            timestamp: now,
            data: SignalData {
                box_details: pattern.box_details.clone(),
                trade_opportunities: self.calculate_opportunities(pattern),
                complete_box_snapshot: pattern.full_pattern.clone(),
                has_trade_rules: true,
            },
        }
    }

    pub fn calculate_opportunities(&self, pattern: &PatternMatch) -> Vec<TradeOpportunity> {
        let sig_type = pattern.traversal_path.signal_type;
        let mut primary: Vec<&BoxDetail> = pattern.box_details.iter()
            .filter(|b| matches!(sig_type, SignalType::LONG if b.integer_value > 0) || matches!(sig_type, SignalType::SHORT if b.integer_value < 0))
            .collect();
        primary.sort_by(|a, b| b.integer_value.abs().cmp(&a.integer_value.abs()));

        get_rules(sig_type).iter()
            .filter(|r| r.level == pattern.level)
            .map(|rule| {
                let entry = get_price(&primary, rule.entry_box, rule.entry_point);
                let stop = get_price(&primary, rule.stop_box, rule.stop_point);
                let target_base = get_price(&primary, rule.target_box, rule.target_point);
                let target = target_base.and_then(|base| {
                    primary.get(rule.target_box.saturating_sub(1)).map(|box1| {
                        let box_size = box1.high - box1.low;
                        match sig_type {
                            SignalType::LONG => base + box_size,
                            SignalType::SHORT => base - box_size,
                        }
                    })
                });
                let rr = entry.zip(stop).zip(target).and_then(|((e, s), t)| {
                    let risk = (e - s).abs();
                    (risk > 0.0).then(|| (t - e).abs() / risk)
                });
                TradeOpportunity {
                    rule_id: rule.id.to_string(),
                    level: rule.level,
                    entry, stop_loss: stop, target,
                    risk_reward_ratio: rr,
                    is_valid: entry.is_some() && stop.is_some() && target.is_some(),
                }
            })
            .collect()
    }
}

fn get_price(boxes: &[&BoxDetail], idx: usize, point: PricePoint) -> Option<f64> {
    boxes.get(idx.saturating_sub(1)).map(|b| match point {
        PricePoint::HIGH => b.high,
        PricePoint::LOW => b.low,
        PricePoint::MID => (b.high + b.low) / 2.0,
    })
}

