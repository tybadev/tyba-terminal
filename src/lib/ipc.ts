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

export const createSession = (opts: CreateSessionOpts) =>
  invoke<Session>("create_session", { opts });

export const writeToSession = (id: SessionId, data: string) =>
  invoke<void>("write_to_session", { id, data: encodeBase64(data) });

export const sessionScrollback = (id: SessionId): Promise<Uint8Array> =>
  invoke<string>("session_scrollback", { id }).then(decodeBase64);

export const resizeSession = (id: SessionId, cols: number, rows: number) =>
  invoke<void>("resize_session", { id, cols, rows });

export const listSessions = () => invoke<Session[]>("list_sessions");

export const disposeSession = (id: SessionId) =>
  invoke<void>("dispose_session", { id });

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
