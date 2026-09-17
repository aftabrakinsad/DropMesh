/**
 * Typed API layer wrapping Tauri IPC commands.
 * Import and call these functions from Svelte components.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// ── Device ─────────────────────────────────────────────

/** Get this device's identity */
export async function getSelfDevice() {
  return invoke("get_self_device");
}

/** Update this device's display name */
export async function updateDeviceName(name) {
  return invoke("update_device_name", { name });
}

// ── Pairing ────────────────────────────────────────────

/** Start pairing — returns QR payload and numeric code */
export async function startPairing() {
  return invoke("start_pairing");
}

/** Complete pairing with a scanned/entered peer */
export async function completePairing(peerDeviceId, peerPublicKey, peerName, peerType, groupId) {
  return invoke("complete_pairing", {
    peerDeviceId,
    peerPublicKey,
    peerName,
    peerType,
    groupId,
  });
}

/** Get all paired devices */
export async function getPairedDevices() {
  return invoke("get_paired_devices");
}

/** Remove a device from the trust group */
export async function removeDevice(deviceId) {
  return invoke("remove_device", { deviceId });
}

// ── Transfer ───────────────────────────────────────────

/** Send files to a paired device */
export async function sendFiles(filePaths, targetDeviceId) {
  return invoke("send_files", { filePaths, targetDeviceId });
}

/** Cancel an active transfer */
export async function cancelTransfer(transferId) {
  return invoke("cancel_transfer", { transferId });
}

/** Get transfer history, optionally filtered by status */
export async function getTransfers(statusFilter = null) {
  return invoke("get_transfers", { statusFilter });
}

// ── Storage ────────────────────────────────────────────

/** Get storage usage stats */
export async function getStorageStats() {
  return invoke("get_storage_stats");
}

/** Update storage quota and/or file size cap */
export async function updateStorageConfig(quotaBytes = null, fileSizeCapBytes = null) {
  return invoke("update_storage_config", { quotaBytes, fileSizeCapBytes });
}

/** Set the storage folder path */
export async function setStorageFolder(folderPath) {
  return invoke("set_storage_folder", { folderPath });
}

// ── Events (Backend → Frontend) ────────────────────────

/** Listen for device discovered on network */
export function onDeviceDiscovered(callback) {
  return listen("on_device_discovered", (event) => callback(event.payload));
}

/** Listen for device going offline */
export function onDeviceLost(callback) {
  return listen("on_device_lost", (event) => callback(event.payload));
}

/** Listen for transfer progress updates */
export function onTransferProgress(callback) {
  return listen("on_transfer_progress", (event) => callback(event.payload));
}

/** Listen for transfer completion */
export function onTransferComplete(callback) {
  return listen("on_transfer_complete", (event) => callback(event.payload));
}

/** Listen for incoming file received */
export function onIncomingFile(callback) {
  return listen("on_incoming_file", (event) => callback(event.payload));
}
