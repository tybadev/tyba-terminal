// Camada IPC: espelho tipado dos commands do core Rust.
// Princípio #1 do CLAUDE.md: nenhuma lógica de sessão aqui —
// só serialização de intenções e subscrição de eventos.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SessionId = string;

export type SessionKind =
  | { type: "shell" }
  | { type: "agent"; runner: "claude_code" | "codex" | { custom: string } };

export type SessionStatus =
  | { state: "running" }
  | { state: "awaiting_input"; hint: string | null }
  | { state: "idle" }
  | { state: "exited"; code: number }
  | { state: "failed"; reason: string };

export interface Session {
  id: SessionId;
  kind: SessionKind;
  title: string;
  repo_root: string | null;
  worktree: unknown | null;
  status: SessionStatus;
  created_at: string;
}

export interface CreateSessionOpts {
  kind: SessionKind;
  title?: string;
  cwd?: string;
  cols: number;
  rows: number;
}

// --- base64 <-> bytes (chunks de PTY podem quebrar UTF-8 no meio) ---

const decodeBase64 = (data: string): Uint8Array =>
  Uint8Array.from(atob(data), (c) => c.charCodeAt(0));

const encodeBase64 = (data: string): string => {
  const bytes = new TextEncoder().encode(data);
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
};

// --- commands ---

export const createSession = (opts: CreateSessionOpts) =>
  invoke<Session>("create_session", { opts });

export const writeToSession = (id: SessionId, data: string) =>
  invoke<void>("write_to_session", { id, data: encodeBase64(data) });

export const resizeSession = (id: SessionId, cols: number, rows: number) =>
  invoke<void>("resize_session", { id, cols, rows });

export const listSessions = () => invoke<Session[]>("list_sessions");

export const disposeSession = (id: SessionId) =>
  invoke<void>("dispose_session", { id });

// --- aprovações ---

export type RiskLevel = "green" | "yellow" | "red";
export type ApprovalDecision = "approved" | "denied";

export interface ApprovalRequest {
  id: number;
  session_id: SessionId;
  command: string;
  cwd: string | null;
  risk: RiskLevel;
  context: string | null;
  requested_at_ms: number;
}

export const requestApproval = (
  sessionId: SessionId,
  command: string,
  cwd?: string,
  context?: string,
) =>
  invoke<ApprovalRequest>("request_approval", {
    sessionId,
    command,
    cwd: cwd ?? null,
    context: context ?? null,
  });

export const listApprovals = () =>
  invoke<ApprovalRequest[]>("list_approvals");

export const resolveApproval = (id: number, decision: ApprovalDecision) =>
  invoke<void>("resolve_approval", { id, decision });

export const onApprovalRequested = (
  handler: (request: ApprovalRequest) => void,
): Promise<UnlistenFn> =>
  listen<ApprovalRequest>("approvals://requested", (e) => handler(e.payload));

export const onApprovalResolved = (
  handler: (resolved: { id: number; decision: ApprovalDecision }) => void,
): Promise<UnlistenFn> =>
  listen<{ id: number; decision: ApprovalDecision }>(
    "approvals://resolved",
    (e) => handler(e.payload),
  );

// --- events ---

export const onPtyOutput = (
  id: SessionId,
  handler: (bytes: Uint8Array) => void,
): Promise<UnlistenFn> =>
  listen<{ data: string }>(`pty://output/${id}`, (e) =>
    handler(decodeBase64(e.payload.data)),
  );

export const onPtyExit = (
  id: SessionId,
  handler: () => void,
): Promise<UnlistenFn> => listen(`pty://exit/${id}`, () => handler());

export const onSessionStatus = (
  id: SessionId,
  handler: (session: Session) => void,
): Promise<UnlistenFn> =>
  listen<Session>(`session://status/${id}`, (e) => handler(e.payload));
