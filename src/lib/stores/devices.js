import { writable, derived } from "svelte/store";
import * as api from "../api/tauri.js";

/** This device's identity */
export const selfDevice = writable(null);

/** All paired devices */
export const pairedDevices = writable([]);

/** Online devices (filtered from paired) */
export const onlineDevices = derived(pairedDevices, ($devices) =>
  $devices.filter((d) => d.is_online)
);

/** Offline devices */
export const offlineDevices = derived(pairedDevices, ($devices) =>
  $devices.filter((d) => !d.is_online)
);

/** Currently selected device for sending */
export const selectedDeviceId = writable(null);

/** Load device identity from backend */
export async function loadSelfDevice() {
  try {
    const device = await api.getSelfDevice();
    selfDevice.set(device);
  } catch (e) {
    console.error("Failed to load device identity:", e);
  }
}

/** Load paired devices from backend */
export async function loadPairedDevices() {
  try {
    const devices = await api.getPairedDevices();
    pairedDevices.set(devices);
  } catch (e) {
    console.error("Failed to load paired devices:", e);
  }
}

/** Subscribe to discovery events */
export function subscribeToDiscovery() {
  api.onDeviceDiscovered((device) => {
    pairedDevices.update((devices) => {
      const idx = devices.findIndex((d) => d.device_id === device.device_id);
      if (idx >= 0) {
        devices[idx] = { ...devices[idx], is_online: true };
      }
      return [...devices];
    });
  });

  api.onDeviceLost((device) => {
    pairedDevices.update((devices) => {
      const idx = devices.findIndex((d) => d.device_id === device.device_id);
      if (idx >= 0) {
        devices[idx] = { ...devices[idx], is_online: false };
      }
      return [...devices];
    });
  });
}
