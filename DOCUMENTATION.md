# signals.rthmn.com - Complete Documentation

## Table of Contents
1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Signal Detection Flow](#signal-detection-flow)
4. [Deduplication Strategies](#deduplication-strategies)
5. [State Management](#state-management)
6. [Trade Rules & Signal Generation](#trade-rules--signal-generation)
7. [Signal Tracking & Settlement](#signal-tracking--settlement)
8. [Edge Cases & Special Handling](#edge-cases--special-handling)
9. [API Endpoints](#api-endpoints)
10. [Configuration](#configuration)

---

## Overview

**signals.rthmn.com** is a high-performance Rust microservice that detects trading signals by matching real-time price box data against pre-computed pattern traversals. The service acts as the intelligence layer between box data generation and signal broadcasting.

### Key Responsibilities
- Receive real-time box updates via WebSocket
- Match box sequences against 1.5+ million pre-computed patterns
- Calculate signal levels (L1-L6) based on pattern reversals
- Generate trade opportunities with entry/stop/target prices
- Deduplicate signals using multiple strategies
- Track active signals and settle them when price hits stop loss or target
- Forward valid signals to the main server for user distribution

---

## Architecture

```
┌─────────────────┐         ┌──────────────────┐         ┌─────────────────┐
│ boxes.rthmn.com │ ──WS──> │ signals.rthmn.com│ ──HTTP─> │server.rthmn.com │
│  (Box Generator)│         │  (Pattern Match)  │         │   (Broadcast)   │
└─────────────────┘         └──────────────────┘         └─────────────────┘
                                      │
                                      ▼
                              ┌───────────────┐
                              │   Supabase    │
                              │ (Signal Store)│
                              └───────────────┘
```

### Component Flow

1. **boxes.rthmn.com** → Sends 38 boxes per trading pair via WebSocket
2. **signals.rthmn.com** → Processes boxes, detects patterns, generates signals
3. **server.rthmn.com** → Receives signals and broadcasts to users
4. **Supabase** → Stores active signals and settlement history

---

## Signal Detection Flow

### Step 1: Receive Box Data

**Source**: WebSocket connection from `boxes.rthmn.com`

**Data Structure**: Each update contains:
- `pair`: Trading pair identifier (e.g., "GBPCAD", "BTCUSD")
- `boxes`: Array of 38 boxes, each with:
  - `high`: Upper price boundary
  - `low`: Lower price boundary  
  - `value`: Positive (bullish) or negative (bearish) value
- `price`: Current market price
- `timestamp`: ISO 8601 timestamp

**Example Box Data**:
```json
{
  "pair": "BTCUSD",
  "boxes": [
    {"high": 98000, "low": 78000, "value": 20000},
    {"high": 97000, "low": 80680, "value": 17320},
    {"high": 85000, "low": 70000, "value": -15000},
    ...
  ],
  "price": 92000.50,
  "timestamp": "2025-12-19T01:06:23.123Z"
}
```

### Step 2: Convert to Integer Values

**Purpose**: Normalize box values to integers for pattern matching

**Process**:
1. Get instrument's `point` value (e.g., BTCUSD = 10, EURUSD = 0.0001)
2. Divide each box value by point: `integer_value = box.value / point`
3. Round to nearest integer

**Example**:
```
BTCUSD point = 10
Box 1: +20000 / 10 = +2000
Box 2: +17320 / 10 = +1732
Box 3: -15000 / 10 = -1500
```

**Result**: Integer array like `[2000, 1732, -1500, -1299, 1125, ...]`

### Step 3: Pattern Matching

**Pattern Database**: Pre-computed traversal paths stored in `patterns.rs`

**Total Patterns**: ~1,506,648 unique paths

**Matching Algorithm**:
1. Convert current boxes to integer set: `{2000, 1732, -1500, -1299, ...}`
2. For each pre-computed path, check if ALL values exist in current boxes
3. Match must be exact (sign matters: `-1732` ≠ `+1732`)

**Example Match**:
```
Path: [2000, 1732, -1500, 200]
Current boxes: {2000, 1732, -1500, -1299, 1125, 200, ...}

Check:
  2000  ∈ boxes? ✓ YES
  1732  ∈ boxes? ✓ YES
  -1500 ∈ boxes? ✓ YES
  200   ∈ boxes? ✓ YES

Result: MATCH FOUND
```

**Non-Match Example**:
```
Path: [2000, -1732, -1500, 200]
Current boxes: {2000, 1732, -1500, ...}

Check:
  2000  ∈ boxes? ✓ YES
  -1732 ∈ boxes? ✗ NO (we have +1732, not -1732)

Result: NO MATCH
```

### Step 4: Determine Signal Type

**Rule**: First value in path determines signal direction

- **Positive first value** (`+2000`) → **LONG** signal (bullish, price going up)
- **Negative first value** (`-2000`) → **SHORT** signal (bearish, price going down)

### Step 5: Calculate Level

**Definition**: Level = number of complete pattern reversals in the traversal

**Reversal Process**:
1. Start at a key value (e.g., `267`)
2. Look up `BOXES[267]` to get valid patterns like `[-231, 130]`
3. Check if live boxes contain that exact sequence
4. If yes → **one complete reversal**
5. Last value (`130`) becomes new key
6. Repeat from step 2

**Level Examples**:

**Level 1 (L1)**:
```
Path: [267, -231, 130]
- Start at 267
- BOXES[267] contains [-231, 130]
- Live data has [-231, 130] → 1 reversal complete
- End at 130, no more patterns match
Result: Level 1
```

**Level 2 (L2)**:
```
Path: [267, -231, 130, -112, 63]
- Start at 267 → match [-231, 130] → reversal 1
- Continue at 130 → match [-112, 63] → reversal 2
- End
Result: Level 2
```

**Level 3+**: Same pattern, more reversals

**Significance**: Higher levels = deeper fractal structure = stronger but rarer signals

### Step 6: Apply Deduplication Filters

Before generating a signal, multiple deduplication strategies are applied (see [Deduplication Strategies](#deduplication-strategies) section).

### Step 7: Generate Trade Opportunity

For patterns that pass all filters, calculate entry/stop/target prices using level-specific rules (see [Trade Rules](#trade-rules--signal-generation) section).

### Step 8: Track & Forward Signal

1. Store signal in Supabase via `SignalTracker`
2. Forward to `server.rthmn.com` via HTTP POST
3. Monitor price movements for settlement

---

## Deduplication Strategies

The service implements **four independent deduplication strategies** to prevent duplicate or invalid signals. Each strategy addresses a specific edge case.

### Strategy 1: Level 1 First-Only Deduplication

**Problem**: Multiple L1 signals can be generated for the same pattern sequence while box1 (largest box) remains unchanged. We only want the FIRST L1 signal, not subsequent ones.

**Example Scenario**:
```
L1 Signal 1: Pattern [100, -87, -75, -65, -56, -49, -42, -37, 21]
             Box1: high=1.85148, low=1.84333
             Created at: 01:06:23

L1 Signal 2: Pattern [100, -87, -75, -65, -56, -49, -42, 37]
             Box1: high=1.85148, low=1.84333 (SAME)
             Created at: 01:06:45

Result: Signal 2 should be FILTERED because box1 unchanged
```

**Implementation**:
- Track active L1 signals per pair and signal type (LONG/SHORT)
- Key format: `"{pair}:{signal_type}"` (e.g., `"GBPCAD:LONG"`)
- Store box1 high/low with each L1 signal
- If new L1 signal has same box1 high/low → filter it
- Clear L1 tracking when box1 changes (handled by global state management)

**Code Location**: `deduplication.rs::should_filter_l1()`

**Reasoning**: When box1 is unchanged, subsequent L1 signals represent the same market state. The first signal is sufficient; additional signals would be redundant entries.

---

### Strategy 2: Box Flip Detection (All Levels)

**Problem**: If a box value flips between positive/negative more than 3 times while box1 remains unchanged, it indicates excessive noise/chop. Signals generated from such patterns are unreliable.

**Example Scenario**:
```
LVL 2 Pattern: [276, -239, -207, ..., 37, -32, 16]

Box "16" flips:
  Update 1: "16"  (positive)
  Update 2: "-16" (negative) ← flip 1
  Update 3: "16"  (positive) ← flip 2
  Update 4: "-16" (negative) ← flip 3
  Update 5: "16"  (positive) ← flip 4 (EXCEEDS LIMIT)
  
Box1: high=1.85148, low=1.84333 (UNCHANGED throughout)

Result: Signal should be FILTERED (too many flips while box1 stable)
```

**Implementation**:
- Track last value and flip count for each box (by absolute value)
- For each box in pattern sequence:
  - Detect sign flip: `(last > 0 && current < 0) || (last < 0 && current > 0)`
  - If flipped AND box1 unchanged → increment flip count
  - If flip count > 3 → filter signal
  - If box1 changed → reset flip count to 0

**Code Location**: `deduplication.rs::should_filter_box_flips()`

**Reasoning**: Excessive flipping while box1 is stable indicates market indecision/noise. Signals from such patterns have low reliability and should be filtered to reduce false positives.

**Applies To**: ALL levels (L1, L2, L3, L4, L5, L6+)

---

### Strategy 3: Higher Level Preference

**Problem**: The same pattern sequence can match at multiple levels simultaneously. We should prefer the highest level since it represents deeper fractal structure.

**Example Scenario**:
```
Pattern Sequence: [6503, -5637, ..., 1004, -870, ..., 37, -32, 16]

Detected as:
  - Level 5: Full sequence [6503, ..., 16] → Target: 1.89148
  - Level 4: Sub-sequence [1004, ..., 16] → Target: 1.85148

Both detected at same timestamp with same box1 state.

Result: Keep Level 5, filter Level 4 (prefer higher level)
```

**Implementation**:
- Track pattern history per pair: `pattern_key → PatternHistory`
- Pattern key = sequence joined by underscores: `"6503_-5637_..._16"`
- Store all levels seen for each pattern key
- Store box1 high/low with pattern history
- If same pattern key seen again:
  - If box1 unchanged AND new level ≤ max existing level → filter
  - If box1 unchanged AND new level > max existing level → keep new, filter old
  - If box1 changed → treat as new pattern (reset history)

**Code Location**: `deduplication.rs::should_prefer_higher_level()`

**Reasoning**: Higher levels represent deeper market structure and stronger signals. When the same pattern appears at multiple levels, the highest level is most significant. Lower level duplicates are redundant.

---

### Strategy 4: Recent Signal Deduplication

**Problem**: The exact same signal (same pattern sequence, level, and prices) can be generated multiple times within a short time window due to box updates. We should only send it once.

**Example Scenario**:
```
Signal 1:
  Pattern: [-368, 319, 276, 239, 207, -116, 100, -65, 56, 49, 42, 37, -21]
  Level: 3
  Entry: 1.84361, Stop: 1.84382, Target: 1.8416
  Timestamp: 2025-12-19 01:36:32.994

Signal 2 (52 seconds later):
  Pattern: [-368, 319, 276, 239, 207, -116, 100, -65, 56, 49, 42, 37, -21]
  Level: 3
  Entry: 1.84361, Stop: 1.84382, Target: 1.8416 (SAME PRICES)
  Timestamp: 2025-12-19 01:37:24.505

Result: Signal 2 should be FILTERED (duplicate within time window)
```

**Implementation**:
- Track recently sent signals per pair
- Key: `pattern_key + level + prices`
- Time window: 5 minutes (300,000 ms)
- Price tolerance: 0.0001 (accounts for floating-point precision)
- Check before sending:
  - If same pattern + level + prices (within tolerance) sent within 5 minutes → filter
  - Otherwise → allow and add to recent signals
- Auto-cleanup: Remove signals older than 5 minutes

**Code Location**: `deduplication.rs::should_filter_recent_signal()`

**Reasoning**: Identical signals within a short time window are duplicates caused by box updates. Sending them multiple times would spam users. However, if prices change (even slightly), it's a legitimate new signal opportunity.

**Why 5 Minutes**: Long enough to prevent duplicates from rapid box updates, short enough to allow legitimate new signals when market conditions change.

---

## State Management

### Global State Indicator: Box1

**Principle**: Box1 (largest box) high/low serves as the global state indicator for ALL levels.

**Why Box1**: 
- Most stable box (changes least frequently)
- Represents the primary market direction
- When it changes, market state has fundamentally shifted

### State Change Detection

**Process**:
1. Track box1 high/low per pair: `pair → (high, low)`
2. On each pattern check:
   - Compare current box1 high/low with stored state
   - If changed (tolerance: 0.00001) → **state change detected**

**State Change Actions**:
When box1 changes, immediately clear:
- ✅ All box flip histories for that pair
- ✅ All pattern histories for that pair  
- ✅ All active L1 signals for that pair
- ✅ Update box1 state to new values

**Code Location**: `deduplication.rs::should_filter_pattern()` (lines 90-104)

### Why This Works

**Prevents Cache Growth**: 
- Old tracking data is automatically cleared when market state changes
- No need for time-based expiration of state-dependent data
- Cache only grows during stable market periods (bounded)

**Maintains Accuracy**:
- When box1 changes, previous tracking is invalid (different market state)
- Fresh start ensures deduplication logic applies to current state only
- Prevents false positives from stale data

**Example**:
```
State 1: Box1 = (1.85148, 1.84333)
  - Track patterns, flips, L1 signals
  - Market moves, boxes update
  - Box1 changes to (1.85200, 1.84400)

State 2: Box1 = (1.85200, 1.84400)  
  - ALL previous tracking cleared
  - Fresh tracking starts
  - New patterns, flips, signals tracked from clean slate
```

---

## Trade Rules & Signal Generation

### Trade Rule Structure

Each level has specific rules for calculating entry, stop loss, and target prices.

**Rule Components**:
- `level`: Signal level (1-6)
- `entry_box`: Which box to use for entry price
- `entry_point`: HIGH, LOW, or MID of entry box
- `stop_box`: Which box to use for stop loss
- `stop_point`: HIGH, LOW, or MID of stop box
- `target_box`: Which box to use for target price
- `target_point`: HIGH, LOW, or MID of target box

### Box Ordering

Boxes are sorted by absolute value descending:
- **Box 1**: Largest absolute value (primary direction)
- **Box 2**: Second largest
- **Box 3**: Third largest
- ...and so on

### Trade Rules by Level

| Level | Entry Box | Entry Point | Stop Box | Stop Point | Target Box | Target Point |
|-------|-----------|-------------|----------|-----------|------------|--------------|
| L1    | 2         | HIGH        | 2        | LOW       | 1          | HIGH         |
| L2    | 3         | HIGH        | 3        | LOW       | 1          | HIGH         |
| L3    | 4         | HIGH        | 4        | LOW       | 1          | HIGH         |
| L4    | 5         | HIGH        | 5        | LOW       | 1          | HIGH         |
| L5    | 6         | HIGH        | 6        | LOW       | 1          | HIGH         |
| L6    | 7         | HIGH        | 7        | LOW       | 1          | HIGH         |

**Note**: Rules are symmetric for LONG and SHORT, but entry/stop points are inverted for SHORT.

### LONG Signal Rules

**Entry**: Break above `entry_box` HIGH
**Stop Loss**: `entry_box` LOW
**Target**: Box 1 HIGH (full move potential)

**Example LONG L1**:
```
Box 1: high = $98,000, low = $78,000
Box 2: high = $97,000, low = $80,680

Entry:     $97,000 (Box 2 high - break above to enter)
Stop Loss: $80,680 (Box 2 low - if price falls here, exit)
Target:   $98,000 (Box 1 high - take profit here)
```

### SHORT Signal Rules

**Entry**: Break below `entry_box` LOW
**Stop Loss**: `entry_box` HIGH
**Target**: Box 1 LOW (full move potential)

**Example SHORT L1**:
```
Box 1: high = $98,000, low = $78,000
Box 2: high = $97,000, low = $80,680

Entry:     $80,680 (Box 2 low - break below to enter)
Stop Loss: $97,000 (Box 2 high - if price rises here, exit)
Target:   $78,000 (Box 1 low - take profit here)
```

### Risk/Reward Calculation

**Formula**: `risk_reward_ratio = |target - entry| / |entry - stop_loss|`

**Example**:
```
LONG L1:
  Entry: $97,000
  Stop:  $80,680
  Target: $98,000
  
  Risk:   $97,000 - $80,680 = $16,320
  Reward: $98,000 - $97,000 = $1,000
  R:R = $1,000 / $16,320 = 0.061 (6.1% reward per unit risk)
```

### Signal Generation Process

1. Filter patterns by level (only L1, L3, L4, L5, L6 generate signals - L2 is filtered)
2. For each pattern, find matching trade rule by level
3. Extract boxes from pattern (sorted by absolute value)
4. Calculate entry/stop/target using rule
5. Calculate risk/reward ratio
6. Validate: All prices must be present and valid
7. Create `SignalMessage` with trade opportunities

**Code Location**: `signal.rs::generate_signals()`

---

## Signal Tracking & Settlement

### Active Signal Tracking

**Purpose**: Monitor signals until they hit stop loss or target

**Storage**: 
- In-memory: `SignalTracker` maintains active signals per pair
- Persistent: Supabase `active_signals` table

**Signal Lifecycle**:
1. **Created**: Signal generated and added to tracker
2. **Active**: Price monitoring begins
3. **Settled**: Price hits stop loss or target
   - Status: `"success"` (hit target) or `"failed"` (hit stop loss)
   - Store settled price and timestamp
   - Remove from active tracking

### Settlement Logic

**Check Frequency**: Every box update (real-time)

**LONG Signal Settlement**:
- **Success**: `current_price >= target` → Status: `"success"`
- **Failed**: `current_price <= stop_loss` → Status: `"failed"`

**SHORT Signal Settlement**:
- **Success**: `current_price <= target` → Status: `"success"`
- **Failed**: `current_price >= stop_loss` → Status: `"failed"`

**Code Location**: `tracker.rs::check_price()`

### Settlement Cleanup

When a signal is settled:
1. Update Supabase with status and settled price
2. If L1 signal → remove from L1 deduplication tracking
3. Remove from in-memory active signals
4. Log settlement event

**Code Location**: `main.rs::process_box_update()` (lines 263-280)

---

## Edge Cases & Special Handling

### Case 1: Multiple Patterns Detected Simultaneously

**Scenario**: Single box update triggers multiple pattern matches

**Handling**:
1. Detect all matching patterns
2. Apply deduplication filters to each
3. Group by pattern sequence (prefer highest level)
4. Generate signals for unique patterns only

**Code Location**: `main.rs::process_box_update()` (lines 283-328)

### Case 2: Pattern Sequence Appears at Multiple Levels

**Scenario**: Same sequence matches L4 and L5 simultaneously

**Handling**: Strategy 3 (Higher Level Preference) filters lower level

**Example**:
```
Pattern: [6503, ..., 16]
- Detected as L5 → Keep
- Detected as L4 → Filter (L5 already exists)
```

### Case 3: Box1 Changes During Pattern Detection

**Scenario**: Box1 high/low changes between pattern checks

**Handling**: 
- State change detected → clear all tracking
- New patterns tracked with fresh state
- Previous patterns invalidated (different market state)

### Case 4: Rapid Box Updates

**Scenario**: Multiple box updates within milliseconds

**Handling**: 
- Recent signal deduplication (5-minute window)
- Only first signal sent if prices identical
- Subsequent signals filtered until prices change or window expires

### Case 5: Invalid Trade Opportunities

**Scenario**: Pattern matches but trade rule calculation fails (missing boxes, invalid prices)

**Handling**:
- Signal generated but `is_valid = false` for trade opportunity
- Signal not sent to users (filtered in main loop)
- Logged for debugging

**Code Location**: `main.rs::process_box_update()` (lines 334-343)

### Case 6: WebSocket Disconnection

**Scenario**: Connection to boxes.rthmn.com drops

**Handling**:
- WebSocket handler detects disconnect
- Logs disconnection event
- On reconnect, fresh authentication required
- State tracking persists (not cleared on disconnect)

### Case 7: Supabase Write Failures

**Scenario**: Failed to write signal to Supabase

**Handling**:
- Log warning but continue processing
- Signal still forwarded to main server
- In-memory tracking continues
- Retry not implemented (fail-fast approach)

**Code Location**: `tracker.rs::add_signal()` (lines 52-58)

---

## API Endpoints

### GET /health

**Purpose**: Health check endpoint

**Response**:
```json
{
  "status": "ok",
  "service": "signals.rthmn.com (rust)",
  "timestamp": "2025-12-19T01:06:23.123Z"
}
```

**Use Case**: Load balancer health checks, monitoring

---

### GET /api/status

**Purpose**: Service status and statistics

**Response**:
```json
{
  "scanner": {
    "totalPaths": 1506648,
    "isInitialized": true
  },
  "signalsSent": 1234,
  "activeSignals": {
    "total": 45,
    "byPair": {
      "GBPCAD": 12,
      "BTCUSD": 8,
      "EURUSD": 25
    }
  }
}
```

**Use Case**: Monitoring, debugging, operational dashboards

---

### WebSocket /ws

**Purpose**: Receive box updates from boxes.rthmn.com

**Authentication**: 
- Server sends `{"type": "authRequired"}` on connect
- Client sends `{"type": "auth", "token": "..."}`
- Server responds `{"type": "welcome"}` on success

**Message Format**: MessagePack binary encoding

**Message Types**:
- `boxUpdate`: Contains pair, boxes array, price, timestamp
- `heartbeat`: Keep-alive (acknowledged but not processed)

**Example boxUpdate**:
```json
{
  "type": "boxUpdate",
  "pair": "GBPCAD",
  "data": {
    "boxes": [
      {"high": 1.85148, "low": 1.84333, "value": 815},
      ...
    ],
    "price": 1.84349,
    "timestamp": "2025-12-19T01:06:23.123Z"
  }
}
```

---

## Configuration

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `PORT` | No | `3003` | HTTP server port |
| `SUPABASE_URL` | Yes | - | Supabase project URL |
| `SUPABASE_SERVICE_ROLE_KEY` | Yes | - | Supabase service role key |
| `MAIN_SERVER_URL` | No | `https://server.rthmn.com` | Main server URL for signal forwarding |

### Example .env

```bash
PORT=3003
SUPABASE_URL=https://xxxxx.supabase.co
SUPABASE_SERVICE_ROLE_KEY=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...
MAIN_SERVER_URL=https://server.rthmn.com
```

---

## Performance Characteristics

### Pattern Matching
- **Pattern Database**: ~1.5 million paths loaded at startup
- **Matching Algorithm**: O(n*m) where n = patterns, m = box count
- **Optimization**: Early exit on first mismatch, HashSet for O(1) lookups

### Memory Usage
- **Pattern Storage**: ~50-100MB (depends on path lengths)
- **Deduplication State**: Bounded by:
  - Active pairs (typically 10-50)
  - Recent signals (5-minute window, ~100-1000 per pair)
  - Pattern histories (cleared on box1 change)

### Throughput
- **Box Updates**: Processed in <1ms per update
- **Signal Generation**: <10ms per signal
- **Supabase Writes**: Async, non-blocking

---

## Error Handling

### WebSocket Errors
- **Connection Drops**: Logged, handler exits gracefully
- **Invalid Messages**: Ignored, connection continues
- **Auth Failures**: Connection closed

### Pattern Detection Errors
- **Empty Boxes**: Pattern detection skipped
- **Invalid Box Data**: Pattern detection skipped
- **Missing Instrument Config**: Default point value used (may cause incorrect matching)

### Signal Generation Errors
- **Missing Trade Rules**: Signal filtered (not sent)
- **Invalid Prices**: Trade opportunity marked `is_valid = false`
- **Calculation Errors**: Signal filtered, error logged

---

## Testing

### Unit Tests
```bash
cargo test
```

### Pattern Tests
```bash
cargo test --test patterns_test
```

### Generate Pattern Output
```bash
cargo test --test patterns_test test_generate_all_paths -- --nocapture
```
Output: `paths_output.txt` with all 1,506,648 paths

---

## Deployment

### Railway Deployment
1. Root directory: `signals.rthmn.com`
2. Builder: Dockerfile
3. Add environment variables in Railway dashboard
4. Deploy automatically on git push

### Docker Build
```bash
docker build -t signals-rthmn .
docker run -p 3003:3003 --env-file .env signals-rthmn
```

---

## Monitoring & Observability

### Logging
- **Level**: INFO by default
- **Format**: Structured logging with tracing
- **Key Events**:
  - Pattern matches
  - Signal generation
  - Signal settlements
  - Deduplication filters applied
  - WebSocket connections

### Metrics (via /api/status)
- Total signals sent
- Active signals count
- Active signals by pair
- Pattern database size

### Health Checks
- `/health` endpoint for uptime monitoring
- WebSocket connection status
- Supabase connectivity (implicit via writes)

---

## Future Enhancements

### Potential Improvements
1. **Pattern Caching**: Cache frequently matched patterns
2. **Batch Processing**: Batch Supabase writes for better throughput
3. **Signal Prioritization**: Prioritize higher level signals
4. **Historical Analysis**: Track signal success rates by level/pattern
5. **Adaptive Filtering**: Adjust deduplication based on market volatility

---

## Summary

**signals.rthmn.com** is a sophisticated pattern matching engine that:

1. ✅ Processes real-time box data via WebSocket
2. ✅ Matches against 1.5M+ pre-computed patterns
3. ✅ Calculates signal levels based on pattern reversals
4. ✅ Applies 4 independent deduplication strategies
5. ✅ Generates trade opportunities with entry/stop/target
6. ✅ Tracks signals until settlement
7. ✅ Forwards valid signals to main server
8. ✅ Maintains bounded memory via state-based cleanup

The service is designed for **high performance**, **low latency**, and **reliable signal generation** while preventing duplicate or invalid signals through comprehensive deduplication.

