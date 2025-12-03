use signals_rthmn::{scanner::MarketScanner, signal::SignalGenerator, types::{BoxData, SignalMessage}};
use axum::{extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State}, response::IntoResponse, routing::get, Json, Router};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use std::{collections::HashMap, env, sync::Arc};
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message as TungMessage};
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

pub struct AppState {
    scanner: RwLock<MarketScanner>,
    generator: SignalGenerator,
    box_data: RwLock<HashMap<String, BoxData>>,
    signals_sent: RwLock<u64>,
    server_connected: RwLock<bool>,
    signal_tx: mpsc::Sender<SignalMessage>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("signals_rthmn=info").init();
    dotenvy::dotenv().ok();

    info!("==================================================");
    info!("  SIGNALS.RTHMN.COM - Rust Edition");
    info!("==================================================");

    let port: u16 = env::var("PORT").unwrap_or("3003".into()).parse().unwrap_or(3003);
    let server_ws = env::var("SERVER_WS_URL").unwrap_or("ws://localhost:3001/ws".into());
    let auth_token = env::var("SUPABASE_SERVICE_ROLE_KEY").expect("SUPABASE_SERVICE_ROLE_KEY required");

    let mut scanner = MarketScanner::default();
    scanner.initialize();
    info!("MarketScanner initialized with {} paths", scanner.path_count());

    let (signal_tx, signal_rx) = mpsc::channel::<SignalMessage>(1000);

    let state = Arc::new(AppState {
        scanner: RwLock::new(scanner),
        generator: SignalGenerator::default(),
        box_data: RwLock::new(HashMap::new()),
        signals_sent: RwLock::new(0),
        server_connected: RwLock::new(false),
        signal_tx,
    });

    // Connect to server.rthmn.com to send signals
    let s = Arc::clone(&state);
    tokio::spawn(async move {
        server_client(s, server_ws, auth_token, signal_rx).await;
    });

    // HTTP + WebSocket server
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/status", get(status))
        .route("/ws", get(ws_handler))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
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
    let server_conn = *s.server_connected.read().await;
    Json(serde_json::json!({
        "scanner": {
            "totalPaths": scanner.path_count(),
            "isInitialized": true
        },
        "signalsSent": signals,
        "serverConnected": server_conn
    }))
}

/// WebSocket handler - receives box updates from boxes.rthmn.com
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    info!("WebSocket client connected (boxes.rthmn.com)");

    // Send auth required
    let auth_msg = rmp_serde::to_vec(&serde_json::json!({"type": "authRequired"})).unwrap();
    let _ = sender.send(Message::Binary(auth_msg)).await;

    let mut authenticated = false;

    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Binary(data)) => {
                if let Ok(m) = rmp_serde::from_slice::<serde_json::Value>(&data) {
                    match m.get("type").and_then(|v| v.as_str()) {
                        Some("auth") => {
                            // Accept any auth for now (boxes.rthmn.com uses service key)
                            authenticated = true;
                            let welcome = rmp_serde::to_vec(&serde_json::json!({"type": "welcome"})).unwrap();
                            let _ = sender.send(Message::Binary(welcome)).await;
                            info!("boxes.rthmn.com authenticated");
                        }
                        Some("boxUpdate") if authenticated => {
                            if let (Some(pair), Some(data)) = (
                                m.get("pair").and_then(|v| v.as_str()),
                                m.get("data")
                            ) {
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

/// Client that sends signals to server.rthmn.com
async fn server_client(state: Arc<AppState>, url: String, token: String, mut signal_rx: mpsc::Receiver<SignalMessage>) {
    loop {
        info!("Connecting to server.rthmn.com at {}...", url);
        match connect_async(&url).await {
            Ok((ws, _)) => {
                info!("Connected to server.rthmn.com");
                let (mut write, mut read) = ws.split();
                let mut authed = false;

                loop {
                    tokio::select! {
                        Some(msg) = read.next() => {
                            match msg {
                                Ok(TungMessage::Binary(data)) => {
                                    if let Ok(m) = rmp_serde::from_slice::<serde_json::Value>(&data) {
                                        match m.get("type").and_then(|v| v.as_str()) {
                                            Some("authRequired") => {
                                                let auth = rmp_serde::to_vec(&serde_json::json!({
                                                    "type": "auth",
                                                    "token": token
                                                })).unwrap();
                                                let _ = write.send(TungMessage::Binary(auth)).await;
                                            }
                                            Some("welcome") => {
                                                authed = true;
                                                *state.server_connected.write().await = true;
                                                info!("Authenticated with server.rthmn.com");
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                Ok(TungMessage::Close(_)) => break,
                                Err(e) => { warn!("server.rthmn.com error: {}", e); break; }
                                _ => {}
                            }
                        }
                        Some(signal) = signal_rx.recv(), if authed => {
                            let msg = rmp_serde::to_vec(&serde_json::json!({
                                "type": "signal",
                                "signalId": signal.signal_id,
                                "pair": signal.pair,
                                "signalType": signal.signal_type,
                                "level": signal.level,
                                "customPattern": signal.custom_pattern,
                                "patternSequence": signal.pattern_sequence,
                                "timestamp": signal.timestamp,
                                "data": signal.data
                            })).unwrap();
                            let _ = write.send(TungMessage::Binary(msg)).await;
                            *state.signals_sent.write().await += 1;
                            info!("Signal sent: {} {} L{}", signal.pair, signal.signal_type, signal.level);
                        }
                    }
                }
                *state.server_connected.write().await = false;
            }
            Err(e) => warn!("Failed to connect to server.rthmn.com: {}", e),
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

async fn process_box_update(state: &Arc<AppState>, pair: &str, data: &serde_json::Value) {
    let boxes: Vec<signals_rthmn::types::Box> = data.get("boxes").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let price = data.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let timestamp = data.get("timestamp").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if boxes.is_empty() { return; }

    state.box_data.write().await.insert(pair.to_string(), BoxData { pair: pair.to_string(), boxes: boxes.clone(), price, timestamp });

    let patterns = state.scanner.read().await.detect_patterns(pair, &boxes);
    if patterns.is_empty() { return; }

    info!("{} @ ${:.2} - {} pattern(s)", pair, price, patterns.len());
    
    for signal in state.generator.generate_signals(pair, &patterns, &boxes, price) {
        info!("SIGNAL: {} {} L{} {:?}", signal.pair, signal.signal_type, signal.level, signal.pattern_sequence);
        for t in signal.data.trade_opportunities.iter().filter(|t| t.is_valid) {
            info!("  {} E:{:.2} S:{:.2} T:{:.2} R:R:{:.2}", t.rule_id, t.entry.unwrap_or(0.0), t.stop_loss.unwrap_or(0.0), t.target.unwrap_or(0.0), t.risk_reward_ratio.unwrap_or(0.0));
        }
        let _ = state.signal_tx.send(signal).await;
    }
}
