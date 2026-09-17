<script>
  import { createEventDispatcher } from "svelte";
  export let device;
  export let selected = false;
  export let offline = false;
  export let delay = 0;

  const dispatch = createEventDispatcher();

  const typeIcons = {
    laptop: "💻",
    phone: "📱",
    tablet: "📟",
    desktop: "🖥️",
  };
</script>

<button
  class="card"
  class:selected
  class:offline
  style="animation-delay: {delay}ms"
  on:click={() => dispatch("click")}
>
  <div class="device-icon">
    <span>{typeIcons[device.device_type] || "💻"}</span>
    {#if !offline}
      <span class="pulse"></span>
    {/if}
  </div>

  <div class="info">
    <div class="name">{device.name}</div>
    <div class="platform mono">
      {device.device_type}
    </div>
  </div>

  <div class="status" class:online={!offline}>
    {#if offline}
      {device.last_seen_at || "Unknown"}
    {:else}
      Ready
    {/if}
  </div>
</button>

<style>
  .card {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 12px 14px;
    border-radius: var(--radius-md);
    margin-bottom: 6px;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border-subtle);
    cursor: pointer;
    transition: all 0.2s;
    animation: float-in 0.3s ease-out both;
    font-family: var(--font-sans);
    color: var(--text-primary);
    text-align: left;
  }

  .card:hover:not(.offline) {
    background: var(--bg-hover);
  }

  .card.selected {
    background: rgba(34, 201, 151, 0.08);
    border-color: var(--accent-border);
  }

  .card.offline {
    opacity: 0.5;
    cursor: default;
  }

  .device-icon {
    position: relative;
    font-size: 18px;
    width: 28px;
    text-align: center;
  }

  .pulse {
    position: absolute;
    top: -2px;
    right: -4px;
    width: 8px;
    height: 8px;
    background: var(--accent);
    border-radius: 50%;
    border: 2px solid var(--bg-primary);
  }

  .info {
    flex: 1;
  }

  .name {
    font-size: 14px;
    font-weight: 500;
  }

  .platform {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .status {
    font-size: 11px;
    color: var(--text-muted);
  }

  .status.online {
    color: var(--accent);
    background: var(--accent-dim);
    padding: 3px 8px;
    border-radius: 6px;
  }
</style>
