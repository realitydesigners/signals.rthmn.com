# Signals Table Migration Summary

## Current Database Structure (Verified)

Based on actual data from your database:

```json
{
  "targets": [507.55, 505.38, 501.72],           // numeric[] - array of prices
  "stop_losses": [508.41],                        // numeric[] - array of prices
  "target_hits": [                                // jsonb - array of {price, timestamp} or null
    null,
    null,
    {"price": 5.805, "timestamp": "2026-01-03T15:20:08.089000Z"}
  ],
  "stop_loss_hit": {                              // jsonb - object {price, timestamp} or null
    "price": 508.61,
    "timestamp": "2026-01-03T15:25:22.044000Z"
  }
}
```

## Target Structure

```json
{
  "targets": [                                    // jsonb - array of {price, timestamp}
    {"price": 507.55, "timestamp": null},
    {"price": 505.38, "timestamp": null},
    {"price": 501.72, "timestamp": "2026-01-03T15:20:08.089000Z"}
  ],
  "stop_losses": [                                // jsonb - array of {price, timestamp}
    {"price": 508.41, "timestamp": "2026-01-03T15:25:22.044000Z"}
  ]
}
```

## What's Been Updated

### ✅ Code Changes (Ready)
1. **Rust Backend** (`signals.rthmn.com`):
   - Updated types to use `Target[]` and `StopLoss[]` with embedded timestamps
   - Updated tracker to set timestamps directly on targets/stop_losses
   - Updated Supabase client to store new format

2. **TypeScript Frontend** (`server.rthmn.com`):
   - Updated data types to use new structure
   - Updated formatters to work with new format
   - Updated columns and display components
   - Added normalization utilities for backward compatibility

3. **Backend Routes**:
   - Added normalization functions to handle both old and new formats
   - Updated `getSignalStats.ts` to use normalized data

### ⚠️ Database Migration (Required)

**Before deploying code changes, you MUST run the database migration:**

1. **Backup your database** (critical!)
2. **Run `migration_check.sql`** to verify current schema
3. **Run `migration.sql`** step by step:
   - Step 1: Add temporary columns
   - Step 2: Migrate data
   - Step 3: Verify results
   - Step 4: Drop old columns (after verification)
   - Step 5: Rename new columns

## Migration Steps

1. **Check Schema**: Run `migration_check.sql` in Supabase SQL Editor
2. **Backup Database**: Create a backup before proceeding
3. **Run Migration**: Execute `migration.sql` step by step
4. **Verify**: Check migrated data looks correct
5. **Deploy Code**: Once migration is complete, deploy the updated code

## Important Notes

- **Timestamps are already in ISO format** (from recent fix) - no conversion needed
- **Migration merges data by index** for targets (targets[0] with target_hits[0])
- **Migration matches by price** for stop_losses (within 0.0001 tolerance)
- **Code includes backward compatibility** - can handle both formats during transition
- **Test on staging first** before running on production

## Rollback

If migration fails, you can restore from backup. The old columns will still exist until Step 4 (drop columns) is executed.

