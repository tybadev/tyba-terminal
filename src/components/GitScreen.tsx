import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import "./gitScreen.css";
import { CiView } from "./CiView";
import { DiffView } from "./DiffView";
import { GitChangesView } from "./GitChangesView";
import { GitHistoryView } from "./GitHistoryView";
import { PrDetailView } from "./PrDetailView";
import { PrListView } from "./PrListView";
import { requestConfirm } from "../lib/confirm";
import { translateError } from "../lib/errors";
import {
  resolvePlace,
  setGitPlace,
  slideOf,
  useGitPlace,
  type GitPlace,
  type GitSection,
} from "../lib/gitScreen";
import {
  forgeWorkflowRuns,
  sessionFetch,
  worktreePull,
  worktreePush,
  type ConflictState,
  type RepoSnapshot,
  type Session,
  type SessionId,
} from "../lib/ipc";
import { snapshotForDir } from "../lib/repoSnapshots";

const TITLE: Record<GitSection, string> = {
  changes: "gitScreenChanges",
  review: "gitScreenReview",
  history: "gitScreenHistory",
  prs: "gitScreenPrs",
  ci: "gitScreenCi",
};

/** O último run da branch falhou? (ponto vermelho no CI do trilho) */
export function useCiFailing(sessionId: SessionId | null, branch: string | null): boolean {
  const [failing, setFailing] = useState(false);
  useEffect(() => {
    setFailing(false);
    if (!sessionId) return;
    let alive = true;
    void forgeWorkflowRuns(sessionId, false)
      .then((runs) => {
        if (!alive || !runs) return;
        const latest = runs.find((r) => !branch || r.head_branch === branch);
        setFailing(latest?.conclusion === "failure");
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [sessionId, branch]);
  return failing;
}

const BTN =
  "shrink-0 whitespace-nowrap rounded-[6px] border border-tyba-border-strong px-2.5 py-1 text-[12px] text-tyba-text-muted hover:text-tyba-text disabled:opacity-50";

/**
 * Tela cheia de Git (protótipo "Git em tela cheia"). Cobre abas e shell; a
 * sidebar de sessões, o header e a barra de status seguem visíveis. Trilho na
 * borda direita; cada seção ocupa a página inteira.
 */
export function GitScreen({
  session,
  repoRoot,
  snapshots,
  exiting,
  editor,
  suggestAgent,
  onClose,
  onFocusSession,
  findSession,
  onSendToAgent,
  onResolveConflicts,
}: {
  session: Session;
  repoRoot: string | null;
  snapshots: Record<string, RepoSnapshot>;
  exiting: boolean;
  editor: string;
  suggestAgent: string;
  onClose: () => void;
  onFocusSession: (id: SessionId) => void;
  findSession: (id: SessionId) => Session | undefined;
  onSendToAgent: (target: Session, prompt: string) => Promise<void>;
  onResolveConflicts: (target: Session, state: ConflictState) => Promise<void>;
}) {
  const { t } = useTranslation();
  const hasWorktree = Boolean(session.worktree);
  const place = resolvePlace(useGitPlace(session.id), hasWorktree);
  const prev = useRef<GitPlace>(place);
  const [slide, setSlide] = useState<"up" | "down" | "left" | "right">("up");
  const [busy, setBusy] = useState<null | "fetch" | "pull" | "push">(null);
  const [note, setNote] = useState<{ text: string; error: boolean } | null>(null);

  const go = (next: GitPlace) => {
    setSlide(slideOf(prev.current, next));
    prev.current = next;
    setGitPlace(session.id, next);
  };
  // Pedido de fora (Ver diff, abrir PR) também anima.
  useEffect(() => {
    if (prev.current.section !== place.section || prev.current.pr !== place.pr) {
      setSlide(slideOf(prev.current, place));
      prev.current = place;
    }
  }, [place]);

  const snapshot = repoRoot ? snapshotForDir(snapshots, repoRoot) : undefined;


  const run = async (kind: "fetch" | "pull" | "push") => {
    if (busy) return;
    if (kind === "push") {
      const ok = await requestConfirm({
        title: t("gitScreenPushConfirm", { branch: snapshot?.branch ?? "HEAD" }),
        detail: t("gitScreenPushConfirmDetail"),
        confirmLabel: t("gitScreenPush"),
      });
      if (!ok) return;
    }
    setBusy(kind);
    setNote(null);
    try {
      if (kind === "fetch") {
        await sessionFetch(session.id);
        setNote({ text: t("gitScreenFetched"), error: false });
      } else if (kind === "pull") {
        const summary = await worktreePull(session.id);
        setNote({ text: t("gitScreenPulled", { summary }), error: false });
      } else {
        await worktreePush(session.id);
        setNote({ text: t("diffPushed"), error: false });
      }
    } catch (e) {
      setNote({ text: translateError(e, t), error: true });
    } finally {
      setBusy(null);
    }
  };

  const titleKey =
    place.section === "prs" && place.pr !== null ? "tabPr" : TITLE[place.section];
  const ahead = snapshot?.ahead ?? null;
  const behind = snapshot?.behind ?? null;
  const animClass = `tyba-git-in-${slide}`;
  const sectionKey = `${place.section}:${place.pr ?? ""}`;

  return (
    <section
      aria-label={t("gitScreenTitle")}
      data-exiting={exiting}
      className="tyba-git-screen absolute inset-0 z-30 flex bg-tyba-bg shadow-[-30px_0_60px_rgba(0,0,0,0.45)]"
      onKeyDown={(e) => {
        if (e.key !== "Escape" || e.defaultPrevented) return;
        const target = e.target as HTMLElement;
        if (
          target.isContentEditable ||
          target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT"
        ) {
          return;
        }
        e.preventDefault();
        onClose();
      }}
    >

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-[46px] shrink-0 items-center gap-3 tyba-divide-b bg-[var(--tyba-panel-bg)] px-[18px] text-[12px]">
          {place.section === "prs" && place.pr !== null && (
            <button type="button" className={BTN} onClick={() => go({ section: "prs", pr: null })}>
              ← {t("gitScreenPrs")}
            </button>
          )}
          <span className="shrink-0 text-[14px] font-medium text-tyba-text">
            {t(titleKey, { number: place.pr ?? 0 })}
          </span>
          <span className="min-w-0 flex-1 truncate text-[11px] text-tyba-text-faint">
            {note ? (
              <span className={note.error ? "text-tyba-red" : "text-tyba-green"} title={note.text}>
                {note.text}
              </span>
            ) : (
              session.title
            )}
          </span>
          <span className="flex shrink-0 items-center gap-2">
            {snapshot?.branch && <span className="font-mono text-tyba-blue">{snapshot.branch}</span>}
            {ahead !== null && behind !== null && (
              <span className="font-mono text-tyba-text-muted">
                ↑{ahead} ↓{behind} {snapshot?.upstream ?? ""}
              </span>
            )}
            <button type="button" className={BTN} disabled={busy !== null} onClick={() => void run("fetch")}>
              {busy === "fetch" ? t("gitScreenFetching") : t("gitScreenFetch")}
            </button>
            <button
              type="button"
              className={BTN}
              disabled={busy !== null || behind === null}
              onClick={() => void run("pull")}
            >
              {busy === "pull" ? t("gitScreenPulling") : t("gitScreenPull")}
            </button>
            <button
              type="button"
              className={`${BTN} text-tyba-text`}
              disabled={busy !== null}
              onClick={() => void run("push")}
            >
              {busy === "push"
                ? t("diffPushing")
                : ahead
                  ? `${t("gitScreenPush")} ↑${ahead}`
                  : t("gitScreenPush")}
            </button>
          </span>
        </div>

        <div key={sectionKey} className={`relative flex min-h-0 flex-1 ${animClass}`}>
          {place.section === "changes" && (
            <GitChangesView session={session} suggestAgent={suggestAgent} />
          )}
          {place.section === "review" && (
            <DiffView
              key={session.id}
              session={session}
              editor={editor}
              suggestAgent={suggestAgent}
              expanded
              asTab
              onToggleExpand={() => {}}
              onClose={onClose}
              onSendToAgent={(prompt) => onSendToAgent(session, prompt)}
              onResolveConflicts={(state) => onResolveConflicts(session, state)}
            />
          )}
          {place.section === "history" && <GitHistoryView session={session} />}
          {place.section === "prs" && place.pr === null && (
            <PrListView
              session={session}
              onFocusSession={onFocusSession}
              onOpenPr={(n) => go({ section: "prs", pr: n })}
            />
          )}
          {place.section === "prs" && place.pr !== null && (
            <PrDetailView
              session={session}
              number={place.pr}
              onForward={async (sid, prompt) => {
                const target = findSession(sid);
                if (target) await onSendToAgent(target, prompt);
              }}
            />
          )}
          {place.section === "ci" && (
            <CiView
              session={session}
              branch={snapshot?.branch ?? null}
              onSendToAgent={(prompt) => onSendToAgent(session, prompt)}
            />
          )}
        </div>
      </div>
    </section>
  );
}
