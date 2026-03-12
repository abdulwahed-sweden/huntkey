# Monitoring Dashboard

## Overview

The monitoring dashboard provides aggregated protocol metrics from the structured event log (black box). It supports real-time snapshots, time-range filtering, and JSON export for dashboard consumption.

## Rust API

### DashboardState

```rust
use huntkey::{EventLog, DashboardState, export_dashboard_state};

let mut log = EventLog::new();
// ... events recorded via IdentityWatcher ...

let dashboard = DashboardState::new(&log);

// Full snapshot
let snap = dashboard.snapshot(now);
println!("Active identities: {}", snap.active_identities);
println!("Executed intents: {}", snap.executed_intents);

// Time-range filtered snapshot
let snap = dashboard.snapshot_in_range(from, to, now);

// Filter entries
let entries = dashboard.entries_in_range(from, to);
let by_type = dashboard.entries_by_type(EventType::IntentExecuted);
let by_id = dashboard.entries_for_identity(&identity);

// Export as JSON
let json = export_dashboard_state(&log, now);
```

### DashboardSnapshot

| Field | Type | Description |
|-------|------|-------------|
| `active_identities` | `usize` | Unique identities with recorded events |
| `pending_recoveries` | `usize` | Identities in RecoveryPending state |
| `executed_intents` | `usize` | Total intent executions |
| `high_value_intents` | `usize` | Intents above the high-value threshold |
| `revoked_sessions` | `usize` | Session epoch revocations |
| `snapshot_timestamp` | `u64` | When the snapshot was generated |

## TypeScript SDK

### ProtocolDashboard

```typescript
import { ProtocolAuditor, ProtocolDashboard } from "@huntkey/sdk";

const auditor = new ProtocolAuditor(provider, contractAddress);
const dashboard = new ProtocolDashboard(auditor);

// Batch query identity states
const states = await dashboard.batchGetIdentityState(rootAddresses);

// Count by state
const counts = await dashboard.countByState(rootAddresses);
console.log(`Active: ${counts.active}`);
console.log(`Recovery Pending: ${counts.recoveryPending}`);
console.log(`Frozen: ${counts.frozen}`);
```

## JSON Export Format

```json
{
  "active_identities": 42,
  "pending_recoveries": 1,
  "executed_intents": 1337,
  "high_value_intents": 5,
  "revoked_sessions": 3,
  "snapshot_timestamp": 1710000000
}
```

## Event Log Integration

The dashboard reads from the `EventLog` (black box) which is automatically populated by `IdentityWatcher` event handlers:

```
IdentityWatcher.on_intent_executed()        → EventType::IntentExecuted
IdentityWatcher.on_session_invalidated()    → EventType::SessionInvalidated
IdentityWatcher.on_recovery_state_changed() → EventType::RecoveryStateChanged
IdentityWatcher.on_high_value_intent()      → EventType::HighValueIntent
```

All event types and their metadata are preserved in the log for time-range filtering and forensic analysis.
