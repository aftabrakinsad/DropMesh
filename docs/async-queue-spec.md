# DropMesh — Async Queue System

> The feature that makes DropMesh different from LocalSend: files persist and deliver automatically when the target device comes online.

## How it works

1. **User drops a file** → file is encrypted and copied to `staging/{queue_id}/`
2. **Queue entry created** in SQLite with target device, file metadata, and status "pending"
3. **Delivery watcher** runs in background, listening for mDNS discovery events
4. **Target comes online** → watcher sees the device, initiates P2P transfer automatically
5. **Transfer completes** → queue entry marked "delivered", staging files cleaned up
6. **Target stays offline** → file stays in staging indefinitely, retried every time the device appears

## Queue states

```
pending → delivering → delivered
                    → failed (retry on next online event)
pending → expired (user-configurable TTL, default: 7 days)
pending → cancelled (user manually cancels)
```

## Key design decisions

- Files are staged BEFORE the target is checked — always async-first
- Staging uses the user's allocated storage quota
- Each queued file is encrypted at rest with the group key
- Queue entries survive app restarts (persisted in SQLite)
- The watcher is event-driven (mDNS), not polling-based
- Multiple files to the same device are batched in delivery order
