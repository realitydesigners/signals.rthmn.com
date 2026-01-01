# signals.rthmn.com - Complete Technical Documentation

High-performance Rust microservice that detects trading signals by matching real-time box data against pre-computed pattern traversals. This service acts as the intelligence layer between box data generation and signal broadcasting.

## Table of Contents

1. [Architecture](#architecture)
2. [Signal Detection Flow](#signal-detection-flow)
3. [Pattern Database Generation](#pattern-database-generation)
4. [Pattern Matching Algorithm](#pattern-matching-algorithm)
5. [Level Calculation](#level-calculation)
6. [Deduplication Strategies](#deduplication-strategies)
7. [Trade Rules & Signal Generation](#trade-rules--signal-generation)
8. [Signal Tracking & Settlement](#signal-tracking--settlement)
9. [API Endpoints](#api-endpoints)
10. [Configuration](#configuration)
11. [Data Structures](#data-structures)
12. [Performance Characteristics](#performance-characteristics)
13. [Error Handling](#error-handling)
14. [Edge Cases & Special Handling](#edge-cases--special-handling)
15. [Testing](#testing)
16. [Monitoring & Observability](#monitoring--observability)
17. [Deployment](#deployment)
18. [Complete End-to-End Example](#complete-end-to-end-example)

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

**Component Flow**:
1. **boxes.rthmn.com** sends 38 boxes per trading pair via WebSocket (MessagePack binary encoding)
2. **signals.rthmn.com** processes boxes, detects patterns, generates signals
3. **server.rthmn.com** receives signals via HTTP POST `/signals/raw` and broadcasts to users
4. **Supabase** stores active signals and settlement history for persistence

**Internal Components**:
- `MarketScanner`: Pattern detection engine (1.5M+ paths)
- `SignalGenerator`: Trade rule application and signal creation
- `Deduplicator`: Four-layer deduplication system
- `SignalTracker`: Active signal monitoring and settlement
- `SupabaseClient`: Database persistence layer

## Signal Detection Flow

### 1. Receive Box Data

**Source**: WebSocket connection from `boxes.rthmn.com`

**Data Structure**: Each update contains:
- `pair`: Trading pair identifier (e.g., "GBPCAD", "BTCUSD")
- `boxes`: Array of 38 boxes, each with:
  - `high`: Upper price boundary
  - `low`: Lower price boundary  
  - `value`: Positive (bullish) or negative (bearish) value
- `price`: Current market price
- `timestamp`: ISO 8601 timestamp

**Example**:
```json
{
  "pair": "BTCUSD",
  "boxes": [
    {"high": 98000, "low": 78000, "value": 20000},
    {"high": 97000, "low": 80680, "value": 17320},
    {"high": 85000, "low": 70000, "value": -15000}
  ],
  "price": 92000.50,
  "timestamp": "2025-12-19T01:06:23.123Z"
}
```

### 2. Convert to Integer Values

**Purpose**: Normalize box values to integers for pattern matching

**Process**:
1. Get instrument's `point` value (e.g., BTCUSD = 10, EURUSD = 0.00001)
2. Divide each box value by point: `integer_value = round(box.value / point)`
3. Round to nearest integer

**Example**:
```
BTCUSD point = 10
Box 0: +20000 / 10 = +2000
Box 1: +17320 / 10 = +1732
Box 2: -15000 / 10 = -1500
```

**Result**: Integer array like `[2000, 1732, -1500, -1299, 1125, ...]`

### 3. Pattern Matching

**Pattern Database**: Pre-computed traversal paths stored in `patterns.rs`
- Generated at startup from `BOXES` map and `STARTING_POINTS` (see [Pattern Database Generation](#pattern-database-generation))
- Total patterns: ~1,506,648 unique paths
- All paths generated as LONG (positive), SHORT patterns are inverted during detection
- Stored in memory as `Vec<TraversalPath>` for O(1) access
- Generated once at startup, reused for all pattern matching

**Matching Algorithm** (`scanner.rs::detect_patterns()`):
1. Convert current boxes to integer set using HashSet: `{2000, 1732, -1500, -1299, ...}`
   - HashSet provides O(1) lookup for membership testing
2. Iterate through all pre-computed paths (~1.5M paths)
3. For each path, check if ALL values exist in current boxes:
   - First check if first value exists (early exit optimization)
   - Then check if all remaining values exist
   - Match must be exact (sign matters: `-1732` ≠ `+1732`)
4. Check both LONG (original) and SHORT (inverted) patterns:
   - LONG: Use path as-is
   - SHORT: Invert all values (multiply by -1)
5. For each match, create `PatternMatch` with box details and calculate level

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

### 4. Determine Signal Type

**Rule**: First value in path determines signal direction
- **Positive first value** (`+2000`) → **LONG** signal (bullish, price going up)
- **Negative first value** (`-2000`) → **SHORT** signal (bearish, price going down)

### 5. Calculate Level

**Definition**: Level = number of complete pattern reversals in the traversal

**Algorithm** (`scanner.rs::calculate_level()`):
1. Start at index 0 with key = `path[0]`
2. While not at end of path:
   - Look up `BOXES[key.abs()]` to get valid patterns
   - For each pattern in `BOXES[key.abs()]`:
     - Adjust pattern sign based on current key (positive key = original, negative key = inverted)
     - Check if path contains this exact sequence starting from `idx + 1`
     - If match found:
       - Increment level counter
       - Move index to end of matched pattern
       - Set new key to last value of matched pattern
       - Continue from step 2
     - If no match found → break (end of reversals)
3. Return `level.max(1)` (minimum level is 1)

**Reversal Process Details**:
1. Start at a key value (e.g., `267`)
2. Look up `BOXES[267]` to get valid patterns like `[-231, 130]`
3. Adjust pattern based on key sign:
   - If key > 0: use pattern as-is
   - If key < 0: invert all pattern values
4. Check if path contains that exact sequence starting from current position
5. If yes → **one complete reversal** (level += 1)
6. Last value (`130`) becomes new key
7. Repeat from step 2 until no more matches

**Level Examples**:

**Level 1 (L1)**:
```
Path: [267, -231, 130]
- Start at 267
- BOXES[267] contains [-231, 130]
- Path has [-231, 130] → 1 reversal complete
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

### 6. Apply Deduplication Filters

Before generating a signal, multiple deduplication strategies are applied in sequence (see [Deduplication Strategies](#deduplication-strategies) section).

**Processing Order**:
1. Detect all matching patterns
2. Apply `should_filter_pattern()` for each pattern (L1 first-only, box 0 state management)
3. Apply `remove_subset_duplicates()` to prefer higher levels
4. Generate signals
5. Apply `should_filter_structural_boxes()` before sending (final structural deduplication)

### 7. Generate Signal

For patterns that pass all filters, calculate entry, stop losses, and targets using level-specific rules (see [Trade Rules](#trade-rules) section).

**Signal Generation Process**:
1. Filter patterns by level (all levels L1-L6 generate signals)
2. For each pattern, find matching trade rule by level
3. Extract primary boxes from pattern:
   - LONG: All positive boxes in pattern
   - SHORT: All negative boxes in pattern
4. Sort primary boxes by absolute value descending (box 0 = largest)
5. Calculate entry, stop losses, and targets using rule
6. Calculate risk/reward ratio using final target and first stop loss
7. Validate: Entry, stop losses, and targets must all be present and valid
8. Create `SignalMessage` with all fields

### 8. Track & Forward Signal

1. Store signal in Supabase via `SignalTracker.add_signal()`
2. Forward to `server.rthmn.com` via HTTP POST `/signals/raw` with Bearer token
3. Monitor price movements for settlement on every box update

## Trade Rules

### Box Ordering (0-Indexed)

Boxes are sorted by absolute value descending and indexed starting from 0:
- **Box 0**: Largest absolute value (primary direction)
- **Box 1**: Second largest
- **Box 2**: Third largest
- **Box 3**: Fourth largest
- ...and so on

### Trade Rules by Level

**All Levels Supported**: L1, L2, L3, L4, L5, L6

| Level | Entry Box | Entry Point | Stop Boxes | Stop Point | Target Boxes    | Target Point |
|-------|-----------|-------------|------------|------------|-----------------|--------------|
| L1    | 1         | HIGH        | [0]        | LOW        | [0]             | HIGH         |
| L2    | 2         | HIGH        | [1]        | LOW        | [0, 1]          | HIGH         |
| L3    | 3         | HIGH        | [2]        | LOW        | [0, 1, 2]       | HIGH         |
| L4    | 4         | HIGH        | [3]        | LOW        | [0, 1, 2, ...,] | HIGH         |
| L5    | 5         | HIGH        | [4]        | LOW        | [0, 1, 2, ...,] | HIGH         |
| L6    | 6         | HIGH        | [5]        | LOW        | [0, 1, 2, ...,] | HIGH         |

**Note**: 
- Rules are symmetric for LONG and SHORT, but entry/stop points are inverted for SHORT
- `stop_boxes` is an array (currently contains one stop loss, structured for future multiple stops)
- `target_boxes` creates cumulative targets: each target adds the size of its box to the previous target

### LONG Signal Rules

**Entry**: Break above `entry_box` HIGH
**Stop Losses**: Array of stop loss prices (currently uses first stop loss from `stop_boxes[0]`)
**Targets**: Array of cumulative target prices, each adding the size of its target box

**Example LONG L3**:
```
Box 0: high = $98,000, low = $78,000 (size = $20,000)
Box 1: high = $97,000, low = $80,680 (size = $16,320)
Box 2: high = $96,000, low = $82,000 (size = $14,000)
Box 3: high = $95,000, low = $83,000 (entry box, size = $12,000)

Entry:      $95,000 (Box 3 high - break above to enter)
Stop Losses: [$82,000] (Box 2 low - first stop loss from array)
Targets:    [$98,000, $114,320] (cumulative)
  - Target 1: $98,000 (Box 0 high)
  - Target 2: $98,000 + $16,320 = $114,320 (Box 0 high + Box 1 size)
```

### SHORT Signal Rules

**Entry**: Break below `entry_box` LOW
**Stop Losses**: Array of stop loss prices (currently uses first stop loss from `stop_boxes[0]`)
**Targets**: Array of cumulative target prices, each subtracting the size of its target box

**Example SHORT L3**:
```
Box 0: high = $98,000, low = $78,000 (size = $20,000)
Box 1: high = $97,000, low = $80,680 (size = $16,320)
Box 2: high = $96,000, low = $82,000 (size = $14,000)
Box 3: high = $95,000, low = $83,000 (entry box, size = $12,000)

Entry:      $83,000 (Box 3 low - break below to enter)
Stop Losses: [$96,000] (Box 2 high - first stop loss from array)
Targets:    [$78,000, $61,680] (cumulative)
  - Target 1: $78,000 (Box 0 low)
  - Target 2: $78,000 - $16,320 = $61,680 (Box 0 low - Box 1 size)
```

### Target Calculation Details

**Detailed Process**:
1. **Get Base Price**: Extract price from first target box (box 0)
   - LONG: Use box 0 HIGH
   - SHORT: Use box 0 LOW
2. **Calculate First Box Size**: `first_box_size = box_0.high - box_0.low`
3. **Calculate Direct Targets**: For each target box in `target_boxes` array:
   - Extract HIGH/LOW value based on `target_point` (HIGH for LONG, LOW for SHORT)
   - Add to `calculated_targets` array
   - Example L3: [box_0.high, box_1.high, box_2.high]
4. **Calculate Last Target** (furthest/extended target):
   - LONG: `last_target = base + first_box_size`
   - SHORT: `last_target = base - first_box_size`
   - This extends beyond the highest/lowest box boundary
5. **Sort Targets**:
   - LONG: Ascending order (closest first: `[target_0, target_1, ..., target_n]`)
   - SHORT: Descending order (closest first: `[target_0, target_1, ..., target_n]` where target_0 > target_1)
   - Ensures targets are ordered from closest to furthest

**Why Last Target is Different**: 
The last target extends beyond the highest/lowest box boundary by one full box size, representing the maximum potential move. This accounts for momentum continuation beyond the immediate box structure.

**Example L3 LONG Target Calculation**:
```
Box 0: high=2994.10, low=2894.10, size=100.00
Box 1: high=2934.50, low=2800.20, size=134.30
Box 2: high=2850.00, low=2750.00, size=100.00

Direct targets:
  Target 1: 2994.10 (Box 0 HIGH)
  Target 2: 2934.50 (Box 1 HIGH) - Note: This is actually closer than target 1
  Target 3: 2850.00 (Box 2 HIGH)

Last target: 2994.10 + 100.00 = 3094.10

After sorting (ascending): [2850.00, 2934.50, 2994.10, 3094.10]
```

**Important Note**: Direct targets are not cumulative - they are the actual HIGH/LOW values of each box. Only the last target adds the first box size.

### Risk/Reward Calculation

**Formula**: `risk_reward_ratios[i] = round(|target[i] - entry| / |entry - first_stop_loss|)` (one ratio per target)

**Detailed Process**:
1. Calculate risk: `risk = |entry - stop_losses[0]|`
2. For each target in targets array:
   - Calculate reward:
     - LONG: `reward = |target[i] - entry|`
     - SHORT: `reward = |entry - target[i]|`
   - Calculate ratio: `ratio = reward / risk`
   - Round to nearest integer
3. Return array of ratios, one per target

**Example LONG L3**:
```
Entry: $95,000
Stop Losses: [$82,000]
Targets: [$98,000, $114,320]

Risk: $95,000 - $82,000 = $13,000

Target 1 ($98,000):
  Reward: $98,000 - $95,000 = $3,000
  R:R = $3,000 / $13,000 = 0.23 → rounded to 0

Target 2 ($114,320):
  Reward: $114,320 - $95,000 = $19,320
  R:R = $19,320 / $13,000 = 1.48 → rounded to 1

Result: risk_reward = [0, 1]
```

**Example SHORT L3**:
```
Entry: $83,000
Stop Losses: [$96,000]
Targets: [$78,000, $61,680]

Risk: $96,000 - $83,000 = $13,000

Target 1 ($78,000):
  Reward: $83,000 - $78,000 = $5,000
  R:R = $5,000 / $13,000 = 0.38 → rounded to 0

Target 2 ($61,680):
  Reward: $83,000 - $61,680 = $21,320
  R:R = $21,320 / $13,000 = 1.64 → rounded to 2

Result: risk_reward = [0, 2]
```

**Note**: Ratios are rounded to integers, so small ratios may round to 0.

## Deduplication Strategies

The service implements **four independent deduplication strategies** to prevent duplicate or invalid signals. Each strategy addresses a specific edge case.

### Strategy 1: Level 1 First-Only Deduplication

**Problem**: Multiple L1 signals can be generated for the same pattern sequence while box 0 (largest box) remains unchanged. We only want the FIRST L1 signal, not subsequent ones.

**Example Scenario**:
```
L1 Signal 1: Pattern [100, -87, -75, -65, -56, -49, -42, -37, 21]
             Box 0: high=1.85148, low=1.84333
             Created at: 01:06:23

L1 Signal 2: Pattern [100, -87, -75, -65, -56, -49, -42, 37]
             Box 0: high=1.85148, low=1.84333 (SAME)
             Created at: 01:06:45

Result: Signal 2 should be FILTERED because box 0 unchanged
```

**Implementation**:
- Track active L1 signals per pair and signal type (LONG/SHORT)
- Key format: `"{pair}:{signal_type}"` (e.g., `"GBPCAD:LONG"`)
- Store box 0 high/low with each L1 signal
- If new L1 signal has same box 0 high/low → filter it
- Clear L1 tracking when box 0 changes (handled by Strategy 2)

**Note**: The internal tracking uses `box1_high` and `box1_low` field names, but these refer to box 0 (the largest box) in the 0-indexed system.

**Reasoning**: When box 0 is unchanged, subsequent L1 signals represent the same market state. The first signal is sufficient; additional signals would be redundant entries.

### Strategy 2: Box 0 State Management

**Problem**: When box 0 (largest box) changes, the market state has fundamentally shifted. Previous tracking data becomes invalid.

**Principle**: Box 0 high/low serves as the global state indicator for ALL levels.

**Why Box 0**: 
- Most stable box (changes least frequently)
- Represents the primary market direction
- When it changes, market state has fundamentally shifted

**State Change Detection**:
1. Track box 0 high/low per pair: `pair → (high, low)`
2. On each pattern check:
   - Compare current box 0 high/low with stored state
   - If changed (tolerance: 0.00001) → **state change detected**

**State Change Actions**:
When box 0 changes, immediately clear:
- ✅ All active L1 signals for that pair
- ✅ Update box 0 state to new values

**Why This Works**: 
- Prevents cache growth: Old tracking data is automatically cleared when market state changes
- Maintains accuracy: Fresh start ensures deduplication logic applies to current state only
- Prevents false positives from stale data

**Example**:
```
State 1: Box 0 = (1.85148, 1.84333)
  - Track L1 signals
  - Market moves, boxes update
  - Box 0 changes to (1.85200, 1.84400)

State 2: Box 0 = (1.85200, 1.84400)  
  - ALL previous L1 tracking cleared
  - Fresh tracking starts
  - New L1 signals tracked from clean slate
```

### Strategy 3: Higher Level Preference (Subset Removal)

**Problem**: The same pattern sequence can match at multiple levels simultaneously. Lower-level patterns are often subsets of higher-level patterns. We should prefer the highest level since it represents deeper fractal structure.

**Example Scenario**:
```
Pattern Sequence: [6503, -5637, ..., 1004, -870, ..., 37, -32, 16]

Detected as:
  - Level 5: Full sequence [6503, ..., 16] → Target: 1.89148
  - Level 4: Sub-sequence [1004, ..., 16] → Target: 1.85148

Both detected at same timestamp.

Result: Keep Level 5, filter Level 4 (L4 is subset of L5)
```

**Implementation**:
- Sort patterns by level descending (highest first)
- For each pattern, check if its values are a subset of any higher-level pattern already kept
- If subset found → filter (duplicate)
- If not a subset → keep (unique pattern)
- Only compare patterns of the same signal type (LONG vs SHORT)

**Reasoning**: Higher levels represent deeper market structure and stronger signals. When a lower-level pattern is a subset of a higher-level pattern, the higher level contains all the information. Lower level duplicates are redundant.

### Strategy 4: Structural Boxes Deduplication

**Problem**: The same pattern sequence can be generated multiple times while the structural boxes (the boxes that define the pattern's structure) remain unchanged. We should only send one signal per pattern sequence while structural boxes' high/low values remain stable.

**Structural Boxes Definition**:
- **LONG signals**: Structural boxes are all **positive** boxes in the pattern sequence
- **SHORT signals**: Structural boxes are all **negative** boxes in the pattern sequence
- Structural boxes are sorted by absolute value descending (box 0 = largest, box 1 = second largest, etc.)
- **Entry box exclusion**: The entry box is at index `level` (0-indexed) in the sorted structural boxes array:
  - **L1**: Tracks box 0 only (excludes entry box at index 1)
  - **L2**: Tracks boxes 0-1 (excludes entry box at index 2)
  - **L3**: Tracks boxes 0-2 (excludes entry box at index 3)
  - **L4**: Tracks boxes 0-3 (excludes entry box at index 4)
  - **L5**: Tracks boxes 0-4 (excludes entry box at index 5)
  - **L6**: Tracks boxes 0-5 (excludes entry box at index 6)
- The entry box is the **trigger point** and is **NOT** considered structural. It's excluded from tracking because it naturally shifts with price movement and is the trigger point, not part of the structural pattern.

**Example Scenario - L5 SHORT Signal**:
```
SHORT Signal 1 (L5):
  Pattern: [-3173, 2748, 2379, -1159, 1000, -488, 422, 366, -178, 154, 133, 115, 100, -49, 42, -24]
  Structural boxes (negative, sorted by abs desc): -3173, -1159, -488, -178, -49, -24
  Tracked boxes (0-4, excluding entry at index 5): -3173, -1159, -488, -178, -49
  Entry box (index 5, not tracked): -24
  Timestamp: 15:38:20.185

SHORT Signal 2 (L5, 20 seconds later):
  Pattern: [-3173, 2748, 2379, -1159, 1000, -488, 422, 366, -178, 154, 133, 115, 100, -49, 42, -24]
  Tracked boxes: -3173, -1159, -488, -178, -49 (SAME high/low)
  Entry box: -24 (shifted from 2928.5→2928.2, but not checked)
  Timestamp: 15:38:41.023

Result: Signal 2 should be FILTERED (all tracked structural boxes unchanged, entry box shift ignored)
```

**Example Scenario 2 - Structural Box Changed (L3 SHORT)**:
```
SHORT Signal 1 (L3):
  Pattern: [-3173, -1159, -488, -178, -75, -10]
  Structural boxes (negative, sorted): -3173, -1159, -488, -178, -75, -10
  Tracked boxes (0-2, excluding entry at index 3): -3173, -1159, -488
  Entry box (index 3, not tracked): -178
  Structural box -488: high=2943.6, low=2894.8
  Timestamp: 13:36:44.978

SHORT Signal 2 (L3, later):
  Pattern: [-3173, -1159, -488, -178, -75, -10] (SAME pattern)
  Tracked boxes: -3173, -1159, -488
  Structural box -488: high=2944.0, low=2895.2 (CHANGED)
  Timestamp: 13:40:12.345

Result: Signal 2 should be ALLOWED (tracked structural box -488 high/low changed, tracker reset)
```

**Implementation**:
1. Filter structural boxes by signal type (positive for LONG, negative for SHORT)
2. Sort structural boxes by absolute value descending (box 0 = largest, box 1 = second largest, etc.)
3. Track boxes 0 through (level-1), excluding entry box at index `level`:
   - L1: Track box 0 only
   - L2: Track boxes 0-1
   - L3: Track boxes 0-2
   - L4: Track boxes 0-3
   - L5: Track boxes 0-4
   - L6: Track boxes 0-5
4. Track structural boxes' high/low values per pattern sequence
5. Key format: `"{pair}:{signal_type}:{structural_key}"` where structural_key is tracked boxes joined by "_"
6. For each tracked structural box:
   - Store `integer_value → (high, low)` mapping
7. Check before sending:
   - If same pattern sequence seen before:
     - If ALL tracked structural boxes' high/low unchanged → filter (duplicate)
     - If ANY tracked structural box's high/low changed → allow (new pattern state, update tracking)
   - If pattern sequence never seen → allow (first occurrence, create tracking)
8. Tolerance: 0.00001 (accounts for floating-point precision)
9. Entry box changes are ignored for deduplication purposes

**Reasoning**: Structural boxes define the pattern's core structure. When they remain unchanged, the pattern represents the same market state regardless of entry box changes. Only when tracked structural boxes' high/low values change does the pattern represent a new market state, warranting a new signal.

**Key Insight**: The entry box is the trigger point and naturally shifts with price movement. It's not part of the structural pattern, so it's excluded from deduplication tracking. We track structural boxes from largest to the box before the entry box (boxes 0 through level-1). If ANY tracked structural box's high/low changes, the tracker resets for that specific pattern sequence, allowing a new signal. This ensures we don't spam duplicate signals while the pattern structure remains stable, but we do allow new signals when the underlying structure shifts.

## Signal Generation Process Flow

**Complete Flow**:
1. **Pattern Detection**: `scanner.detect_patterns()` returns all matching patterns
2. **Initial Deduplication**: `should_filter_pattern()` filters L1 duplicates
3. **Subset Removal**: `remove_subset_duplicates()` prefers higher levels
4. **Signal Generation**: `generate_signals()` creates signals for each pattern:
   - Extract primary boxes (positive for LONG, negative for SHORT)
   - Sort by absolute value descending
   - Find matching trade rule by level
   - Calculate entry, stop_losses, targets, risk_reward
5. **Validation**: Check entry, stop_losses, targets are all present and valid
6. **Structural Deduplication**: `should_filter_structural_boxes()` final check
7. **Storage**: Add to Supabase and in-memory tracker
8. **Forwarding**: Send to main server via HTTP POST

**Code Flow**:
```
process_box_update()
  ├─> tracker.check_price() [Settlement check]
  ├─> scanner.detect_patterns() [Pattern matching]
  ├─> deduplicator.should_filter_pattern() [L1 + Box 0 state]
  ├─> deduplicator.remove_subset_duplicates() [Higher level preference]
  ├─> generator.generate_signals() [Signal creation]
  │   ├─> Extract primary boxes
  │   ├─> Find trade rule
  │   ├─> Calculate entry
  │   ├─> Calculate stop_losses
  │   ├─> Calculate targets
  │   └─> Calculate risk_reward
  ├─> Validate signal (entry, stop_losses, targets)
  ├─> deduplicator.should_filter_structural_boxes() [Final deduplication]
  ├─> tracker.add_signal() [Store in Supabase + memory]
  └─> signal_tx.send() [Forward to main server]
```

## Signal Tracking & Settlement

### Data Storage Format

**Pattern Sequence**:
- Type: `Vec<i32>` (array of integers)
- Content: Integer box values only (e.g., `[1000, -866, -750, -650, ...]`)
- Purpose: Represents the pattern structure for matching and deduplication
- Database: Stored as `integer[]` column in Supabase

**Box Details**:
- Type: `Vec<BoxDetail>` (array of box detail objects)
- Content: Each `BoxDetail` contains:
  - `integer_value: i32` - The box integer value
  - `high: f64` - Box high price boundary
  - `low: f64` - Box low price boundary
  - `value: f64` - Box value (same as integer_value * point)
- Purpose: Provides price context for each box in the pattern
- Database: Stored as JSON array in Supabase `box_details` column
- Frontend: Used to format pattern display with high/low values

**Example**:
```json
{
  "pattern_sequence": [1000, -866, -750, -650],
  "box_details": [
    {"integer_value": 1000, "high": 2994.10, "low": 2894.10, "value": 1000.0},
    {"integer_value": -866, "high": 2934.50, "low": 2800.20, "value": -866.0},
    {"integer_value": -750, "high": 2850.00, "low": 2750.00, "value": -750.0},
    {"integer_value": -650, "high": 2800.00, "low": 2700.00, "value": -650.0}
  ]
}
```

### Active Signal Tracking

**Purpose**: Monitor signals until they hit stop loss or target

**Storage**: 
- In-memory: `SignalTracker` maintains active signals per pair
- Persistent: Supabase `signals` table with both `pattern_sequence` and `box_details`

**Signal Lifecycle**:
1. **Created**: Signal generated and added to tracker
2. **Active**: Price monitoring begins
   - Track which targets are hit: `target_hits: Vec<Option<(timestamp, price)>>`
   - Track stop loss hit: `stop_loss_hit: Option<(timestamp, price)>`
3. **Settled**: Price hits stop loss or final target
   - Status: `"success"` (hit final target) or `"failed"` (hit stop loss)
   - Settled price derived from hit data (stop loss hit or final target hit)
   - Remove from active tracking

### Settlement Logic

**Check Frequency**: Every box update (real-time, typically multiple times per second)

**Detailed Process**:
1. **Get Active Signals**: Retrieve all active signals for the pair
2. **Check Each Signal**:
   - Check stop loss hit first (if not already hit)
   - Check target hits (for all targets not yet hit)
   - Determine if signal should be settled
3. **Update Supabase**: Write target hits and stop loss hits to database
4. **Settle Signals**: Remove from active tracking, update status

**Target Hit Tracking** (`tracker.rs::check_target_hits()`):
- For each target in `targets` array, check if price reached it
- Check condition:
  - LONG: `current_price >= target_price`
  - SHORT: `current_price <= target_price`
- When hit, store `(timestamp, price)` in `target_hits` array at corresponding index
- Multiple targets can be hit sequentially (partial profit-taking)
- Targets are checked in order, but all can be hit before settlement
- Once a target is hit, it's not checked again (stored in array)

**Stop Loss Hit Tracking** (`tracker.rs::check_stop_loss_hit()`):
- Check if price hit first stop loss in `stop_losses` array (currently only one stop loss)
- Check condition:
  - LONG: `current_price <= stop_loss`
  - SHORT: `current_price >= stop_loss`
- When hit, store `(timestamp, price)` in `stop_loss_hit`
- Stop loss hit immediately settles the signal as "failed"
- Only checked if not already hit (early exit)

**LONG Signal Settlement**:
- **Success**: `current_price >= final_target` (last target in array) → Status: `"success"`
- **Failed**: `current_price <= stop_losses[0]` → Status: `"failed"`
- **Active**: Price between stop loss and final target

**SHORT Signal Settlement**:
- **Success**: `current_price <= final_target` (last target in array) → Status: `"success"`
- **Failed**: `current_price >= stop_losses[0]` → Status: `"failed"`
- **Active**: Price between final target and stop loss

**Settled Price Calculation**:
- **Failed**: `settled_price = stop_loss_hit.price` (from stop loss hit data)
- **Success**: `settled_price = target_hits.last().price` (from final target hit data)
- No separate `settled_price` field stored - calculated from hit tracking
- If hit data missing, defaults to 0.0 (should not happen in practice)

**Settlement Priority**:
1. Stop loss hit → immediate settlement (failed)
2. Final target hit → settlement (success)
3. Partial targets hit → continue monitoring

### Settlement Cleanup

When a signal is settled:
1. Calculate settled price from hit data (stop loss or final target)
2. Update Supabase with status and settled price
3. If L1 signal → remove from L1 deduplication tracking
4. Remove from in-memory active signals
5. Log settlement event with hit statistics

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

## Data Structures

### SignalMessage

```rust
pub struct SignalMessage {
    pub id: Option<i64>,              // Supabase id (set after insert)
    pub pair: String,                 // Trading pair
    pub signal_type: String,          // "LONG" or "SHORT"
    pub level: u32,                   // 1-6
    pub pattern_sequence: Vec<i32>,   // Integer box values
    pub box_details: Vec<BoxDetail>,  // Box details with high/low
    pub complete_box_snapshot: Vec<i32>, // Full pattern (same as pattern_sequence)
    pub entry: Option<f64>,           // Entry price
    pub stop_losses: Vec<f64>,        // Array of stop loss prices (currently one)
    pub targets: Vec<f64>,            // Array of cumulative target prices
    pub risk_reward: Vec<f64>,        // Risk/reward ratio per target
}
```

**JSON Example**:
```json
{
  "signal_id": "GBPCAD_LONG_1000_-866_-750_-650_1703123456789",
  "pair": "GBPCAD",
  "signal_type": "LONG",
  "level": 3,
  "pattern_sequence": [1000, -866, -750, -650, -563, 274, -237, -205, -178],
  "timestamp": 1703123456789,
  "box_details": [
    {"integer_value": 1000, "high": 2994.10, "low": 2894.10, "value": 1000.0},
    {"integer_value": -866, "high": 2934.50, "low": 2800.20, "value": -866.0},
    {"integer_value": -750, "high": 2850.00, "low": 2750.00, "value": -750.0}
  ],
  "complete_box_snapshot": [1000, -866, -750, -650, -563, 274, -237, -237, -205, -178],
  "entry": 2850.00,
  "stop_losses": [2750.00],
  "targets": [2994.10, 3158.42],
  "risk_reward": [1.44, 3.08]
}
```

### BoxDetail

```rust
pub struct BoxDetail {
    pub integer_value: i32,  // The box integer value
    pub high: f64,           // Box high price boundary
    pub low: f64,            // Box low price boundary
    pub value: f64,          // Box value (same as integer_value * point)
}
```

## Pattern Database Generation

**Process**:
1. Start with `STARTING_POINTS` array (24 values: 10000, 8660, 7506, ..., 366)
2. For each starting point, recursively traverse all possible paths:
   - Start with path = `[starting_point]`
   - Look up `BOXES[starting_point.abs()]` for valid patterns
   - For each pattern:
     - Adjust pattern sign based on current key
     - Extend path with pattern values
     - Check for cycle (if last value abs equals current key abs)
     - If cycle → save path and continue
     - If no cycle → recursively continue from last value
   - If no patterns found → save current path (terminal path)

**Key Features**:
- **Self-terminating patterns**: Patterns like `[24]` when at key 24 terminate immediately
- **Cycle detection**: Prevents infinite loops when pattern returns to same key
- **Sign adjustment**: Patterns are adjusted based on current key sign (positive/negative)
- **Recursive traversal**: Explores all possible paths from each starting point

**Result**: ~1,506,648 unique traversal paths stored in memory

**Performance**: Generation happens once at startup, typically takes < 1 second

## Pattern Matching Algorithm

**Detailed Algorithm**:
1. **Input Validation**: Return empty if boxes array is empty
2. **Integer Conversion**: Convert all box values to integers using instrument point
3. **HashSet Creation**: Create HashSet from integer values for O(1) lookup
4. **Path Iteration**: For each of ~1.5M pre-computed paths:
   - **Early Exit Check**: First check if `path[0]` exists in boxes (optimization)
   - **Full Match Check**: If first exists, check ALL remaining values exist
   - **LONG Pattern**: Use path as-is if first value is positive
   - **SHORT Pattern**: Invert path (multiply all by -1) if first value is negative
5. **PatternMatch Creation**: For each match:
   - Extract box details (high/low) for each path value
   - Calculate level using `calculate_level()`
   - Create `PatternMatch` struct

**Optimization**: HashSet membership check is O(1), making overall algorithm O(n*m) where n=paths, m=path length

## Level Calculation

**Detailed Algorithm**:
```rust
fn calculate_level(path: &[i32]) -> u32 {
    if path.len() <= 1 { return 1; }
    
    let mut level = 0u32;
    let mut idx = 0;
    let mut key = path[0];
    
    while idx < path.len() - 1 {
        // Get patterns for current key
        let patterns = BOXES.get(&key.abs())?;
        
        // Try to find matching pattern
        for pattern in patterns {
            let adjusted = if key > 0 { pattern } else { invert(pattern) };
            let end = idx + 1 + adjusted.len();
            
            if end <= path.len() && path[idx+1..end] == adjusted {
                level += 1;  // One complete reversal found
                idx = end - 1;
                key = adjusted.last();
                break;
            }
        }
        
        // If no match found, stop
        if no_match { break; }
    }
    
    level.max(1)  // Minimum level is 1
}
```

**Edge Cases**:
- Path length ≤ 1: Always returns level 1
- No pattern matches: Returns level 1 (minimum)
- Multiple possible matches: Uses first match found (left-to-right)

## Error Handling

**Signal Generation Errors**:
- **Invalid Signal Data**: Pattern matches but trade rule calculation fails (missing boxes, invalid prices, empty arrays)
  - Handling: Signal filtered before sending
- **Missing Trade Rule**: Pattern level has no matching trade rule
  - Handling: Signal generated with empty entry/stop_losses/targets, filtered

**Database Errors**:
- **Supabase Write Failures**: Failed to insert signal to Supabase
  - Handling: Return id=0, signal still forwarded, in-memory tracking continues
- **Supabase Update Failures**: Failed to update target hits or status
  - Handling: Continue, in-memory state updated, no retry

**Network Errors**:
- **WebSocket Disconnection**: Connection to boxes.rthmn.com drops
  - Handling: State persists, fresh auth on reconnect
- **Main Server Forwarding Failures**: Failed to forward signal to server.rthmn.com
  - Handling: Signal still stored in Supabase, no retry

**Configuration Errors**:
- **Missing Instrument Config**: Unknown trading pair
  - Handling: Uses default point=0.01, may cause incorrect matching
- **Missing Environment Variables**: Required env vars not set
  - Handling: SUPABASE_URL/KEY panic, PORT defaults to 3003, MAIN_SERVER_URL defaults

**Data Validation Errors**:
- **Empty Box Array**: Box update with empty boxes
  - Handling: Early return, no processing
- **Invalid Box/Price Data**: Cannot parse from JSON
  - Handling: Defaults to empty/0.0, early return

## Performance Characteristics

### Pattern Matching
- **Pattern Database**: ~1.5 million paths loaded at startup
- **Matching Algorithm**: O(n*m) where n = patterns, m = box count
- **Optimization**: Early exit on first mismatch, HashSet for O(1) lookups
- **Throughput**: Processes box updates in <1ms per update

### Memory Usage
- **Pattern Storage**: ~50-100MB (depends on path lengths, static, loaded at startup)
- **Deduplication State**: Bounded by:
  - Active pairs (typically 10-50)
  - Structural boxes tracking (per pattern sequence, persists across box 0 changes)
  - L1 signal tracking (cleared on box 0 change)
  - Box 0 state tracking (one entry per pair, serves as global state indicator)
- **Active Signals**: ~1KB per signal, bounded by active pairs
- **Tolerance**: 0.00001 for floating-point comparisons

### Throughput
- **Box Updates**: Processed in <1ms per update
- **Signal Generation**: <10ms per signal
- **Supabase Writes**: Async, non-blocking
- **WebSocket**: Handles multiple pairs concurrently

## Testing

### Unit Tests
```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_name
```

### Pattern Tests
```bash
# Run pattern generation tests
cargo test --test patterns_test

# Generate pattern output file
cargo test --test patterns_test test_generate_all_paths -- --nocapture
```

**Output**: `paths_output.txt` with all 1,506,648 paths

**Test Coverage**:
- Pattern generation algorithm (`traverse_all_paths`)
- Path traversal correctness
- Cycle detection (patterns that return to same key)
- Self-terminating patterns (single-element patterns)
- Pattern count verification
- Starting points coverage

### Integration Testing

**Manual Testing**:
1. Start service: `cargo run --release`
2. Connect WebSocket client to `ws://localhost:3003/ws`
3. Send auth message: `{"type": "auth", "token": "test"}`
4. Send box update: `{"type": "boxUpdate", "pair": "GBPCAD", "data": {...}}`
5. Verify signal generation in logs
6. Check Supabase for signal storage
7. Verify forwarding to main server

**Test Scenarios**:
- Multiple patterns detected simultaneously
- L1 deduplication (same box 0)
- Box 0 state change
- Subset removal (L4 vs L5)
- Structural boxes deduplication
- Signal settlement (target hit, stop loss hit)
- WebSocket reconnection

## Monitoring & Observability

### Metrics (via GET /api/status)

**Available Metrics**:
- `scanner.totalPaths`: Total pattern paths loaded (~1,506,648)
- `scanner.isInitialized`: Scanner initialization status (true/false)
- `signalsSent`: Total signals forwarded to main server (cumulative counter)
- `activeSignals.total`: Current active signals across all pairs
- `activeSignals.byPair`: Active signals per trading pair (HashMap)

**Use Cases**:
- Monitoring signal generation rate
- Tracking active signal count
- Identifying pairs with most signals
- Verifying scanner initialization
- Performance monitoring

### Health Checks

**GET /health**:
- Returns: `{"status": "ok", "service": "signals.rthmn.com (rust)", "timestamp": "..."}`
- Use for: Load balancer health checks, uptime monitoring
- No database or external service checks (lightweight)

**Supabase Connectivity**:
- Implicit check via write operations
- Does not affect service availability (fail-fast, continue processing)

## Deployment

### Railway Deployment
1. Root directory: `signals.rthmn.com`
2. Builder: Dockerfile
3. Add environment variables in Railway dashboard:
   - `SUPABASE_URL`
   - `SUPABASE_SERVICE_ROLE_KEY`
   - `MAIN_SERVER_URL` (optional)
   - `PORT` (optional, default 3003)
4. Deploy automatically on git push

### Docker Build
```bash
docker build -t signals-rthmn .
docker run -p 3003:3003 --env-file .env signals-rthmn
```

**Dockerfile Details**:
- Multi-stage build: Rust builder + Debian slim runtime
- Optimized release build with LTO
- Minimal runtime image (~50MB)
- Exposes port 3003

### Local Development
```bash
# Install dependencies
cargo build

# Run with debug logging
RUST_LOG=signals_rthmn=debug cargo run

# Run release build
cargo run --release
```

## Edge Cases & Special Handling

### Case 1: Multiple Patterns Detected Simultaneously
**Scenario**: Single box update triggers multiple pattern matches

**Handling**:
1. Detect all matching patterns
2. Apply deduplication filters to each
3. Group by pattern sequence (prefer highest level)
4. Generate signals for unique patterns only

### Case 2: Pattern Sequence Appears at Multiple Levels
**Scenario**: Same sequence matches L4 and L5 simultaneously

**Handling**: Strategy 3 (Higher Level Preference) filters lower level via `remove_subset_duplicates()`

**Example**:
```
Pattern: [6503, ..., 16]
- Detected as L5 → Keep
- Detected as L4 → Filter (L5 already exists, L4 is subset)
```

### Case 3: Box 0 Changes During Pattern Detection
**Scenario**: Box 0 high/low changes between pattern checks

**Handling**: 
- State change detected → clear all L1 tracking
- New patterns tracked with fresh state
- Previous patterns invalidated (different market state)
- Structural boxes tracking persists (tracked per pattern sequence, not cleared on box 0 change)

### Case 4: Duplicate Pattern Sequences
**Scenario**: Same pattern sequence detected multiple times with same structural boxes

**Handling**: 
- Strategy 4 (Structural Boxes Deduplication) filters duplicates
- Only first signal sent if all structural boxes' high/low unchanged
- Subsequent signals filtered until any structural box changes
- No time-based window - purely based on structural box state

### Case 5: Invalid Signal Data
**Scenario**: Pattern matches but trade rule calculation fails (missing boxes, invalid prices, empty arrays)

**Handling**:
- Signal generated but entry, stop_losses, or targets are invalid/empty
- Signal not sent to users (filtered before sending)

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

### Case 8: Missing Instrument Configuration
**Scenario**: Unknown trading pair (not in `instruments.rs`)

**Handling**:
- Uses default point value: 0.01
- May cause incorrect pattern matching for exotic pairs

### Case 9: Empty Box Array
**Scenario**: Box update received with empty boxes array

**Handling**:
- Early return, no processing

### Case 10: Price Hits Multiple Targets Simultaneously
**Scenario**: Price jumps past multiple targets in single update

**Handling**:
- All targets checked sequentially
- All hit targets recorded with same timestamp
- Settlement occurs if final target hit

## Complete End-to-End Example

This section demonstrates a complete signal detection and settlement flow.

### Scenario: GBPCAD LONG L3 Signal Detection and Settlement

**Step 1: Receive Box Update**
```json
{
  "type": "boxUpdate",
  "pair": "GBPCAD",
  "data": {
    "boxes": [
      {"high": 1.85148, "low": 1.84333, "value": 815},
      {"high": 1.85000, "low": 1.84400, "value": 600},
      {"high": 1.84800, "low": 1.84200, "value": 600},
      {"high": 1.84600, "low": 1.84000, "value": 600}
    ],
    "price": 1.84349,
    "timestamp": "2025-12-19T01:06:23.123Z"
  }
}
```

**Step 2: Convert to Integers**
- GBPCAD point = 0.00001
- Box 0: 815 / 0.00001 = 81500
- Box 1: 600 / 0.00001 = 60000
- Integer values: `[81500, 60000, 60000, 60000]`
- Value set: `{81500, 60000}`

**Step 3: Pattern Matching**
- Scanner checks ~1.5M pre-computed paths
- Path found: `[1000, -866, -750, -650, -563, 274, -237, -205, -178]`
- Match found: LONG pattern (first value positive)

**Step 4: Calculate Level**
- Path: `[1000, -866, -750, -650, -563, 274, -237, -205, -178]`
- Reversal 1: 1000 → matches pattern ending at 274
- Reversal 2: 274 → matches pattern ending at -178
- Reversal 3: -178 → no more matches
- Level = 3 (3 complete reversals)

**Step 5: Apply Deduplication Filters**
1. **L1 First-Only**: Not L1, skip
2. **Box 0 State**: Check box 0 (1.85148, 1.84333), no change from previous state
3. **Subset Removal**: Check if lower-level subset exists, none found, keep
4. **Structural Boxes**: First occurrence of this pattern sequence, allow

**Step 6: Generate Signal**
- Pattern: `[1000, -866, 750, -650, 563, 274, -237, 205, -178, 154]`
- Primary boxes (positive for LONG): `[1000, 750, 563, 274, 205, 154]`
- Sorted: `[1000, 750, 563, 274, 205, 154]`
- L3 Rule:
  - Entry: Box 3 HIGH = 274.high
  - Stop: Box 2 LOW = 563.low
  - Targets: [Box 0 HIGH, Box 1 HIGH, Box 2 HIGH] + last target

**Step 7: Store & Forward**
- Signal stored in Supabase
- Forwarded to server.rthmn.com via HTTP POST
- Added to active tracking for settlement monitoring

**Step 8: Settlement (Price Movement)**
- Initial price: 1.84349
- Entry: 1.84600 (break above to enter)
- Price moves up, hits Target 1: 1.85148 → recorded
- Price continues, hits Target 2: 1.85500 → recorded  
- Price hits final target: 1.86000 → settlement triggered
- Status: "success"
- Settled price: 1.86000 (from final target hit)
- Removed from active tracking
