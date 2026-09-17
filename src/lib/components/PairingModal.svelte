<script>
  import { createEventDispatcher, onMount } from "svelte";
  import { startPairing } from "../api/tauri.js";
  import { loadPairedDevices } from "../stores/devices.js";

  const dispatch = createEventDispatcher();

  let pairingData = null;
  let error = null;
  let timeLeft = 300; // 5 minutes in seconds
  let timer;

  onMount(async () => {
    try {
      pairingData = await startPairing();

      // Countdown timer
      timer = setInterval(() => {
        timeLeft--;
        if (timeLeft <= 0) {
          clearInterval(timer);
          dispatch("close");
        }
      }, 1000);
    } catch (e) {
      error = e;
    }

    return () => {
      if (timer) clearInterval(timer);
    };
  });

  function formatTime(seconds) {
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  function formatCode(code) {
    if (!code) return "...";
    return `${code.slice(0, 3)} ${code.slice(3)}`;
  }
</script>

<div class="overlay" on:click|self={() => dispatch("close")} role="dialog">
  <div class="modal">
    <h2>Pair a new device</h2>
    <p class="subtitle">
      Scan this QR code from the other device, or enter the code manually.
    </p>

    {#if error}
      <div class="error">{error}</div>
    {:else if pairingData}
      <!-- QR Code placeholder — in production, generate actual QR from pairingData -->
      <div class="qr-container">
        <div class="qr-placeholder">
          {#each Array(121) as _, i}
            <div
              class="qr-cell"
              style="background: {Math.random() > 0.45 ? '#111318' : '#fff'}"
            ></div>
          {/each}
        </div>
      </div>

      <!-- Pairing code -->
      <div class="pairing-code mono">
        {formatCode(pairingData.pairing_code)}
      </div>

      <div class="expires">
        Expires in {formatTime(timeLeft)}
      </div>
    {:else}
      <div class="loading">Generating pairing code...</div>
    {/if}

    <button class="btn-close" on:click={() => dispatch("close")}>
      Cancel
    </button>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    backdrop-filter: blur(8px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }

  .modal {
    background: var(--bg-secondary);
    border-radius: var(--radius-lg);
    padding: 28px;
    width: 85%;
    max-width: 360px;
    border: 1px solid var(--border);
    animation: float-in 0.25s ease-out;
  }

  h2 {
    font-size: 16px;
    font-weight: 600;
    margin-bottom: 4px;
  }

  .subtitle {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-bottom: 20px;
  }

  .qr-container {
    display: flex;
    justify-content: center;
    margin-bottom: 16px;
  }

  .qr-placeholder {
    width: 160px;
    height: 160px;
    background: #fff;
    border-radius: 12px;
    padding: 12px;
    display: grid;
    grid-template-columns: repeat(11, 1fr);
    grid-template-rows: repeat(11, 1fr);
    gap: 1px;
  }

  .qr-cell {
    border-radius: 1px;
  }

  .pairing-code {
    text-align: center;
    font-size: 22px;
    font-weight: 600;
    letter-spacing: 6px;
    color: var(--accent);
    margin-bottom: 20px;
  }

  .expires {
    text-align: center;
    font-size: 11px;
    color: var(--text-muted);
    margin-bottom: 16px;
  }

  .loading {
    text-align: center;
    padding: 40px 0;
    color: var(--text-secondary);
    font-size: 13px;
  }

  .error {
    text-align: center;
    padding: 20px;
    color: var(--danger);
    font-size: 13px;
  }

  .btn-close {
    width: 100%;
    padding: 12px;
    border: 1px solid var(--border);
    background: var(--bg-hover);
    color: var(--text-secondary);
    border-radius: 10px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    font-family: var(--font-sans);
    transition: all 0.2s;
  }

  .btn-close:hover {
    background: rgba(255, 255, 255, 0.08);
  }
</style>
