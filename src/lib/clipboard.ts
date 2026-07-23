import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";

export async function readClipboardText(): Promise<string> {
  try {
    return await readText();
  } catch {
    return (await navigator.clipboard?.readText()) ?? "";
  }
}

export async function writeClipboardText(text: string): Promise<void> {
  try {
    await writeText(text);
  } catch {
    await navigator.clipboard?.writeText(text);
  }
}

const BRACKET_PASTE_END = /\x1b\[201~/g;
const TRAILING_NEWLINE = /(?:\r\n|\n|\r)$/;
const NEWLINE_RUN = /(?:\r\n|\n|\r)+/g;
const UNSAFE_CONTROL = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/;
const UNSAFE_CONTROL_GLOBAL = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g;

export function sanitizePaste(text: string): string {
  return text.replace(BRACKET_PASTE_END, "");
}

export function stripTrailingNewline(text: string): string {
  return text.replace(TRAILING_NEWLINE, "");
}

export function hasUnsafeControlChars(text: string): boolean {
  return UNSAFE_CONTROL.test(text);
}

export function isMultilinePaste(text: string): boolean {
  return /\r|\n/.test(stripTrailingNewline(text));
}

export function flattenPaste(text: string): string {
  return stripTrailingNewline(text)
    .replace(NEWLINE_RUN, " ")
    .replace(UNSAFE_CONTROL_GLOBAL, "");
}

const SAFE_URL_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);

export function isSafeExternalUrl(raw: string): boolean {
  try {
    const url = new URL(raw);
    if (!SAFE_URL_PROTOCOLS.has(url.protocol)) return false;
    if (url.protocol === "mailto:") return url.pathname.length > 0;
    if (url.hostname.length === 0) return false;
    if (url.username.length > 0 || url.password.length > 0) return false;
    return true;
  } catch {
    return false;
  }
}

export async function openExternalUrl(raw: string): Promise<void> {
  if (!isSafeExternalUrl(raw)) return;
  await openUrl(raw);
}
