import { writable, derived } from "svelte/store";
import * as api from "../api/tauri.js";

/** All transfers (history + active) */
export const transfers = writable([]);

/** Active transfers only */
export const activeTransfers = derived(transfers, ($transfers) =>
  $transfers.filter((t) => t.status === "in_progress" || t.status === "queued")
);

/** Completed transfers */
export const completedTransfers = derived(transfers, ($transfers) =>
  $transfers.filter((t) => t.status === "completed" || t.status === "failed" || t.status === "cancelled")
);

/** Number of active transfers (for badge) */
export const activeCount = derived(activeTransfers, ($active) => $active.length);

/** Load transfer history from backend */
export async function loadTransfers(statusFilter = null) {
  try {
    const data = await api.getTransfers(statusFilter);
    transfers.set(data);
  } catch (e) {
    console.error("Failed to load transfers:", e);
  }
}

/** Subscribe to transfer progress events */
export function subscribeToTransfers() {
  api.onTransferProgress((progress) => {
    transfers.update((list) => {
      const idx = list.findIndex((t) => t.transfer_id === progress.transfer_id);
      if (idx >= 0) {
        list[idx] = {
          ...list[idx],
          chunks_completed: progress.chunks_completed,
          status: "in_progress",
        };
      }
      return [...list];
    });
  });

  api.onTransferComplete((result) => {
    transfers.update((list) => {
      const idx = list.findIndex((t) => t.transfer_id === result.transfer_id);
      if (idx >= 0) {
        list[idx] = {
          ...list[idx],
          status: result.success ? "completed" : "failed",
          completed_at: new Date().toISOString(),
          error_message: result.error || null,
        };
      }
      return [...list];
    });
  });

  api.onIncomingFile((file) => {
    // Add newly received file to the list
    transfers.update((list) => [file, ...list]);
  });
}

/** Format bytes to human-readable string */
export function formatBytes(bytes) {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}
