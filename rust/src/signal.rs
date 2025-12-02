use crate::types::{Box, BoxDetail, PatternMatch, SignalData, SignalMessage, SignalType, TradeOpportunity};
use chrono::Utc;

/// Trade rules configuration
#[derive(Debug, Clone)]
pub struct TradeRule {
    pub id: String,
    pub level: u32,
    pub entry_box: usize,
    pub entry_point: PricePoint,
    pub stop_box: usize,
    pub stop_point: PricePoint,
    pub target_box: usize,
    pub target_point: PricePoint,
    pub enabled: bool,
    pub alert: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum PricePoint {
    HIGH,
    LOW,
    MID,
}

lazy_static::lazy_static! {
    static ref LONG_RULES: Vec<TradeRule> = vec![
        TradeRule { id: "L1_RULE_1".into(), level: 1, entry_box: 2, entry_point: PricePoint::HIGH, stop_box: 2, stop_point: PricePoint::LOW, target_box: 1, target_point: PricePoint::HIGH, enabled: true, alert: true },
        TradeRule { id: "L3_RULE_1".into(), level: 3, entry_box: 4, entry_point: PricePoint::HIGH, stop_box: 4, stop_point: PricePoint::LOW, target_box: 1, target_point: PricePoint::HIGH, enabled: true, alert: true },
        TradeRule { id: "L4_RULE_1".into(), level: 4, entry_box: 5, entry_point: PricePoint::HIGH, stop_box: 5, stop_point: PricePoint::LOW, target_box: 1, target_point: PricePoint::HIGH, enabled: true, alert: true },
    ];
    
    static ref SHORT_RULES: Vec<TradeRule> = vec![
        TradeRule { id: "L1_RULE_1".into(), level: 1, entry_box: 2, entry_point: PricePoint::LOW, stop_box: 2, stop_point: PricePoint::HIGH, target_box: 1, target_point: PricePoint::LOW, enabled: true, alert: true },
        TradeRule { id: "L3_RULE_1".into(), level: 3, entry_box: 4, entry_point: PricePoint::LOW, stop_box: 4, stop_point: PricePoint::HIGH, target_box: 1, target_point: PricePoint::LOW, enabled: true, alert: true },
        TradeRule { id: "L4_RULE_1".into(), level: 4, entry_box: 5, entry_point: PricePoint::LOW, stop_box: 5, stop_point: PricePoint::HIGH, target_box: 1, target_point: PricePoint::LOW, enabled: true, alert: true },
    ];
}

/// SignalGenerator - Generates trading signals from pattern matches
pub struct SignalGenerator {
    signals_generated: u64,
}

impl SignalGenerator {
    pub fn new() -> Self {
        Self { signals_generated: 0 }
    }

    pub fn generate_signals(
        &self,
        pair: &str,
        patterns: &[PatternMatch],
        boxes: &[Box],
        price: f64,
    ) -> Vec<SignalMessage> {
        let mut signals = Vec::new();

        for pattern in patterns {
            // Check if this level should alert
            if !self.should_alert(pattern.level, &pattern.traversal_path.signal_type) {
                continue;
            }

            let signal = self.create_signal(pair, pattern, boxes, price);
            signals.push(signal);
        }

        signals
    }

    fn should_alert(&self, level: u32, signal_type: &SignalType) -> bool {
        let rules = match signal_type {
            SignalType::LONG => &*LONG_RULES,
            SignalType::SHORT => &*SHORT_RULES,
        };
        rules.iter().any(|r| r.level == level && r.alert)
    }

    fn create_signal(
        &self,
        pair: &str,
        pattern: &PatternMatch,
        boxes: &[Box],
        price: f64,
    ) -> SignalMessage {
        let signal_id = format!(
            "{}_{}_{}_{}_L{}",
            pair,
            pattern.traversal_path.signal_type,
            pattern.traversal_path.path.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("_"),
            pattern.level,
            Utc::now().timestamp_millis()
        );

        let custom_pattern = pattern.full_pattern
            .iter()
            .map(|&n| if n > 0 { "1" } else { "0" })
            .collect::<Vec<_>>()
            .join("");

        // Get trade opportunities
        let trade_opportunities = self.calculate_trade_opportunities(pattern, boxes);

        SignalMessage {
            signal_id,
            pair: pair.to_string(),
            signal_type: pattern.traversal_path.signal_type.to_string(),
            level: pattern.level,
            custom_pattern: Some(custom_pattern),
            pattern_sequence: pattern.traversal_path.path.clone(),
            timestamp: Utc::now().timestamp_millis(),
            data: SignalData {
                box_details: pattern.box_details.clone(),
                trade_opportunities,
                complete_box_snapshot: pattern.full_pattern.clone(),
                has_trade_rules: true,
            },
        }
    }

    fn calculate_trade_opportunities(
        &self,
        pattern: &PatternMatch,
        boxes: &[Box],
    ) -> Vec<TradeOpportunity> {
        let rules = match pattern.traversal_path.signal_type {
            SignalType::LONG => &*LONG_RULES,
            SignalType::SHORT => &*SHORT_RULES,
        };

        let active_rules: Vec<&TradeRule> = rules
            .iter()
            .filter(|r| r.level == pattern.level && r.enabled)
            .collect();

        // Get primary boxes (same direction as signal)
        let primary_boxes: Vec<&BoxDetail> = pattern.box_details
            .iter()
            .filter(|b| match pattern.traversal_path.signal_type {
                SignalType::LONG => b.integer_value > 0,
                SignalType::SHORT => b.integer_value < 0,
            })
            .collect();

        let mut opportunities = Vec::new();

        for rule in active_rules {
            let entry = self.get_price(&primary_boxes, rule.entry_box, rule.entry_point);
            let stop_loss = self.get_price(&primary_boxes, rule.stop_box, rule.stop_point);
            let target = self.get_price(&primary_boxes, rule.target_box, rule.target_point);

            let risk_reward = if let (Some(e), Some(s), Some(t)) = (entry, stop_loss, target) {
                let risk = (e - s).abs();
                let reward = (t - e).abs();
                if risk > 0.0 { Some(reward / risk) } else { None }
            } else {
                None
            };

            opportunities.push(TradeOpportunity {
                rule_id: rule.id.clone(),
                level: rule.level,
                entry,
                stop_loss,
                target,
                risk_reward_ratio: risk_reward,
                is_valid: entry.is_some() && stop_loss.is_some() && target.is_some(),
            });
        }

        opportunities
    }

    fn get_price(&self, boxes: &[&BoxDetail], box_index: usize, point: PricePoint) -> Option<f64> {
        // Sort by absolute value descending
        let mut sorted = boxes.to_vec();
        sorted.sort_by(|a, b| b.integer_value.abs().cmp(&a.integer_value.abs()));

        let idx = box_index.saturating_sub(1);
        sorted.get(idx).map(|b| match point {
            PricePoint::HIGH => b.high,
            PricePoint::LOW => b.low,
            PricePoint::MID => (b.high + b.low) / 2.0,
        })
    }
}

