import {
  readText,
  writeText,
} from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";

export function readClipboardText(): Promise<string> {
  return readText();
}

export function writeClipboardText(text: string): Promise<void> {
  return writeText(text);
}

export function isMultilinePaste(text: string): boolean {
  return text.includes("\n") || text.includes("\r");
}

export function flattenPaste(text: string): string {
  return text.replace(/\r\n|\n|\r/g, " ").replace(/ {2,}/g, " ");
}

const SAFE_URL_PROTOCOLS = new Set(["http:", "https:"]);

export function isSafeExternalUrl(raw: string): boolean {
  try {
    const url = new URL(raw);
    return SAFE_URL_PROTOCOLS.has(url.protocol) && url.hostname.length > 0;
  } catch {
    return false;
  }
}

export async function openExternalUrl(raw: string): Promise<void> {
  if (!isSafeExternalUrl(raw)) return;
  await openUrl(raw);
}
