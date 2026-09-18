// Formulário do Host: o que vai e volta entre o `Host` do core e os campos da
// tela. Validar o render é trabalho do core; aqui só se evita mandar o que a
// tela já sabe que está incompleto.

import type {
  AgentKey,
  AgentKeyListing,
  AuthMethod,
  Host,
  HostInput,
  Tunnel,
} from "./ipc";

export const NO_GROUP_VALUE = "__none__";

export interface HostFormValues {
  alias: string;
  hostname: string;
  port: string;
  username: string;
  auth_method: AuthMethod;
  identity_file: string;
  agent_key: AgentKey | null;
  proxy_jump: string;
  group_id: string;
  color: string | null;
  notes: string;
  tunnels: Tunnel[];
}

export function emptyHostForm(): HostFormValues {
  return {
    alias: "",
    hostname: "",
    port: "",
    username: "",
    auth_method: "auto",
    identity_file: "",
    agent_key: null,
    proxy_jump: "",
    group_id: NO_GROUP_VALUE,
    color: null,
    notes: "",
    tunnels: [],
  };
}

export function hostToForm(host: Host): HostFormValues {
  const declared = host.auth_method ?? "auto";
  // Regra 5: `auto` com arquivo é resto de antes desta entrega.
  const authMethod: AuthMethod =
    declared === "auto" && host.identity_file ? "file" : declared;
  return {
    alias: host.alias,
    hostname: host.hostname,
    port: host.port === null ? "" : String(host.port),
    username: host.username ?? "",
    auth_method: authMethod,
    identity_file: host.identity_file ?? "",
    agent_key: host.agent_key ?? null,
    proxy_jump: host.proxy_jump ?? "",
    group_id: host.group_id ?? NO_GROUP_VALUE,
    color: host.color,
    notes: host.notes ?? "",
    tunnels: host.tunnels,
  };
}

export const usernameNeedsWarning = (username: string): boolean =>
  /\p{Lu}/u.test(username);

export function validPort(p: number | null): boolean {
  return p !== null && Number.isInteger(p) && p >= 1 && p <= 65535;
}

function normalizeTunnelDraft(d: Tunnel): Tunnel | null {
  if (!validPort(d.listen_port)) return null;
  if (d.kind === "dynamic") return { ...d, target_host: null, target_port: null };
  const target = d.target_host?.trim();
  if (!target || !validPort(d.target_port)) return null;
  return { ...d, target_host: target };
}

function tunnelRowHasInput(d: Tunnel): boolean {
  return d.listen_port > 0 || (d.target_port ?? 0) > 0;
}

export type FormResult =
  | { ok: true; input: HostInput }
  | { ok: false; errorKey: string; field: "form" | "tunnels" };

const orNull = (value: string): string | null => value.trim() || null;

export function formToInput(values: HostFormValues): FormResult {
  const method = values.auth_method;
  const fail = (errorKey: string, field: "form" | "tunnels" = "form"): FormResult => ({
    ok: false,
    errorKey,
    field,
  });
  const alias = values.alias.trim();
  const hostname = values.hostname.trim();
  if (!alias) return fail("hostFieldAliasRequired");
  if (!hostname) return fail("hostFieldHostnameRequired");
  let port: number | null = null;
  const portText = values.port.trim();
  if (portText) {
    port = Number(portText);
    if (!validPort(port)) return fail("hostFieldPortInvalid");
  }
  const tunnels: Tunnel[] = [];
  for (const row of values.tunnels) {
    if (!tunnelRowHasInput(row)) continue;
    const norm = normalizeTunnelDraft(row);
    if (norm === null) return fail("hostTunnelInvalid", "tunnels");
    tunnels.push(norm);
  }
  if (method === "agent" && !values.agent_key) {
    return fail("hostAuthAgentKeyRequired");
  }
  if (method === "file" && !values.identity_file.trim()) {
    return fail("hostAuthIdentityFileRequired");
  }
  return {
    ok: true,
    input: {
      alias,
      hostname,
      port,
      username: orNull(values.username),
      auth_method: method,
      identity_file: method === "file" ? orNull(values.identity_file) : null,
      agent_key: method === "agent" ? values.agent_key : null,
      proxy_jump: orNull(values.proxy_jump),
      group_id: values.group_id === NO_GROUP_VALUE ? null : values.group_id,
      color: values.color,
      notes: orNull(values.notes),
      tunnels,
    },
  };
}

/** `update_host` recebe o `Host` inteiro: id, posição e datas vêm do salvo. */
export function applyInputToHost(host: Host, input: HostInput): Host {
  return {
    ...host,
    alias: input.alias,
    hostname: input.hostname,
    port: input.port ?? null,
    username: input.username ?? null,
    auth_method: input.auth_method ?? "auto",
    identity_file: input.identity_file ?? null,
    agent_key: input.agent_key ?? null,
    proxy_jump: input.proxy_jump ?? null,
    group_id: input.group_id ?? null,
    color: input.color ?? null,
    notes: input.notes ?? null,
    tunnels: input.tunnels ?? [],
  };
}

export const agentKeyType = (publicKey: string): string =>
  publicKey.trim().split(/\s+/)[0] ?? "";

export type AgentListingState = "no_agent" | "empty" | "keys";

export const agentListingState = (listing: AgentKeyListing): AgentListingState => {
  if (listing.keys.length > 0) return "keys";
  return listing.socket === null ? "no_agent" : "empty";
};
