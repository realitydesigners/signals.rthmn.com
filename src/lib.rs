pub mod deduplication;
pub mod instruments;
pub mod patterns;
pub mod scanner;
pub mod signal;
pub mod supabase;
pub mod tracker;
pub mod types;

// Re-export for tests
pub use scanner::MarketScanner;
