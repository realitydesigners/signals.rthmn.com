use signals_rthmn::{
    deduplication::Deduplicator,
    scanner::MarketScanner,
    signal::SignalGenerator,
    supabase::SupabaseClient,
    tracker::{ActiveSignal, SignalTracker},
    types::{SignalMessage, SignalType},
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
use std::{env, sync::Arc};
use tokio::sync::{mpsc, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, info, warn};

pub struct AppState {
    scanner: RwLock<MarketScanner>,
    generator: SignalGenerator,
    tracker: SignalTracker,
    deduplicator: Deduplicator,
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
    info!("==================================================");

    let port: u16 = env::var("PORT")
        .unwrap_or("3003".into())
        .parse()
        .unwrap_or(3003);
    let main_server_url =
        env::var("MAIN_SERVER_URL").unwrap_or("https://server.rthmn.com".into());
    let auth_token = env::var("SUPABASE_SERVICE_ROLE_KEY").expect("SUPABASE_SERVICE_ROLE_KEY required");
    let supabase_url = env::var("SUPABASE_URL").expect("SUPABASE_URL required");
    let supabase_key = auth_token.clone();

    info!("Supabase URL: {}", supabase_url);
    info!("Main server URL: {}", main_server_url);

    let mut scanner = MarketScanner::default();
    scanner.initialize();
    info!("MarketScanner initialized with {} paths", scanner.path_count());

    let supabase = SupabaseClient::new(&supabase_url, &supabase_key);
    let tracker = SignalTracker::new(supabase);
    info!("SignalTracker initialized");

    let (signal_tx, signal_rx) = mpsc::channel::<SignalMessage>(1000);

    let state = Arc::new(AppState {
        scanner: RwLock::new(scanner),
        generator: SignalGenerator::default(),
        tracker,
        deduplicator: Deduplicator::new(),
        signals_sent: RwLock::new(0),
        main_server_url,
        signal_tx,
    });

    let state_clone = Arc::clone(&state);
    tokio::spawn(async move {
        main_server_forwarder(state_clone, auth_token, signal_rx).await;
    });

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

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    info!("WebSocket upgrade request received");
    ws.on_upgrade(|socket| {
        info!("WebSocket upgrade completed, starting handler");
        handle_socket(socket, state)
    })
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    info!("WebSocket client connected (boxes.rthmn.com)");

    let auth_msg = rmp_serde::to_vec(&serde_json::json!({"type": "authRequired"})).unwrap();
    let _ = sender.send(Message::Binary(auth_msg.into())).await;

    let mut authenticated = false;

    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Binary(data)) => {
                if let Ok(m) = rmp_serde::from_slice::<serde_json::Value>(&data) {
                    match m.get("type").and_then(|v| v.as_str()) {
                        Some("auth") => {
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
                        Some("heartbeat") => {}
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

async fn main_server_forwarder(state: Arc<AppState>, token: String, mut signal_rx: mpsc::Receiver<SignalMessage>) {
    let client = reqwest::Client::new();
    let url = format!("{}/signals/raw", state.main_server_url.trim_end_matches('/'));
    
    while let Some(signal) = signal_rx.recv().await {
        let result = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&signal)
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() => {
                *state.signals_sent.write().await += 1;
                info!("Forwarded raw signal to main server: {} {} L{}", signal.pair, signal.signal_type, signal.level);
            }
            Ok(resp) => warn!("Failed to forward raw signal to main server: {}", resp.status()),
            Err(e) => warn!("Failed to forward raw signal to main server: {}", e),
        }
    }
}

async fn process_box_update(state: &Arc<AppState>, pair: &str, data: &serde_json::Value) {
    let boxes: Vec<signals_rthmn::types::Box> = data
        .get("boxes")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let price = data.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);

    if boxes.is_empty() {
        return;
    }

    // Step 1: Check existing active signals for price hits (stop loss or targets)
    // Step 1: Check existing active signals for price hits (stop loss or targets)
    let settlements = state.tracker.check_price(pair, price).await;
    if !settlements.is_empty() {
        info!(
            "{} @ ${:.5} - {} signal(s) settled",
            pair,
            price,
            settlements.len()
        );
        
        for settlement in &settlements {
            if settlement.signal.level == 1 {
                state
                    .deduplicator
                    .remove_l1_signal(pair, &settlement.signal.signal_type.to_string())
                    .await;
            }
        }
    }

    // Step 2: Detect new patterns and generate signals
    let all_patterns = state.scanner.read().await.detect_patterns(pair, &boxes);
    if all_patterns.is_empty() {
        let (point, _) = signals_rthmn::instruments::get_instrument_config(pair);
        let integer_values: Vec<i32> = boxes.iter().map(|b| (b.value / point).round() as i32).collect();
        debug!("{}: No patterns detected. Box integer values: {:?}", pair, integer_values);
        return;
    }
    
    info!("{}: Detected {} pattern(s)", pair, all_patterns.len());

    let timestamp_ms = chrono::Utc::now().timestamp_millis();

    let mut filtered_patterns = Vec::new();
    for pattern in &all_patterns {
        if !state.deduplicator.should_filter_pattern(pair, pattern, &boxes, timestamp_ms).await {
            filtered_patterns.push(pattern.clone());
        }
    }

    if filtered_patterns.is_empty() {
        debug!("{}: All {} pattern(s) filtered by deduplicator", pair, all_patterns.len());
        return;
    }
    
    info!("{}: {} pattern(s) passed deduplication", pair, filtered_patterns.len());

    let unique_patterns = state.deduplicator.remove_subset_duplicates(filtered_patterns);
    info!("{} @ ${:.2} - {} pattern(s) after deduplication", pair, price, unique_patterns.len());

    for signal in state.generator.generate_signals(pair, &unique_patterns, &boxes, price) {
        if signal.entry.is_none() || signal.stop_losses.is_empty() || signal.targets.is_empty() {
            continue;
        }

        let signal_type_enum = match signal.signal_type.as_str() {
            "LONG" => SignalType::LONG,
            _ => SignalType::SHORT,
        };
        
        if state.deduplicator.should_filter_structural_boxes(pair, &signal.box_details, signal_type_enum, signal.level).await {
            info!("FILTERED: {} {} L{} - duplicate signal (structural boxes unchanged)", signal.pair, signal.signal_type, signal.level);
            continue;
        }

        let entry = signal.entry.unwrap_or(0.0);
        let stop_losses = signal.stop_losses.clone();
        let targets = signal.targets.clone();
        let first_stop = stop_losses.first().copied().unwrap_or(0.0);
        let final_target = targets.last().copied().unwrap_or(0.0);

        info!("SIGNAL: {} {} L{} {:?}", signal.pair, signal.signal_type, signal.level, signal.pattern_sequence);
        for (i, b) in signal.box_details.iter().enumerate() {
            info!("  Box {}: {} H:{:.5} L:{:.5}", i, b.integer_value, b.high, b.low);
        }
        let final_rr = signal.risk_reward.last().copied().unwrap_or(0.0);
        info!("  E:{:.5} S:{:?} (first: {:.5}) T:{:?} (final: {:.5}) R:R:{:?} (final: {:.2})", entry, stop_losses, first_stop, targets, final_target, signal.risk_reward, final_rr);

        let target_hits = vec![None; targets.len()];

        let active_signal = ActiveSignal {
            id: 0, // Will be set after Supabase insert
            pair: pair.to_string(),
            signal_type: signal_type_enum,
            level: signal.level,
            entry,
            stop_losses,
            targets,
            target_hits,
            stop_loss_hit: None,
            risk_reward: signal.risk_reward.clone(),
            pattern_sequence: signal.pattern_sequence.clone(),
            box_details: signal.box_details.clone(),
            created_at: chrono::Utc::now().timestamp_millis(),
        };

        let signal_id = state.tracker.add_signal(active_signal).await;
        let signal_with_id = signals_rthmn::types::SignalMessage {
            id: Some(signal_id),
            ..signal
        };
        let _ = state.signal_tx.send(signal_with_id).await;
    }
}
