import type { Tunnel } from "@/lib/ipc";

export const BLANK_TUNNEL: Tunnel = {
  kind: "local",
  listen_port: 0,
  listen_host: null,
  target_host: "localhost",
  target_port: null,
};

export function describeTunnel(t: Tunnel): string {
  const bind = t.listen_host ?? "127.0.0.1";
  if (t.kind === "dynamic") return `${bind}:${t.listen_port} → SOCKS`;
  return `${bind}:${t.listen_port} → ${t.target_host}:${t.target_port}`;
}

export function tunnelFlag(t: Tunnel): string {
  if (t.kind === "local") return "-L";
  if (t.kind === "remote") return "-R";
  return "-D";
}

function exposure(t: Tunnel): string {
  return [
    t.kind,
    t.listen_host ?? "127.0.0.1",
    t.listen_port,
    t.target_host ?? "",
    t.target_port ?? "",
  ].join("|");
}

export function addedRiskyTunnels(prev: Tunnel[], next: Tunnel[]): Tunnel[] {
  const seen = new Set(prev.map(exposure));
  return next.filter((t) => t.kind !== "local" && !seen.has(exposure(t)));
}
