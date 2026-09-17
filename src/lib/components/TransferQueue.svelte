<script>
  import { activeTransfers } from "../stores/transfers.js";
  import { formatBytes } from "../stores/transfers.js";
  import { cancelTransfer } from "../api/tauri.js";

  async function handleCancel(transferId) {
    try {
      await cancelTransfer(transferId);
    } catch (e) {
      console.error("Cancel failed:", e);
    }
  }
</script>

<div class="queue">
  {#if $activeTransfers.length === 0}
    <div class="empty">
      <div class="empty-text">No active transfers</div>
      <div class="empty-hint">Drop a file onto a device to start</div>
    </div>
  {:else}
    {#each $activeTransfers as transfer, i}
      <div class="transfer-card" style="animation-delay: {i * 80}ms">
        <div class="transfer-header">
          <div class="transfer-info">
            <div class="file-name truncate">{transfer.file_name}</div>
            <div class="file-meta">
              {formatBytes(transfer.file_size)} ·
              {transfer.peer_device_name || transfer.peer_device_id}
            </div>
          </div>
          <button class="btn-cancel" on:click={() => handleCancel(transfer.transfer_id)}>
            Cancel
          </button>
        </div>

        <div class="progress-track">
          <div
            class="progress-fill"
            style="width: {(transfer.chunks_completed / transfer.chunks_total) * 100}%"
          ></div>
        </div>

        <div class="progress-meta mono">
          <span>{Math.round((transfer.chunks_completed / transfer.chunks_total) * 100)}%</span>
          <span>{transfer.chunks_completed} / {transfer.chunks_total} chunks</span>
        </div>
      </div>
    {/each}
  {/if}
</div>

<style>
  .queue {
    animation: float-in 0.3s ease-out;
  }

  .transfer-card {
    padding: 14px;
    border-radius: var(--radius-md);
    margin-bottom: 8px;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border-subtle);
    animation: float-in 0.3s ease-out both;
  }

  .transfer-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
  }

  .file-name {
    font-size: 14px;
    font-weight: 500;
  }

  .file-meta {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .btn-cancel {
    background: var(--danger-dim);
    border: 1px solid var(--danger-border);
    color: var(--danger);
    border-radius: 8px;
    padding: 4px 10px;
    font-size: 11px;
    cursor: pointer;
    font-family: var(--font-sans);
  }

  .progress-track {
    height: 4px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.06);
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: linear-gradient(90deg, #1a9e78, var(--accent));
    border-radius: 2px;
    transition: width 0.5s ease;
    background-size: 200% 100%;
    animation: shimmer 2s linear infinite;
  }

  .progress-meta {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--text-tertiary);
    margin-top: 6px;
  }

  .empty {
    text-align: center;
    padding: 40px 20px;
  }

  .empty-text {
    font-size: 14px;
    color: var(--text-secondary);
    margin-bottom: 4px;
  }

  .empty-hint {
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
