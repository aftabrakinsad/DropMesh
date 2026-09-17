import { writable } from "svelte/store";
import * as api from "../api/tauri.js";

/** Storage statistics */
export const storageStats = writable({
  folder_path: "",
  quota_bytes: 0,
  used_bytes: 0,
  file_size_cap_bytes: 0,
  file_count: 0,
});

/** Whether the first-launch setup has been completed */
export const setupComplete = writable(false);

/** Current active tab */
export const activeTab = writable("devices");

/** Load storage stats from backend */
export async function loadStorageStats() {
  try {
    const stats = await api.getStorageStats();
    storageStats.set(stats);
  } catch (e) {
    console.error("Failed to load storage stats:", e);
  }
}

/** Update storage quota */
export async function updateQuota(quotaBytes) {
  try {
    const stats = await api.updateStorageConfig(quotaBytes, null);
    storageStats.set(stats);
  } catch (e) {
    console.error("Failed to update quota:", e);
    throw e;
  }
}

/** Update file size cap */
export async function updateFileSizeCap(capBytes) {
  try {
    const stats = await api.updateStorageConfig(null, capBytes);
    storageStats.set(stats);
  } catch (e) {
    console.error("Failed to update file size cap:", e);
    throw e;
  }
}
