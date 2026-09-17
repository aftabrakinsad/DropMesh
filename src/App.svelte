<script>
  import { onMount } from "svelte";
  import { writable, derived } from "svelte/store";

  // --- Stores ---
  let selfDevice = writable(null);
  let pairedDevices = writable([]);
  let discoveredPeers = writable([]);
  let queueEntries = writable([]);
  let queueStats = writable({ pending_count: 0, delivering_count: 0, delivered_today: 0, failed_count: 0, pending_size_bytes: 0 });
  let storageStats = writable({ folder_path: "", quota_bytes: 2147483648, used_bytes: 0 });
  let activeTab = writable("devices");
  let selectedDeviceId = writable(null);

  let onlineDevices = derived(pairedDevices, $d => $d.filter(d => d.is_online));
  let offlineDevices = derived(pairedDevices, $d => $d.filter(d => !d.is_online));
  let activeCount = derived(queueStats, $s => ($s.pending_count || 0) + ($s.delivering_count || 0));

  // --- Pairing state ---
  let showPairing = false;
  // Steps:
  //   "initiator" — Device A: shows its code + emojis, waits for Device B to confirm
  //   "qr"        — Device B: enters Device A's code
  //   "verify"    — Device B: sees emojis after entering code
  //   "done"      — both: success screen
  let pairingStep = "qr";
  let pairingData = null;
  let verificationEmojis = "";
  let pairingTarget = null;
  let manualCode = "";
  let pairingError = "";
  let pairingBusy = false;
  let pairedPeerName = "";
  let expiryTimer = null;
  let timeLeft = 300;

  // --- UI state ---
  let dragOver = false;
  let pickingFiles = false;
  let notification = null;
  let invoke = null;

  const typeIcons = { laptop: "💻", phone: "📱", tablet: "📟", desktop: "🖥️" };

  function formatBytes(bytes) {
    if (!bytes || bytes === 0) return "0 B";
    const units = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
  }

  function timeAgo(dateStr) {
    if (!dateStr) return "";
    const diff = Date.now() - new Date(dateStr).getTime();
    const mins = Math.floor(diff / 60000);
    if (mins < 1) return "just now";
    if (mins < 60) return `${mins}m ago`;
    return `${Math.floor(mins / 60)}h ago`;
  }

  function formatCountdown(secs) {
    const m = Math.floor(secs / 60);
    const s = secs % 60;
    return `${m}:${String(s).padStart(2, "0")}`;
  }

  function showNotification(msg, type = "success") {
    notification = { msg, type };
    setTimeout(() => notification = null, 3500);
  }

  // ── Data loading ──────────────────────────────────────────────────────────

  async function loadData() {
    if (!invoke) return;
    try {
      const [device, devices, queue, stats, storage] = await Promise.all([
        invoke("get_self_device").catch(() => null),
        invoke("get_paired_devices").catch(() => []),
        invoke("get_queue").catch(() => []),
        invoke("get_queue_stats").catch(() => ({})),
        invoke("get_storage_stats").catch(() => ({})),
      ]);
      if (device) selfDevice.set(device);
      if (devices) {
        pairedDevices.set(devices);
        const pairedIds = new Set(devices.map(d => d.device_id));
        discoveredPeers.update(peers => peers.filter(p => !pairedIds.has(p.device_id)));
      }
      if (queue) queueEntries.set(queue);
      if (stats) queueStats.set(stats);
      if (storage) storageStats.set(storage);
    } catch (e) {
      console.error("loadData error:", e);
    }
  }

  // ── Discovery event handlers ──────────────────────────────────────────────

  function handleDeviceDiscovered(peer) {
    const pairedIds = $pairedDevices.map(d => d.device_id);
    if (peer.is_paired || pairedIds.includes(peer.device_id)) {
      pairedDevices.update(devices => {
        const idx = devices.findIndex(d => d.device_id === peer.device_id);
        if (idx >= 0) {
          devices[idx] = { ...devices[idx], is_online: true };
          return [...devices];
        }
        return [...devices, { ...peer, is_online: true }];
      });
      showNotification(`${peer.name} is online`);
      deliverQueuedFiles(peer.device_id);
    } else {
      discoveredPeers.update(peers => {
        const idx = peers.findIndex(p => p.device_id === peer.device_id);
        if (idx >= 0) { peers[idx] = peer; return [...peers]; }
        return [...peers, peer];
      });
    }
    // A device can be discovered before it is paired. Once it appears in the
    // paired list, drop it from Nearby so it isn't offered for pairing twice.
    discoveredPeers.update(peers =>
      peers.filter(p => !$pairedDevices.some(d => d.device_id === p.device_id))
    );
  }

  function handleDeviceLost(payload) {
    const instance = payload.instance;
    pairedDevices.update(devices =>
      devices.map(d => d.device_id.startsWith(instance) ? { ...d, is_online: false } : d)
    );
    discoveredPeers.update(peers =>
      peers.filter(p => !p.device_id.startsWith(instance))
    );
  }

  async function deliverQueuedFiles(deviceId) {
    if (!invoke) return;
    setTimeout(async () => {
      const queue = await invoke("get_queue").catch(() => null);
      if (queue) queueEntries.set(queue);
      const stats = await invoke("get_queue_stats").catch(() => null);
      if (stats) queueStats.set(stats);
    }, 1500);
  }

  // ── File operations ───────────────────────────────────────────────────────

  async function openFilePicker() {
    if (!invoke || pickingFiles) return;
    if ($pairedDevices.length === 0) {
      showNotification("No paired devices. Pair a device first.", "warning");
      return;
    }
    const targetId = $selectedDeviceId || $pairedDevices[0]?.device_id;
    if (!targetId) return;
    pickingFiles = true;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ multiple: true, title: "Select files to send" });
      if (!selected) { pickingFiles = false; return; }
      const files = Array.isArray(selected) ? selected : [selected];
      let queued = 0;
      for (const filePath of files) {
        try { await invoke("queue_file", { filePath, targetDeviceId: targetId }); queued++; }
        catch (e) { console.error("Queue failed:", filePath, e); }
      }
      if (queued > 0) {
        showNotification(`${queued} file${queued > 1 ? "s" : ""} queued`);
        activeTab.set("queue");
        await loadData();
      }
    } catch (e) {
      showNotification("Could not open file picker", "error");
    }
    pickingFiles = false;
  }

  async function handleDrop(e) {
    e.preventDefault();
    dragOver = false;
    const files = e.dataTransfer?.files;
    if (!files?.length || !invoke) return;
    const targetId = $selectedDeviceId || $pairedDevices[0]?.device_id;
    if (!targetId) { showNotification("Select a device first", "warning"); return; }
    let queued = 0;
    for (const file of files) {
      if (!file.path) continue;
      try { await invoke("queue_file", { filePath: file.path, targetDeviceId: targetId }); queued++; }
      catch (e) { console.error("Queue failed:", e); }
    }
    if (queued > 0) {
      showNotification(`${queued} file${queued > 1 ? "s" : ""} queued`);
      activeTab.set("queue");
      await loadData();
    }
  }

  // ── Pairing flow ──────────────────────────────────────────────────────────
  //
  // DEVICE A ("+ Add device"): generates a PIN and waits. Its QUIC listener
  // answers the incoming PairRequest automatically and fires
  // `on_pairing_complete`, which advances this device to the verify screen.
  //
  // DEVICE B ("Enter code"): picks Device A from the Nearby list (mDNS gives
  // us its IP and port), types the PIN, and calls `pair_with_device`, which
  // opens a QUIC connection and exchanges real identities.

  async function openPairing(targetPeer = null) {
    pairingError = "";
    pairingTarget = targetPeer;
    verificationEmojis = "";
    manualCode = "";

    try {
      pairingData = await invoke("start_pairing");
      timeLeft = 300;
      if (expiryTimer) clearInterval(expiryTimer);
      expiryTimer = setInterval(() => {
        timeLeft--;
        if (timeLeft <= 0) closePairing();
      }, 1000);

      verificationEmojis = pairingData.verification_emojis || "";
      pairingStep = "initiator";
      showPairing = true;
    } catch (e) {
      showNotification("Failed to start pairing: " + e, "error");
    }
  }

  function openResponder(peer = null) {
    pairingError = "";
    pairingTarget = peer ?? ($discoveredPeers.length === 1 ? $discoveredPeers[0] : null);
    verificationEmojis = "";
    manualCode = "";
    pairingStep = "responder";
    showPairing = true;
    timeLeft = 300;
    if (expiryTimer) clearInterval(expiryTimer);
    expiryTimer = setInterval(() => {
      timeLeft--;
      if (timeLeft <= 0) closePairing();
    }, 1000);
  }

  async function submitPairingCode() {
    const code = manualCode.replace(/\s/g, "");
    if (code.length !== 6) {
      pairingError = "Enter the 6-digit code shown on the other device.";
      return;
    }
    if (!pairingTarget) {
      pairingError = "Select which device to pair with.";
      return;
    }

    pairingError = "";
    pairingBusy = true;

    try {
      const result = await invoke("pair_with_device", {
        peerHost: pairingTarget.host,
        peerPort: pairingTarget.port,
        pin: code,
      });

      verificationEmojis = result.verification_emojis;
      pairedPeerName = result.peer_name;
      pairingStep = "verify";
      await loadData();
    } catch (e) {
      pairingError = typeof e === "string" ? e : "Pairing failed. Check the code and try again.";
    }
    pairingBusy = false;
  }

  function confirmVerification() {
    pairingStep = "done";
    if (expiryTimer) clearInterval(expiryTimer);
    showNotification(`Paired with ${pairedPeerName || "device"}`);
    setTimeout(() => {
      showPairing = false;
      pairingStep = "responder";
      loadData();
    }, 1800);
  }

  function closePairing() {
    showPairing = false;
    pairingStep = "responder";
    pairingData = null;
    pairingError = "";
    pairingBusy = false;
    if (expiryTimer) clearInterval(expiryTimer);
  }

  async function clearAllDevices() {
    if (!invoke) return;
    try {
      const n = await invoke("clear_all_devices");
      showNotification(`Removed ${n} device(s)`);
      await loadData();
    } catch (e) {
      showNotification("Could not clear devices", "error");
    }
  }

  // ── Queue operations ──────────────────────────────────────────────────────

  async function cancelQueued(queueId) {
    if (!invoke) return;
    try {
      await invoke("cancel_queued_file", { queueId });
      showNotification("Cancelled");
      await loadData();
    } catch (e) { showNotification("Cancel failed", "error"); }
  }

  async function retryQueued(queueId) {
    if (!invoke) return;
    try {
      await invoke("retry_queued_file", { queueId });
      showNotification("Queued for retry");
      await loadData();
    } catch (e) { showNotification("Retry failed", "error"); }
  }

  // ── Init ──────────────────────────────────────────────────────────────────

  onMount(async () => {
    invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) {
      try {
        const api = await import("@tauri-apps/api/core");
        invoke = api.invoke;
      } catch (e) { console.error("Tauri API unavailable:", e); }
    }
    await loadData();
    setInterval(loadData, 5000);
    if (window.__TAURI__?.event?.listen) {
      window.__TAURI__.event.listen("on_device_discovered", e => handleDeviceDiscovered(e.payload));
      window.__TAURI__.event.listen("on_device_lost", e => handleDeviceLost(e.payload));
      window.__TAURI__.event.listen("queue_stats_update", e => queueStats.set(e.payload));
      window.__TAURI__.event.listen("on_pairing_complete", e => {
        // Device A: a peer completed the handshake with our PIN
        verificationEmojis = e.payload.verification_emojis;
        pairedPeerName = e.payload.peer_name;
        pairingStep = "verify";
        showPairing = true;
        loadData();
      });
      window.__TAURI__.event.listen("on_incoming_file", e => {
        showNotification(`Received ${e.payload.file_name}`);
        loadData();
      });
    }
  });

  const tabs = [
    { id: "devices", label: "Devices" },
    { id: "queue", label: "Queue" },
    { id: "history", label: "History" },
  ];
</script>

<main>
  <!-- Header -->
  <header>
    <div class="header-left">
      <div class="logo">D</div>
      <div>
        <h1>DropMesh</h1>
        <span class="mono sub">{$selfDevice?.name ?? "Starting..."}</span>
      </div>
    </div>
    <!-- Two pairing entry points clearly labelled -->
    <div class="header-btns">
      <button class="btn-secondary" on:click={openResponder} title="Enter code from another device">Enter code</button>
      <button class="btn-add" on:click={() => openPairing()}>+ Add device</button>
    </div>
  </header>

  <!-- Notification -->
  {#if notification}
    <div class="notif" class:warn={notification.type === "warning"} class:err={notification.type === "error"}>
      {notification.msg}
    </div>
  {/if}

  <!-- Drop zone -->
  <div
    class="dropzone" class:active={dragOver} class:picking={pickingFiles}
    on:dragover|preventDefault={() => dragOver = true}
    on:dragleave={() => dragOver = false}
    on:drop={handleDrop}
    on:click={openFilePicker}
    role="button" tabindex="0"
    on:keydown={e => e.key === "Enter" && openFilePicker()}
  >
    <div class="dz-icon">{dragOver ? "↓" : pickingFiles ? "…" : "⊕"}</div>
    <div class="dz-label">
      {#if $pairedDevices.length === 0}Pair a device first, then drop files here
      {:else if dragOver}Release to queue files
      {:else if $selectedDeviceId}Drop or click → {$pairedDevices.find(d => d.device_id === $selectedDeviceId)?.name}
      {:else}Drop files or click to browse → {$pairedDevices[0]?.name}
      {/if}
    </div>
    <div class="dz-hint">Files queue and deliver automatically · Max 2 GB</div>
  </div>

  <!-- Tabs -->
  <nav class="tabs">
    {#each tabs as tab}
      <button class="tab" class:active={$activeTab === tab.id} on:click={() => activeTab.set(tab.id)}>
        {tab.label}
        {#if tab.id === "queue" && $activeCount > 0}
          <span class="badge">{$activeCount}</span>
        {/if}
        {#if $activeTab === tab.id}<div class="tab-line"></div>{/if}
      </button>
    {/each}
  </nav>

  <!-- Content -->
  <section class="content">

    {#if $activeTab === "devices"}
      {#if $onlineDevices.length > 0}
        <div class="section-label">Online · tap to select</div>
        {#each $onlineDevices as d}
          <button
            class="device-card" class:selected={$selectedDeviceId === d.device_id}
            on:click={() => selectedDeviceId.set($selectedDeviceId === d.device_id ? null : d.device_id)}
          >
            <div class="device-icon-wrap">
              <span>{typeIcons[d.device_type] ?? "💻"}</span>
              <span class="dot-online"></span>
            </div>
            <div class="device-info">
              <div class="dname">{d.name}</div>
              <div class="dsub mono">{d.device_type}</div>
            </div>
            {#if $selectedDeviceId === d.device_id}
              <div class="badge-selected">Selected</div>
            {:else}
              <div class="badge-online">Ready</div>
            {/if}
          </button>
        {/each}
      {/if}

      {#if $offlineDevices.length > 0}
        <div class="section-label" style="margin-top:14px">Offline · files will queue</div>
        {#each $offlineDevices as d}
          <button
            class="device-card offline" class:selected={$selectedDeviceId === d.device_id}
            on:click={() => selectedDeviceId.set($selectedDeviceId === d.device_id ? null : d.device_id)}
          >
            <span style="font-size:18px">{typeIcons[d.device_type] ?? "💻"}</span>
            <div class="device-info">
              <div class="dname">{d.name}</div>
              <div class="dsub mono">{d.device_type} · {d.last_seen_at ? timeAgo(d.last_seen_at) : "never seen"}</div>
            </div>
            {#if $selectedDeviceId === d.device_id}
              <div class="badge-selected">Selected</div>
            {:else}
              <div class="badge-offline">Offline</div>
            {/if}
          </button>
        {/each}
      {/if}

      {#if $discoveredPeers.length > 0}
        <div class="section-label" style="margin-top:14px">Nearby · tap to pair</div>
        {#each $discoveredPeers as peer}
          <button class="device-card nearby" on:click={() => openResponder(peer)}>
            <span style="font-size:18px">{typeIcons[peer.device_type] ?? "💻"}</span>
            <div class="device-info">
              <div class="dname">{peer.name}</div>
              <div class="dsub mono">{peer.device_type} · {peer.host}</div>
            </div>
            <div class="badge-pair">Enter code</div>
          </button>
        {/each}
      {/if}

      {#if $onlineDevices.length === 0 && $offlineDevices.length === 0 && $discoveredPeers.length === 0}
        <div class="empty">
          <div class="eicon">📡</div>
          <div class="etitle">No devices yet</div>
          <div class="esub">
            Whichever device lists the other under <strong>Nearby</strong> is the one
            that types the code.<br/>
            On the other device, tap <strong>"+ Add device"</strong> to display it.<br/>
            Both devices must be on the same Wi-Fi.
          </div>
        </div>
      {/if}

    {:else if $activeTab === "queue"}
      {#if $queueStats.pending_count > 0 || $queueStats.delivering_count > 0 || $queueStats.failed_count > 0}
        <div class="stats-row">
          <div class="stat"><div class="sv">{$queueStats.pending_count ?? 0}</div><div class="sl">Pending</div></div>
          <div class="stat"><div class="sv delivering">{$queueStats.delivering_count ?? 0}</div><div class="sl">Sending</div></div>
          <div class="stat"><div class="sv success">{$queueStats.delivered_today ?? 0}</div><div class="sl">Today</div></div>
          <div class="stat"><div class="sv failed">{$queueStats.failed_count ?? 0}</div><div class="sl">Failed</div></div>
        </div>
      {/if}

      {#if $queueEntries.filter(e => ["pending","delivering","failed"].includes(e.status)).length === 0}
        <div class="empty">
          <div class="eicon">📭</div>
          <div class="etitle">Queue is empty</div>
          <div class="esub">Drop a file onto the drop zone above.<br/>Files deliver automatically when the target device comes online.</div>
        </div>
      {:else}
        {#each $queueEntries.filter(e => ["pending","delivering","failed"].includes(e.status)) as entry}
          <div class="queue-card" class:sending={entry.status === "delivering"}>
            <div class="qrow">
              <div class="qfile">
                <div class="qname truncate">{entry.file_name}</div>
                <div class="qmeta mono">{formatBytes(entry.file_size)} · to {entry.target_device_name ?? entry.target_device_id.slice(0,8)}</div>
              </div>
              <div class="qstatus s-{entry.status}">{entry.status}</div>
            </div>
            {#if entry.status === "delivering"}
              <div class="progress"><div class="progress-fill shimmer"></div></div>
            {/if}
            <div class="qfooter">
              <span class="mono qtime">{timeAgo(entry.created_at)}</span>
              <div style="display:flex;gap:6px">
                {#if entry.status === "failed"}
                  <button class="btn-retry" on:click={() => retryQueued(entry.queue_id)}>Retry</button>
                {/if}
                {#if entry.status !== "delivering"}
                  <button class="btn-cancel" on:click={() => cancelQueued(entry.queue_id)}>Cancel</button>
                {/if}
              </div>
            </div>
            {#if entry.error_message}
              <div class="qerror">{entry.error_message}</div>
            {/if}
          </div>
        {/each}
      {/if}

    {:else if $activeTab === "history"}
      {#if $queueEntries.filter(e => ["delivered","cancelled","expired"].includes(e.status)).length === 0}
        <div class="empty">
          <div class="eicon">📋</div>
          <div class="etitle">No history yet</div>
          <div class="esub">Completed and cancelled transfers appear here.</div>
        </div>
      {:else}
        {#each $queueEntries.filter(e => ["delivered","cancelled","expired"].includes(e.status)) as entry}
          <div class="hist-item">
            <div class="hist-icon s-{entry.status}">
              {entry.status === "delivered" ? "✓" : "✕"}
            </div>
            <div class="device-info">
              <div class="qname truncate">{entry.file_name}</div>
              <div class="qmeta mono">{formatBytes(entry.file_size)} · {entry.target_device_name ?? entry.target_device_id.slice(0,8)}</div>
            </div>
            <div class="mono qtime">{timeAgo(entry.delivered_at ?? entry.created_at)}</div>
          </div>
        {/each}
      {/if}
    {/if}

  </section>

  <!-- Storage bar -->
  <footer>
    <div class="sfooter">
      <span>Storage</span>
      <span class="mono">{formatBytes($storageStats.used_bytes)} / {formatBytes($storageStats.quota_bytes)}</span>
    </div>
    <div class="strack">
      <div class="sfill" style="width:{$storageStats.quota_bytes > 0 ? Math.min(100, $storageStats.used_bytes / $storageStats.quota_bytes * 100) : 0}%"></div>
    </div>
  </footer>

  <!-- ── PAIRING MODAL ─────────────────────────────────────────────────────── -->
  {#if showPairing}
    <div class="overlay" on:click|self={closePairing} role="dialog" tabindex="-1" on:keydown={() => {}}>
      <div class="modal">

        <!-- ── INITIATOR STEP (Device A) ────────────────────────────────────
             Device A tapped "+ Add device". Shows its code AND emojis.
             Device B must enter this code, then both confirm matching emojis. -->
        {#if pairingStep === "initiator"}
          <h2>Add a device</h2>
          <p class="msub">
            On the other device, tap this device under <strong>Nearby</strong> and type
            the code below. When both show the same emojis, confirm pairing.
          </p>

          <div class="code-section">
            <div class="code-label">Your pairing code</div>
            <div class="mono big-code">
              {pairingData?.pairing_code?.replace(/(\d{3})(\d{3})/, "$1 $2") ?? "------"}
            </div>
            <div class="mono code-expire">Expires in {formatCountdown(timeLeft)}</div>
          </div>

          {#if verificationEmojis}
            <div class="emoji-verify">{verificationEmojis}</div>
            <div class="verify-note">
              When the other device shows these same emojis, tap Confirm.
            </div>
            <button class="btn-primary" on:click={confirmVerification}>
              ✓ They match — Confirm pairing
            </button>
          {/if}

          <button class="btn-close" on:click={closePairing}>Cancel</button>

        <!-- ── QR STEP (Device B) ────────────────────────────────────────────
             Device B tapped "Enter code". Enters Device A's 6-digit code. -->
        {:else if pairingStep === "responder"}
          <h2>Enter pairing code</h2>
          <p class="msub">
            Pick the device showing the code, then type the 6 digits.
          </p>

          {#if $discoveredPeers.length === 0}
            <div class="picker-empty">
              No other DropMesh devices found on this network yet.<br/>
              Make sure the other device is running and on the same Wi-Fi.
            </div>
          {:else}
            <div class="picker-label">Pair with</div>
            {#each $discoveredPeers as peer}
              <button
                class="picker-item"
                class:picked={pairingTarget?.device_id === peer.device_id}
                on:click={() => pairingTarget = peer}
              >
                <span style="font-size:16px">{typeIcons[peer.device_type] ?? "\u{1F4BB}"}</span>
                <div class="device-info">
                  <div class="dname">{peer.name}</div>
                  <div class="dsub mono">{peer.host}:{peer.port}</div>
                </div>
                {#if pairingTarget?.device_id === peer.device_id}
                  <span class="picker-tick">✓</span>
                {/if}
              </button>
            {/each}
          {/if}

          <input
            class="code-input mono"
            type="text"
            maxlength="7"
            placeholder="000 000"
            bind:value={manualCode}
            on:keydown={e => e.key === "Enter" && !pairingBusy && submitPairingCode()}
          />

          {#if pairingError}
            <div class="perror">{pairingError}</div>
          {/if}

          <button class="btn-primary" on:click={submitPairingCode} disabled={pairingBusy}>
            {pairingBusy ? "Connecting…" : "Connect"}
          </button>
          <button class="btn-close" on:click={closePairing}>Cancel</button>

        {:else if pairingStep === "verify"}
          <h2>Verify pairing</h2>
          <p class="msub">
            {pairedPeerName ? `Pairing with ${pairedPeerName}. ` : ""}Check that the other device shows these same 4 emojis.
          </p>

          <div class="emoji-verify">{verificationEmojis}</div>
          <div class="verify-note">Both devices must show the same emojis</div>

          <button class="btn-primary" on:click={confirmVerification}>
            ✓ They match — Confirm pairing
          </button>
          <button class="btn-close" on:click={closePairing}>They don't match — Cancel</button>

        <!-- ── DONE ──────────────────────────────────────────────────────── -->
        {:else if pairingStep === "done"}
          <div class="done-state">
            <div class="done-icon">✓</div>
            <div class="done-text">Paired!</div>
          </div>
        {/if}

      </div>
    </div>
  {/if}
</main>

<style>
  main { max-width: 440px; margin: 0 auto; height: 100vh; display: flex; flex-direction: column; background: var(--bg-primary); border: 1px solid var(--border); font-family: var(--font-sans); color: var(--text-primary); }

  header { padding: 20px 20px 0; display: flex; align-items: center; justify-content: space-between; }
  .header-left { display: flex; align-items: center; gap: 10px; }
  .logo { width: 32px; height: 32px; border-radius: 10px; background: linear-gradient(135deg, #22c997, #1a9e78); display: flex; align-items: center; justify-content: center; font-size: 15px; font-weight: 600; color: #fff; }
  h1 { font-size: 16px; font-weight: 600; letter-spacing: -0.3px; margin: 0; }
  .sub { font-size: 11px; color: var(--text-tertiary); }
  .header-btns { display: flex; gap: 6px; align-items: center; }
  .btn-add { background: var(--accent-dim); border: 1px solid var(--accent-border); color: var(--accent); border-radius: 10px; padding: 7px 12px; font-size: 12px; font-weight: 500; cursor: pointer; font-family: var(--font-sans); }
  .btn-secondary { background: rgba(255,255,255,0.04); border: 1px solid var(--border); color: var(--text-secondary); border-radius: 10px; padding: 7px 12px; font-size: 12px; font-weight: 500; cursor: pointer; font-family: var(--font-sans); }

  .notif { margin: 10px 20px 0; padding: 10px 14px; border-radius: 10px; background: rgba(34,201,151,0.1); border: 1px solid var(--accent-border); color: var(--accent); font-size: 13px; font-weight: 500; animation: float-in 0.2s ease-out; }
  .notif.warn { background: rgba(255,180,0,0.1); border-color: rgba(255,180,0,0.25); color: #ffb400; }
  .notif.err { background: var(--danger-dim); border-color: var(--danger-border); color: var(--danger); }

  .dropzone { margin: 14px 20px 10px; border: 2px dashed var(--border); border-radius: 16px; padding: 24px 20px; text-align: center; cursor: pointer; transition: all 0.3s; background: rgba(255,255,255,0.02); }
  .dropzone:hover { border-color: rgba(34,201,151,0.3); background: rgba(34,201,151,0.03); }
  .dropzone.active { border-color: var(--accent); background: rgba(34,201,151,0.06); }
  .dropzone.picking { border-color: var(--accent); opacity: 0.7; cursor: wait; }
  .dz-icon { font-size: 26px; margin-bottom: 6px; opacity: 0.45; }
  .dropzone.active .dz-icon { opacity: 1; }
  .dz-label { font-size: 13px; font-weight: 500; color: var(--text-secondary); }
  .dropzone.active .dz-label { color: var(--accent); }
  .dz-hint { font-size: 11px; color: var(--text-muted); margin-top: 4px; }

  .tabs { display: flex; padding: 0 20px; gap: 4px; }
  .tab { flex: 1; padding: 10px 0; border: none; background: none; color: var(--text-tertiary); font-size: 13px; font-weight: 500; cursor: pointer; font-family: var(--font-sans); position: relative; transition: color 0.2s; display: flex; align-items: center; justify-content: center; gap: 6px; }
  .tab.active { color: var(--text-primary); }
  .tab-line { position: absolute; bottom: 0; left: 20%; right: 20%; height: 2px; background: var(--accent); border-radius: 1px; }
  .badge { background: var(--accent); color: #111318; font-size: 10px; font-weight: 600; border-radius: 6px; padding: 1px 6px; }

  .content { flex: 1; padding: 8px 20px; overflow-y: auto; min-height: 0; }
  .section-label { font-size: 11px; color: var(--text-tertiary); font-weight: 500; text-transform: uppercase; letter-spacing: 1px; margin-bottom: 8px; }

  .device-card { display: flex; align-items: center; gap: 12px; padding: 12px 14px; border-radius: 12px; margin-bottom: 6px; background: rgba(255,255,255,0.03); border: 1px solid var(--border-subtle); cursor: pointer; transition: all 0.2s; width: 100%; font-family: var(--font-sans); color: var(--text-primary); text-align: left; }
  .device-card:hover:not(.offline) { background: var(--bg-hover); }
  .device-card.selected { background: rgba(34,201,151,0.08); border-color: var(--accent-border); }
  .device-card.offline { opacity: 0.5; }
  .device-card.nearby { border-color: rgba(100,140,255,0.2); }
  .device-icon-wrap { position: relative; font-size: 18px; }
  .dot-online { position: absolute; top: -2px; right: -4px; width: 8px; height: 8px; background: var(--accent); border-radius: 50%; border: 2px solid var(--bg-primary); }
  .device-info { flex: 1; min-width: 0; }
  .dname { font-size: 14px; font-weight: 500; }
  .dsub { font-size: 11px; color: var(--text-tertiary); }
  .badge-online { font-size: 11px; padding: 3px 8px; border-radius: 6px; color: var(--accent); background: var(--accent-dim); }
  .badge-offline { font-size: 11px; padding: 3px 8px; border-radius: 6px; color: var(--text-muted); background: rgba(255,255,255,0.04); }
  .badge-selected { font-size: 11px; padding: 3px 8px; border-radius: 6px; color: #fff; background: var(--accent); }
  .badge-pair { font-size: 11px; padding: 3px 8px; border-radius: 6px; color: #648cff; background: rgba(100,140,255,0.1); border: 1px solid rgba(100,140,255,0.2); }

  .stats-row { display: flex; gap: 8px; margin-bottom: 12px; }
  .stat { flex: 1; background: rgba(255,255,255,0.03); border: 1px solid var(--border-subtle); border-radius: 10px; padding: 10px 8px; text-align: center; }
  .sv { font-size: 20px; font-weight: 600; }
  .sv.delivering { color: #ffb400; }
  .sv.success { color: var(--accent); }
  .sv.failed { color: var(--danger); }
  .sl { font-size: 10px; color: var(--text-muted); margin-top: 2px; text-transform: uppercase; letter-spacing: 0.5px; }

  .queue-card { padding: 12px 14px; border-radius: 12px; margin-bottom: 8px; background: rgba(255,255,255,0.03); border: 1px solid var(--border-subtle); }
  .queue-card.sending { border-color: rgba(255,180,0,0.2); }
  .qrow { display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; margin-bottom: 8px; }
  .qfile { flex: 1; min-width: 0; }
  .qname { font-size: 14px; font-weight: 500; }
  .qmeta { font-size: 11px; color: var(--text-tertiary); margin-top: 2px; }
  .qstatus { font-size: 10px; font-weight: 600; padding: 3px 8px; border-radius: 6px; text-transform: uppercase; letter-spacing: 0.5px; flex-shrink: 0; }
  .s-pending { background: rgba(255,255,255,0.06); color: var(--text-secondary); }
  .s-delivering { background: rgba(255,180,0,0.15); color: #ffb400; }
  .s-delivered { background: var(--accent-dim); color: var(--accent); }
  .s-failed { background: var(--danger-dim); color: var(--danger); }
  .s-cancelled, .s-expired { background: rgba(255,255,255,0.04); color: var(--text-muted); }
  .progress { height: 3px; border-radius: 2px; background: rgba(255,255,255,0.06); margin-bottom: 8px; overflow: hidden; }
  .progress-fill { height: 100%; width: 100%; background: linear-gradient(90deg, #1a9e78, var(--accent)); border-radius: 2px; }
  .shimmer { background-size: 200% 100%; animation: shimmer 1.5s linear infinite; }
  .qfooter { display: flex; align-items: center; justify-content: space-between; }
  .qtime { font-size: 11px; color: var(--text-muted); }
  .qerror { font-size: 11px; color: var(--danger); margin-top: 6px; padding: 6px 8px; background: var(--danger-dim); border-radius: 6px; }
  .btn-cancel { background: var(--danger-dim); border: 1px solid var(--danger-border); color: var(--danger); border-radius: 6px; padding: 3px 10px; font-size: 11px; cursor: pointer; font-family: var(--font-sans); }
  .btn-retry { background: var(--accent-dim); border: 1px solid var(--accent-border); color: var(--accent); border-radius: 6px; padding: 3px 10px; font-size: 11px; cursor: pointer; font-family: var(--font-sans); }

  .hist-item { display: flex; align-items: center; gap: 12px; padding: 12px 14px; border-radius: 12px; margin-bottom: 6px; background: rgba(255,255,255,0.03); border: 1px solid var(--border-subtle); }
  .hist-icon { width: 32px; height: 32px; border-radius: 8px; display: flex; align-items: center; justify-content: center; font-size: 14px; flex-shrink: 0; }
  .hist-icon.s-delivered { background: var(--accent-dim); color: var(--accent); }
  .hist-icon.s-cancelled, .hist-icon.s-expired { background: rgba(255,255,255,0.04); color: var(--text-muted); }

  .empty { text-align: center; padding: 40px 20px; }
  .eicon { font-size: 32px; margin-bottom: 12px; opacity: 0.5; }
  .etitle { font-size: 14px; font-weight: 500; color: var(--text-secondary); margin-bottom: 6px; }
  .esub { font-size: 12px; color: var(--text-muted); line-height: 1.7; }

  footer { padding: 12px 20px 16px; border-top: 1px solid var(--border-subtle); }
  .sfooter { display: flex; justify-content: space-between; font-size: 11px; color: var(--text-tertiary); margin-bottom: 4px; }
  .strack { height: 3px; border-radius: 2px; background: rgba(255,255,255,0.06); }
  .sfill { height: 100%; border-radius: 2px; background: linear-gradient(90deg, #1a9e78, var(--accent)); transition: width 0.5s; }

  .overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.75); backdrop-filter: blur(8px); display: flex; align-items: center; justify-content: center; z-index: 100; }
  .modal { background: var(--bg-secondary); border-radius: 18px; padding: 28px; width: 88%; max-width: 360px; border: 1px solid var(--border); animation: float-in 0.25s ease-out; }
  .modal h2 { font-size: 17px; font-weight: 600; margin-bottom: 6px; }
  .msub { font-size: 12px; color: var(--text-tertiary); margin-bottom: 20px; line-height: 1.5; }

  .code-section { background: rgba(34,201,151,0.06); border: 1px solid var(--accent-border); border-radius: 12px; padding: 16px; text-align: center; margin-bottom: 16px; }
  .code-label { font-size: 11px; color: var(--text-tertiary); text-transform: uppercase; letter-spacing: 1px; margin-bottom: 8px; }
  .big-code { font-size: 28px; font-weight: 700; letter-spacing: 8px; color: var(--accent); margin-bottom: 4px; }
  .code-expire { font-size: 11px; color: var(--text-muted); }

  .code-input { width: 100%; padding: 14px; background: rgba(255,255,255,0.04); border: 1px solid var(--border); border-radius: 10px; color: var(--text-primary); font-size: 22px; font-family: var(--font-mono); text-align: center; letter-spacing: 6px; box-sizing: border-box; margin-bottom: 12px; outline: none; }
  .code-input:focus { border-color: var(--accent-border); background: rgba(34,201,151,0.04); }

  .perror { font-size: 12px; color: var(--danger); margin-bottom: 10px; }

  .picker-label { font-size: 11px; color: var(--text-tertiary); text-transform: uppercase; letter-spacing: 1px; margin-bottom: 6px; }
  .picker-item { display: flex; align-items: center; gap: 10px; width: 100%; padding: 10px 12px; margin-bottom: 6px; border-radius: 10px; background: rgba(255,255,255,0.03); border: 1px solid var(--border-subtle); cursor: pointer; font-family: var(--font-sans); color: var(--text-primary); text-align: left; }
  .picker-item.picked { background: rgba(34,201,151,0.08); border-color: var(--accent-border); }
  .picker-tick { color: var(--accent); font-weight: 600; }
  .picker-empty { font-size: 12px; color: var(--text-muted); line-height: 1.6; padding: 14px; background: rgba(255,255,255,0.02); border-radius: 10px; margin-bottom: 12px; text-align: center; }
  .btn-primary:disabled { opacity: 0.6; cursor: wait; }

  .emoji-verify { font-size: 38px; text-align: center; padding: 20px 0; letter-spacing: 6px; }
  .verify-note { font-size: 12px; color: var(--text-tertiary); text-align: center; margin-bottom: 20px; }

  .btn-primary { width: 100%; padding: 13px; background: var(--accent); border: none; border-radius: 10px; color: #111318; font-size: 14px; font-weight: 600; cursor: pointer; font-family: var(--font-sans); margin-bottom: 8px; }
  .btn-close { width: 100%; padding: 12px; border: 1px solid var(--border); background: rgba(255,255,255,0.04); color: var(--text-secondary); border-radius: 10px; font-size: 13px; font-weight: 500; cursor: pointer; font-family: var(--font-sans); }

  .done-state { text-align: center; padding: 20px 0; }
  .done-icon { font-size: 48px; color: var(--accent); margin-bottom: 12px; }
  .done-text { font-size: 18px; font-weight: 600; color: var(--accent); }

  .truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mono { font-family: var(--font-mono); }
</style>