use signals_rthmn::{
    deduplication::Deduplicator,
    scanner::MarketScanner,
    signal::SignalGenerator,
    supabase::SupabaseClient,
    tracker::{ActiveSignal, SignalTracker},
    types::{BoxData, PatternMatch, SignalMessage, SignalType},
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use std::{collections::HashMap, env, sync::Arc};
use tokio::sync::{mpsc, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, info, warn};

pub struct AppState {
    scanner: RwLock<MarketScanner>,
    generator: SignalGenerator,
    tracker: SignalTracker,
    deduplicator: Deduplicator,
    box_data: RwLock<HashMap<String, BoxData>>,
    signals_sent: RwLock<u64>,
    main_server_url: String,
    signal_tx: mpsc::Sender<SignalMessage>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("signals_rthmn=info")
        .init();
    dotenvy::dotenv().ok();

    info!("==================================================");
    info!("  SIGNALS.RTHMN.COM - Rust Edition");
    info!("  Supabase-only signals + server-side matching");
    info!("==================================================");

    let port: u16 = env::var("PORT")
        .unwrap_or("3003".into())
        .parse()
        .unwrap_or(3003);
    let main_server_url =
        env::var("MAIN_SERVER_URL").unwrap_or("https://server.rthmn.com".into());
    let auth_token = env::var("SUPABASE_SERVICE_ROLE_KEY").expect("SUPABASE_SERVICE_ROLE_KEY required");

    // Supabase configuration
    let supabase_url = env::var("SUPABASE_URL").expect("SUPABASE_URL required");
    let supabase_key = auth_token.clone();

    info!("Supabase URL: {}", supabase_url);
    info!("Main server URL: {}", main_server_url);

    // Initialize scanner
    let mut scanner = MarketScanner::default();
    scanner.initialize();
    info!("MarketScanner initialized with {} paths", scanner.path_count());

    // Initialize clients
    let supabase = SupabaseClient::new(&supabase_url, &supabase_key);
    let tracker = SignalTracker::new(supabase);
    info!("SignalTracker initialized");

    let (signal_tx, signal_rx) = mpsc::channel::<SignalMessage>(1000);

    let state = Arc::new(AppState {
        scanner: RwLock::new(scanner),
        generator: SignalGenerator::default(),
        tracker,
        deduplicator: Deduplicator::new(),
        box_data: RwLock::new(HashMap::new()),
        signals_sent: RwLock::new(0),
        main_server_url,
        signal_tx,
    });

    // HTTP client that forwards raw signals to main server immediately
    let s = Arc::clone(&state);
    tokio::spawn(async move {
        main_server_forwarder(s, auth_token, signal_rx).await;
    });

    // HTTP + WebSocket server
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/status", get(status))
        .route("/ws", get(ws_handler))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    info!("Server running on port {} (WebSocket at /ws)", port);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "signals.rthmn.com (rust)",
        "timestamp": Utc::now().to_rfc3339()
    }))
}

async fn status(State(s): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let scanner = s.scanner.read().await;
    let signals = *s.signals_sent.read().await;
    let active_signals = s.tracker.get_active_count().await;
    let active_by_pair = s.tracker.get_active_by_pair().await;

    Json(serde_json::json!({
        "scanner": {
            "totalPaths": scanner.path_count(),
            "isInitialized": true
        },
        "signalsSent": signals,
        "activeSignals": {
            "total": active_signals,
            "byPair": active_by_pair
        }
    }))
}

/// WebSocket handler - receives box updates from boxes.rthmn.com
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    info!("WebSocket client connected (boxes.rthmn.com)");

    // Send auth required
    let auth_msg = rmp_serde::to_vec(&serde_json::json!({"type": "authRequired"})).unwrap();
    let _ = sender.send(Message::Binary(auth_msg.into())).await;

    let mut authenticated = false;

    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Binary(data)) => {
                if let Ok(m) = rmp_serde::from_slice::<serde_json::Value>(&data) {
                    match m.get("type").and_then(|v| v.as_str()) {
                        Some("auth") => {
                            // Accept any auth for now (boxes.rthmn.com uses service key)
                            authenticated = true;
                            let welcome =
                                rmp_serde::to_vec(&serde_json::json!({"type": "welcome"})).unwrap();
                            let _ = sender.send(Message::Binary(welcome.into())).await;
                            info!("boxes.rthmn.com authenticated");
                        }
                        Some("boxUpdate") if authenticated => {
                            if let (Some(pair), Some(data)) =
                                (m.get("pair").and_then(|v| v.as_str()), m.get("data"))
                            {
                                debug!("Received boxUpdate for {}", pair);
                                process_box_update(&state, pair, data).await;
                            }
                        }
                        Some("heartbeat") => {
                            // Acknowledge heartbeat
                        }
                        _ => {}
                    }
                }
            }
            Ok(Message::Close(_)) => break,
            Err(e) => {
                warn!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }
    info!("WebSocket client disconnected");
}

/// Forwards raw signals to main server via HTTP (no batching)
async fn main_server_forwarder(
    state: Arc<AppState>,
    token: String,
    mut signal_rx: mpsc::Receiver<SignalMessage>,
) {
    let client = reqwest::Client::new();
    loop {
        let Some(signal) = signal_rx.recv().await else { break };

        let url = format!("{}/signals/raw", state.main_server_url.trim_end_matches('/'));
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&signal)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                *state.signals_sent.write().await += 1;
                info!(
                    "Forwarded raw signal to main server: {} {} L{}",
                    signal.pair, signal.signal_type, signal.level
                );
            }
            Ok(resp) => {
                warn!(
                    "Failed to forward raw signal to main server: {}",
                    resp.status()
                );
            }
            Err(e) => {
                warn!("Failed to forward raw signal to main server: {}", e);
            }
        }
    }
}

async fn process_box_update(state: &Arc<AppState>, pair: &str, data: &serde_json::Value) {
    let boxes: Vec<signals_rthmn::types::Box> = data
        .get("boxes")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let price = data.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let timestamp = data
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if boxes.is_empty() {
        return;
    }

    // Store box data
    state.box_data.write().await.insert(
        pair.to_string(),
        BoxData {
            pair: pair.to_string(),
            boxes: boxes.clone(),
            price,
            timestamp,
        },
    );

    // CHECK ACTIVE SIGNALS AGAINST CURRENT PRICE
    // This will settle any signals that hit their SL or TP
    let settlements = state.tracker.check_price(pair, price).await;
    if !settlements.is_empty() {
        info!(
            "{} @ ${:.5} - {} signal(s) settled",
            pair,
            price,
            settlements.len()
        );
        
        // Remove settled L1 signals from deduplicator
        for settlement in &settlements {
            if settlement.signal.level == 1 {
                state
                    .deduplicator
                    .remove_l1_signal(pair, &settlement.signal.signal_type.to_string())
                    .await;
            }
        }
    }

    // Detect patterns and generate new signals
    let all_patterns = state.scanner.read().await.detect_patterns(pair, &boxes);
    if all_patterns.is_empty() {
        // Debug: log when no patterns detected
        let (point, _) = signals_rthmn::instruments::get_instrument_config(pair);
        let integer_values: Vec<i32> = boxes.iter().map(|b| (b.value / point).round() as i32).collect();
        debug!("{}: No patterns detected. Box integer values: {:?}", pair, integer_values);
        return;
    }
    
    info!("{}: Detected {} pattern(s)", pair, all_patterns.len());

    let timestamp_ms = chrono::Utc::now().timestamp_millis();

    // Filter patterns through deduplicator
    let mut filtered_patterns = Vec::new();
    for pattern in &all_patterns {
        if !state
            .deduplicator
            .should_filter_pattern(pair, pattern, &boxes, timestamp_ms)
            .await
        {
            filtered_patterns.push(pattern.clone());
        }
    }

    if filtered_patterns.is_empty() {
        debug!("{}: All {} pattern(s) filtered by deduplicator", pair, all_patterns.len());
        return;
    }
    
    info!("{}: {} pattern(s) passed deduplication", pair, filtered_patterns.len());

    // Group patterns by sequence and prefer highest level
    let mut pattern_groups: HashMap<String, PatternMatch> = HashMap::new();
    for pattern in filtered_patterns {
        let key = pattern
            .traversal_path
            .path
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("_");
        
        if let Some(existing) = pattern_groups.get(&key) {
            if pattern.level > existing.level {
                pattern_groups.insert(key, pattern);
            }
        } else {
            pattern_groups.insert(key, pattern);
        }
    }

    let unique_patterns: Vec<_> = pattern_groups.into_values().collect();
    info!("{} @ ${:.2} - {} pattern(s) after deduplication", pair, price, unique_patterns.len());

    for signal in state
        .generator
        .generate_signals(pair, &unique_patterns, &boxes, price)
    {
        // Find a valid trade opportunity
        let valid_trade = signal
            .data
            .trade_opportunities
            .iter()
            .find(|t| t.is_valid);

        let Some(trade) = valid_trade else {
            continue;
        };

        let entry = trade.entry.unwrap_or(0.0);
        let stop_loss = trade.stop_loss.unwrap_or(0.0);
        let target = trade.target.unwrap_or(0.0);

        // Check if we've sent this exact signal recently (same pattern + level + prices)
        if state
            .deduplicator
            .should_filter_recent_signal(
                pair,
                &signal.pattern_sequence,
                signal.level,
                entry,
                stop_loss,
                target,
                signal.timestamp,
            )
            .await
        {
            info!(
                "FILTERED: {} {} L{} - duplicate signal within time window",
                signal.pair, signal.signal_type, signal.level
            );
            continue;
        }

        // Log the signal details
        info!(
            "SIGNAL: {} {} L{} {:?}",
            signal.pair, signal.signal_type, signal.level, signal.pattern_sequence
        );
        for (i, b) in signal.data.box_details.iter().enumerate() {
            info!(
                "  Box {}: {} H:{:.5} L:{:.5}",
                i + 1,
                b.integer_value,
                b.high,
                b.low
            );
        }
        info!(
            "  {} E:{:.5} S:{:.5} T:{:.5} R:R:{:.2}",
            trade.rule_id,
            entry,
            stop_loss,
            target,
            trade.risk_reward_ratio.unwrap_or(0.0)
        );

        // Create active signal for tracking
        let active_signal = ActiveSignal {
            signal_id: signal.signal_id.clone(),
            pair: pair.to_string(),
            signal_type: if signal.signal_type == "LONG" {
                SignalType::LONG
            } else {
                SignalType::SHORT
            },
            level: signal.level,
            entry,
            stop_loss,
            target,
            risk_reward_ratio: trade.risk_reward_ratio,
            pattern_sequence: signal.pattern_sequence.clone(),
            created_at: signal.timestamp,
        };

        // Add to tracker (writes to Convex)
        state.tracker.add_signal(active_signal).await;

        // Immediately forward raw signal JSON to the main server (no batching)
        let _ = state.signal_tx.send(signal).await;
    }
}
