<script>
  import { open } from "@tauri-apps/plugin-dialog";
  import { sendFiles } from "../api/tauri.js";
  import { selectedDeviceId } from "../stores/devices.js";

  let dragOver = false;

  async function handleDrop(e) {
    e.preventDefault();
    dragOver = false;

    // In Tauri, drag-and-drop provides file paths
    const files = e.dataTransfer?.files;
    if (!files?.length) return;

    const paths = Array.from(files).map((f) => f.path || f.name);

    if ($selectedDeviceId) {
      try {
        await sendFiles(paths, $selectedDeviceId);
      } catch (err) {
        console.error("Send failed:", err);
      }
    }
  }

  async function browseFiles() {
    const selected = await open({
      multiple: true,
      title: "Select files to share",
    });

    if (selected && $selectedDeviceId) {
      const paths = Array.isArray(selected) ? selected : [selected];
      try {
        await sendFiles(paths, $selectedDeviceId);
      } catch (err) {
        console.error("Send failed:", err);
      }
    }
  }
</script>

<div
  class="dropzone"
  class:active={dragOver}
  on:dragover|preventDefault={() => (dragOver = true)}
  on:dragleave={() => (dragOver = false)}
  on:drop={handleDrop}
  on:click={browseFiles}
  role="button"
  tabindex="0"
>
  <div class="icon">{dragOver ? "↓" : "⊕"}</div>
  <div class="label">
    {#if dragOver}
      Release to queue files
    {:else if $selectedDeviceId}
      Drop files here or tap to browse
    {:else}
      Select a device first, then drop files
    {/if}
  </div>
  <div class="hint">Max 2 GB per file</div>
</div>

<style>
  .dropzone {
    margin: 16px 20px 12px;
    border: 2px dashed var(--border);
    border-radius: var(--radius-lg);
    padding: 28px 20px;
    text-align: center;
    cursor: pointer;
    transition: all 0.3s ease;
    background: rgba(255, 255, 255, 0.02);
  }

  .dropzone.active {
    border-color: var(--accent);
    background: rgba(34, 201, 151, 0.06);
    animation: drop-glow 1.5s ease-in-out infinite;
  }

  .icon {
    font-size: 28px;
    margin-bottom: 6px;
    opacity: 0.4;
    transition: opacity 0.3s;
  }

  .dropzone.active .icon {
    opacity: 1;
  }

  .label {
    font-size: 14px;
    font-weight: 500;
    color: var(--text-secondary);
    transition: color 0.3s;
  }

  .dropzone.active .label {
    color: var(--accent);
  }

  .hint {
    font-size: 11px;
    color: var(--text-muted);
    margin-top: 4px;
  }
</style>
