<script>
  import { onlineDevices, offlineDevices, selectedDeviceId } from "../stores/devices.js";
  import DeviceCard from "./DeviceCard.svelte";
</script>

<div class="radar">
  {#if $onlineDevices.length > 0}
    <div class="section-label">Online</div>
    {#each $onlineDevices as device, i}
      <DeviceCard
        {device}
        selected={$selectedDeviceId === device.device_id}
        delay={i * 60}
        on:click={() =>
          selectedDeviceId.set(
            $selectedDeviceId === device.device_id ? null : device.device_id
          )
        }
      />
    {/each}
  {/if}

  {#if $offlineDevices.length > 0}
    <div class="section-label" style="margin-top: 16px;">Offline</div>
    {#each $offlineDevices as device}
      <DeviceCard {device} offline />
    {/each}
  {/if}

  {#if $onlineDevices.length === 0 && $offlineDevices.length === 0}
    <div class="empty">
      <div class="empty-icon">📡</div>
      <div class="empty-text">No paired devices yet</div>
      <div class="empty-hint">Tap "+ Add device" to pair your first device</div>
    </div>
  {/if}
</div>

<style>
  .radar {
    animation: float-in 0.3s ease-out;
  }

  .section-label {
    font-size: 11px;
    color: var(--text-tertiary);
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 1px;
    margin-bottom: 8px;
  }

  .empty {
    text-align: center;
    padding: 40px 20px;
  }

  .empty-icon {
    font-size: 32px;
    margin-bottom: 12px;
    opacity: 0.5;
  }

  .empty-text {
    font-size: 14px;
    font-weight: 500;
    color: var(--text-secondary);
    margin-bottom: 4px;
  }

  .empty-hint {
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
