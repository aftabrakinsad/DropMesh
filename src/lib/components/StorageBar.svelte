<script>
  import { storageStats } from "../stores/settings.js";
  import { formatBytes } from "../stores/transfers.js";

  $: usagePercent =
    $storageStats.quota_bytes > 0
      ? ($storageStats.used_bytes / $storageStats.quota_bytes) * 100
      : 0;
</script>

<footer>
  <div class="storage-info">
    <div class="storage-header">
      <span>Storage</span>
      <span class="mono">
        {formatBytes($storageStats.used_bytes)} / {formatBytes($storageStats.quota_bytes)}
      </span>
    </div>
    <div class="storage-track">
      <div
        class="storage-fill"
        class:warning={usagePercent > 80}
        class:critical={usagePercent > 95}
        style="width: {usagePercent}%"
      ></div>
    </div>
  </div>
</footer>

<style>
  footer {
    padding: 12px 20px 16px;
    border-top: 1px solid var(--border-subtle);
  }

  .storage-header {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--text-tertiary);
    margin-bottom: 4px;
  }

  .storage-track {
    height: 3px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.06);
  }

  .storage-fill {
    height: 100%;
    border-radius: 2px;
    background: linear-gradient(90deg, #1a9e78, var(--accent));
    transition: width 0.5s ease;
  }

  .storage-fill.warning {
    background: linear-gradient(90deg, #e0a030, #ebb040);
  }

  .storage-fill.critical {
    background: linear-gradient(90deg, #d04040, var(--danger));
  }
</style>
