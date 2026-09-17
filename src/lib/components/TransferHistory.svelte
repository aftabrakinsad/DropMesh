<script>
  import { completedTransfers, formatBytes } from "../stores/transfers.js";
</script>

<div class="history">
  {#if $completedTransfers.length === 0}
    <div class="empty">
      <div class="empty-text">No transfer history yet</div>
    </div>
  {:else}
    {#each $completedTransfers as transfer, i}
      <div class="history-item" style="animation-delay: {i * 60}ms">
        <div
          class="direction-icon"
          class:sent={transfer.direction === "sent"}
          class:received={transfer.direction === "received"}
          class:failed={transfer.status === "failed"}
        >
          {#if transfer.status === "failed"}✕
          {:else if transfer.direction === "sent"}↑
          {:else}↓
          {/if}
        </div>

        <div class="info">
          <div class="file-name truncate">{transfer.file_name}</div>
          <div class="meta">
            {transfer.direction === "sent" ? "→" : "←"}
            {transfer.peer_device_name || transfer.peer_device_id}
            · {formatBytes(transfer.file_size)}
          </div>
        </div>

        <div class="status-col">
          <div class="status" class:failed={transfer.status === "failed"}>
            {transfer.status}
          </div>
          <div class="time">
            {#if transfer.completed_at}
              {new Date(transfer.completed_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
            {/if}
          </div>
        </div>
      </div>
    {/each}
  {/if}
</div>

<style>
  .history {
    animation: float-in 0.3s ease-out;
  }

  .history-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
    border-radius: var(--radius-md);
    margin-bottom: 6px;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border-subtle);
    animation: float-in 0.3s ease-out both;
  }

  .direction-icon {
    width: 32px;
    height: 32px;
    border-radius: 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 14px;
    flex-shrink: 0;
  }

  .direction-icon.sent {
    background: var(--accent-dim);
    color: var(--accent);
  }

  .direction-icon.received {
    background: var(--info-dim);
    color: var(--info);
  }

  .direction-icon.failed {
    background: var(--danger-dim);
    color: var(--danger);
  }

  .info {
    flex: 1;
    min-width: 0;
  }

  .file-name {
    font-size: 13px;
    font-weight: 500;
  }

  .meta {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .status-col {
    text-align: right;
    flex-shrink: 0;
  }

  .status {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .status.failed {
    color: var(--danger);
  }

  .time {
    font-size: 11px;
    color: var(--text-muted);
  }

  .empty {
    text-align: center;
    padding: 40px 20px;
  }

  .empty-text {
    font-size: 14px;
    color: var(--text-secondary);
  }
</style>
