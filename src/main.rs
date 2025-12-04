use signals_rthmn::{
    grpc::{create_box_service, SignalClient},
    scanner::MarketScanner,
    signal::SignalGenerator,
    types::SignalMessage,
};
use axum::{routing::get, Json, Router};
use chrono::Utc;
use std::{env, sync::Arc};
use tokio::sync::{mpsc, RwLock};
use tonic::transport::Server;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("signals_rthmn=info").init();
    dotenvy::dotenv().ok();

    info!("==================================================");
    info!("  SIGNALS.RTHMN.COM - gRPC Edition");
    info!("==================================================");

    let http_port: u16 = env::var("PORT").unwrap_or("3003".into()).parse().unwrap_or(3003);
    let grpc_port: u16 = env::var("GRPC_PORT").unwrap_or("50051".into()).parse().unwrap_or(50051);
    let server_grpc = env::var("SERVER_GRPC_URL").unwrap_or("http://localhost:50052".into());

    let mut scanner = MarketScanner::default();
    scanner.initialize();
    info!("MarketScanner initialized with {} paths", scanner.path_count());

    let scanner = Arc::new(RwLock::new(scanner));
    let generator = Arc::new(SignalGenerator::default());
    let (signal_tx, signal_rx) = mpsc::channel::<SignalMessage>(1000);

    let signals_sent = Arc::new(RwLock::new(0u64));
    let signals_sent_clone = Arc::clone(&signals_sent);

    // gRPC client to server.rthmn.com
    tokio::spawn(async move {
        let client = SignalClient::new(server_grpc);
        client.stream_signals(signal_rx).await;
    });

    // gRPC server for boxes.rthmn.com
    let box_service = create_box_service(
        Arc::clone(&scanner),
        Arc::clone(&generator),
        signal_tx,
    );

    let grpc_addr = format!("0.0.0.0:{}", grpc_port).parse()?;
    info!("gRPC server listening on {}", grpc_addr);
    
    tokio::spawn(async move {
        Server::builder()
            .add_service(box_service)
            .serve(grpc_addr)
            .await
            .expect("gRPC server failed");
    });

    // HTTP server for health checks
    let scanner_clone = Arc::clone(&scanner);
    let app = Router::new()
        .route("/health", get(|| async {
    Json(serde_json::json!({
        "status": "ok",
                "service": "signals.rthmn.com (gRPC)",
        "timestamp": Utc::now().to_rfc3339()
    }))
        }))
        .route("/api/status", get(move || {
            let scanner = Arc::clone(&scanner_clone);
            let signals = Arc::clone(&signals_sent_clone);
            async move {
                let s = scanner.read().await;
                let count = *signals.read().await;
    Json(serde_json::json!({
        "scanner": {
                        "totalPaths": s.path_count(),
            "isInitialized": true
        },
                    "signalsSent": count,
                    "protocol": "gRPC"
                }))
            }
        }))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", http_port)).await?;
    info!("HTTP server running on port {}", http_port);
    axum::serve(listener, app).await?;
    Ok(())
}
