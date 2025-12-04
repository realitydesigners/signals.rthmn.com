use crate::proto::{
    box_service_server::{BoxService, BoxServiceServer},
    signal_service_client::SignalServiceClient,
    Ack, BoxUpdate, Signal, SignalData, BoxDetail, TradeOpportunity,
};
use crate::scanner::MarketScanner;
use crate::signal::SignalGenerator;
use crate::types::{ResoBox, SignalMessage};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};
use tracing::{info, warn};

pub struct BoxServiceImpl {
    pub scanner: Arc<RwLock<MarketScanner>>,
    pub generator: Arc<SignalGenerator>,
    pub signal_tx: mpsc::Sender<SignalMessage>,
}

#[tonic::async_trait]
impl BoxService for BoxServiceImpl {
    async fn stream_boxes(
        &self,
        request: Request<Streaming<BoxUpdate>>,
    ) -> Result<Response<Ack>, Status> {
        let mut stream = request.into_inner();
        info!("[gRPC] BoxService stream started");

        while let Some(result) = stream.next().await {
            match result {
                Ok(update) => {
                    self.process_box_update(update).await;
                }
                Err(e) => {
                    warn!("[gRPC] Stream error: {}", e);
                    break;
                }
            }
        }

        info!("[gRPC] BoxService stream ended");
        Ok(Response::new(Ack {
            success: true,
            message: Some("Stream completed".into()),
        }))
    }
}

impl BoxServiceImpl {
    async fn process_box_update(&self, update: BoxUpdate) {
        let boxes: Vec<ResoBox> = update.boxes.iter().map(|b| ResoBox {
            high: b.high,
            low: b.low,
            value: b.value,
        }).collect();

        if boxes.is_empty() { return; }

        let pair = &update.pair;
        let price = update.price;

        let patterns = self.scanner.read().await.detect_patterns(pair, &boxes);
        if patterns.is_empty() { return; }

        info!("{} @ ${:.2} - {} pattern(s)", pair, price, patterns.len());

        for signal in self.generator.generate_signals(pair, &patterns, &boxes, price) {
            info!("SIGNAL: {} {} L{} {:?}", signal.pair, signal.signal_type, signal.level, signal.pattern_sequence);
            for b in &signal.data.box_details {
                info!("  Box: {} H:{:.5} L:{:.5}", b.integer_value, b.high, b.low);
            }
            for t in signal.data.trade_opportunities.iter().filter(|t| t.is_valid) {
                info!("  {} E:{:.5} S:{:.5} T:{:.5} R:R:{:.2}", 
                    t.rule_id, 
                    t.entry.unwrap_or(0.0), 
                    t.stop_loss.unwrap_or(0.0), 
                    t.target.unwrap_or(0.0), 
                    t.risk_reward_ratio.unwrap_or(0.0)
                );
            }
            let _ = self.signal_tx.send(signal).await;
        }
    }
}

pub fn create_box_service(
    scanner: Arc<RwLock<MarketScanner>>,
    generator: Arc<SignalGenerator>,
    signal_tx: mpsc::Sender<SignalMessage>,
) -> BoxServiceServer<BoxServiceImpl> {
    BoxServiceServer::new(BoxServiceImpl {
        scanner,
        generator,
        signal_tx,
    })
}

// Signal client - streams signals to server.rthmn.com
pub struct SignalClient {
    url: String,
}

impl SignalClient {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    pub async fn stream_signals(self, rx: mpsc::Receiver<SignalMessage>) {
        // Wrap receiver in Arc<Mutex> for reconnection reuse
        let rx = std::sync::Arc::new(tokio::sync::Mutex::new(rx));
        
        loop {
            info!("[gRPC] Connecting to server.rthmn.com at {}...", self.url);
            match SignalServiceClient::connect(self.url.clone()).await {
                Ok(mut client) => {
                    info!("[gRPC] Connected to server.rthmn.com");
                    
                    let (tx, grpc_rx) = mpsc::channel::<Signal>(1000);
                    let stream = tokio_stream::wrappers::ReceiverStream::new(grpc_rx);
                    
                    let rx_clone = rx.clone();
                    let forward_task = tokio::spawn(async move {
                        let mut rx_guard = rx_clone.lock().await;
                        while let Some(signal) = rx_guard.recv().await {
                            let proto_signal = signal_to_proto(&signal);
                            if tx.send(proto_signal).await.is_err() {
                                break;
                            }
                        }
                    });

                    match client.stream_signals(stream).await {
                        Ok(response) => {
                            info!("[gRPC] Stream ended: {:?}", response.into_inner().message);
                        }
                        Err(e) => {
                            warn!("[gRPC] Stream error: {}", e);
                        }
                    }
                    
                    forward_task.abort();
                    // Reconnect after error instead of breaking
                    warn!("[gRPC] Disconnected from server.rthmn.com, reconnecting in 5s...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
                Err(e) => {
                    warn!("[gRPC] Connection failed: {}. Retrying in 5s...", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    }
}

fn signal_to_proto(signal: &SignalMessage) -> Signal {
    Signal {
        signal_id: signal.signal_id.clone(),
        pair: signal.pair.clone(),
        signal_type: signal.signal_type.clone(),
        level: signal.level,
        custom_pattern: signal.custom_pattern.clone(),
        pattern_sequence: signal.pattern_sequence.clone(),
        timestamp: signal.timestamp,
        data: Some(SignalData {
            box_details: signal.data.box_details.iter().map(|b| BoxDetail {
                integer_value: b.integer_value,
                high: b.high,
                low: b.low,
                value: b.value,
            }).collect(),
            trade_opportunities: signal.data.trade_opportunities.iter().map(|t| TradeOpportunity {
                rule_id: t.rule_id.clone(),
                level: t.level,
                entry: t.entry,
                stop_loss: t.stop_loss,
                target: t.target,
                risk_reward_ratio: t.risk_reward_ratio,
                is_valid: t.is_valid,
            }).collect(),
            complete_box_snapshot: signal.data.complete_box_snapshot.clone(),
            has_trade_rules: signal.data.has_trade_rules,
        }),
    }
}

