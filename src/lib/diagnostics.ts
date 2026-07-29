import type { BuildInfo } from "./ipc";

const OS_LABELS: Record<string, string> = {
  macos: "macOS",
  linux: "Linux",
  windows: "Windows",
};

export function osLabel(os: string): string {
  return OS_LABELS[os] ?? (os || "—");
}

export function platformLabel(info: BuildInfo): string {
  if (!info.os && !info.arch) return "—";
  if (!info.arch) return osLabel(info.os);
  return `${osLabel(info.os)} · ${info.arch}`;
}

export function shortCommitDate(date: string, locale: string): string {
  if (!date) return "";
  const parsed = new Date(date);
  if (Number.isNaN(parsed.getTime())) return "";
  return parsed.toLocaleDateString(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  });
}

export function formatDiagnostics(info: BuildInfo, locale: string): string {
  const lines = [`TYBA ${info.version || "—"}`];
  if (info.commit) {
    const date = shortCommitDate(info.commit_date, locale);
    lines.push(date ? `commit ${info.commit} (${date})` : `commit ${info.commit}`);
  }
  lines.push(platformLabel(info));
  if (info.webview) lines.push(`webview ${info.webview}`);
  lines.push(`locale ${locale}`);
  return lines.join("\n");
}
