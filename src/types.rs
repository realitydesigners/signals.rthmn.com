use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Box {
    pub high: f64,
    pub low: f64,
    pub value: f64,
}

#[derive(Debug, Clone)]
pub struct BoxData {
    pub pair: String,
    pub boxes: Vec<Box>,
    pub price: f64,
    pub timestamp: String,
}

#[derive(Debug, Clone)]
pub struct TraversalPath { pub path: Vec<i32> }

impl TraversalPath {
    pub fn length(&self) -> usize { self.path.len() }
    pub fn starting_point(&self) -> i32 { self.path[0] }
    pub fn signal_type(&self) -> SignalType { if self.path[0] > 0 { SignalType::LONG } else { SignalType::SHORT } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SignalType { #[default] LONG, SHORT }

impl std::fmt::Display for SignalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self { Self::LONG => "LONG", Self::SHORT => "SHORT" })
    }
}

#[derive(Debug, Clone)]
pub struct PatternMatch {
    pub pair: String,
    pub level: u32,
    pub traversal_path: TraversalPath,
    pub full_pattern: Vec<i32>,
    pub box_details: Vec<BoxDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxDetail {
    pub integer_value: i32,
    pub high: f64,
    pub low: f64,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalMessage { 
    pub id: Option<i64>, // Supabase id (set after insert)
    pub pair: String, 
    pub signal_type: String, 
    pub level: u32, 
    pub pattern_sequence: Vec<i32>, 
    pub box_details: Vec<BoxDetail>, 
    pub complete_box_snapshot: Vec<i32>, 
    pub entry: Option<f64>,
    pub stop_losses: Vec<f64>,
    pub targets: Vec<f64>,
    pub risk_reward: Vec<f64>,
}

