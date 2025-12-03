# signals.rthmn.com

Signal detection microservice in Rust.

## Architecture

```
boxes.rthmn.com --> signals.rthmn.com --> server.rthmn.com --> Users
     (boxes)           (scanning)           (broadcast)
```

## Files

```
src/
  main.rs        - Entry point, WebSocket server/client
  lib.rs         - Library exports for tests
  scanner.rs     - Pattern detection (MarketScanner)
  signal.rs      - Signal generation (SignalGenerator)
  patterns.rs    - Box traversal patterns
  instruments.rs - Trading pair configs (point, digits)
  types.rs       - Data structures

tests/
  patterns_test.rs - Tests for patterns.rs
```

---

## How Signal Detection Works

### Step 1: Receive Boxes

boxes.rthmn.com sends 38 boxes for each pair. Each box has:

- high: upper price level
- low: lower price level
- value: positive (bullish) or negative (bearish)

Example for BTCUSD:

```
Box 1: value = +20000 (bullish, largest)
Box 2: value = +17320 (bullish)
Box 3: value = -15000 (bearish)
Box 4: value = -12990 (bearish)
...
```

### Step 2: Convert to Integers

Divide each box value by the instrument's "point" value.

BTCUSD has point = 10, so:

```
+20000 / 10 = +2000
+17320 / 10 = +1732
-15000 / 10 = -1500
-12990 / 10 = -1299
```

This gives us integers like: [2000, 1732, -1500, -1299, 1125, ...]

### Step 3: Pattern Matching

The scanner has thousands of pre-calculated "paths" stored in patterns.rs.

A path is a sequence of integers that represents a valid price movement:

```
Example path: [2000, -1732, -1500, -1299, 200]
```

The scanner checks: "Do ALL these numbers exist in the current boxes?"

```
Current boxes: {2000, 1732, -1500, -1299, 1125, 974, 200, ...}

Check path [2000, -1732, -1500, 200]:
  2000  exists? YES
  -1732 exists? NO (we have +1732, not -1732)

Result: NO MATCH
```

```
Check path [2000, 1732, -1500, 200]:
  2000  exists? YES
  1732  exists? YES
  -1500 exists? YES
  200   exists? YES

Result: MATCH FOUND
```

### Step 4: Determine Signal Type

The first number in the path determines the signal type:

- Positive start (+2000) = LONG signal (price going up)
- Negative start (-2000) = SHORT signal (price going down)

### Step 5: Calculate Level

A **level** counts how many complete pattern **reversals** occur in the traversal.

Each reversal works like this:
1. Start at a key (e.g. 267)
2. Look up BOXES[267] to get valid patterns like `[-231, 130]`
3. Check if the live boxes contain that exact sequence
4. If yes → that's **one complete reversal**
5. The last value (130) becomes the new key
6. Repeat from step 2 with the new key

**Level Examples:**

```
L1 = 1 reversal
  Path: [267, -231, 130]
  - Start at 267
  - BOXES[267] contains [-231, 130]
  - Live data has [-231, 130] → 1 reversal complete
  - End at 130, no more patterns match → Level 1

L2 = 2 reversals  
  Path: [267, -231, 130, -112, 63]
  - Start at 267 → match [-231, 130] → reversal 1
  - Continue at 130 → match [-112, 63] → reversal 2
  - End → Level 2

L3 = 3 reversals
  - Three complete pattern traversals from start to end

L4 = 4 reversals
  - Four complete pattern traversals (rare, deep structure)
```

**Higher levels = deeper fractal structure = stronger but rarer signals**

### Step 6: Check Alert Rules

Not every pattern triggers an alert. Only certain levels do:

| Level | Sends Alert? |
| ----- | ------------ |
| 1     | Yes          |
| 2     | No           |
| 3     | Yes          |
| 4     | Yes          |

### Step 7: Generate Trade Opportunity

For patterns that trigger alerts, calculate entry/stop/target prices.

The rules use specific boxes based on level:

| Level | Entry | Stop Loss | Target |
| ----- | ----- | --------- | ------ |
| 1     | Box 2 | Box 2     | Box 1  |
| 3     | Box 4 | Box 4     | Box 1  |
| 4     | Box 5 | Box 5     | Box 1  |

For LONG signals:

- Entry = Box HIGH
- Stop Loss = Box LOW
- Target = Box HIGH

For SHORT signals:

- Entry = Box LOW
- Stop Loss = Box HIGH
- Target = Box LOW

Example LONG Level 1:

```
Box 1: high=$98,000, low=$78,000
Box 2: high=$97,000, low=$80,680

Entry:     $97,000 (Box 2 high)
Stop Loss: $80,680 (Box 2 low)
Target:    $98,000 (Box 1 high)
```

### Step 8: Send Signal

The signal is sent to server.rthmn.com which broadcasts to users.

---

## Setup

```bash
# Create .env file
SUPABASE_SERVICE_ROLE_KEY=your_key
SERVER_WS_URL=wss://server.rthmn.com/ws
PORT=3003
```

## Run

```bash
cargo run --release
```

## API

| Endpoint        | Description                         |
| --------------- | ----------------------------------- |
| GET /health     | Health check                        |
| GET /api/status | Scanner stats                       |
| WS /ws          | Receives boxes from boxes.rthmn.com |

## Testing

```bash
# Run all tests
cargo test

# Run patterns tests
cargo test --test patterns_test

# Generate paths output file
cargo test --test patterns_test test_generate_all_paths -- --nocapture
```

Output: `paths_output.txt` - All traversal paths (1,506,648 paths)

## Deploy (Railway)

1. Root directory: `signals.rthmn.com`
2. Builder: Dockerfile
3. Add env vars in dashboard
