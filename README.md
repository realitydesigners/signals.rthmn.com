# signals.rthmn.com - Signal Detection Service (Rust)

High-performance trading signal detection microservice.

## Architecture

```
boxes.rthmn.com (Rust)
       |
       | boxUpdate (WebSocket)
       v
+-------------------+          +--------------------+
| server.rthmn.com  |          | signals.rthmn.com  |
|                   |  signal  |                    |
| - Broadcasts to   |<---------| - Scans patterns   |
|   clients         |          | - Generates signals|
+-------------------+          +--------------------+
```

## Setup

```bash
# Create .env
cp .env.example .env

# Edit with your values:
# - BOXES_WS_URL=wss://boxes.rthmn.com/ws
# - SERVER_WS_URL=wss://server.rthmn.com/ws
# - SUPABASE_SERVICE_ROLE_KEY=your_key
```

## Run

```bash
cargo run --release
```

## API Endpoints

- `GET /health` - Health check
- `GET /api/status` - Scanner stats

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| PORT | HTTP server port | 3003 |
| BOXES_WS_URL | boxes.rthmn.com WebSocket URL | ws://localhost:3002/ws |
| SERVER_WS_URL | server.rthmn.com WebSocket URL | ws://localhost:3001/ws |
| SUPABASE_SERVICE_ROLE_KEY | Auth token | required |
