import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { splitPath } from "../lib/agentsDrawer";
import { numberHunk } from "../lib/diff";
import {
  sessionTurnDiff,
  sessionTurnHunks,
  type FileDiff,
  type FileHunks,
  type SessionId,
  type TurnDigest,
} from "../lib/ipc";
import { TURN_LETTER, TURN_LETTER_COLOR } from "./TurnPanel";

const LINE: Record<string, string> = {
  add: "bg-tyba-diff-add-bg text-tyba-diff-add-text",
  del: "bg-tyba-diff-del-bg text-tyba-diff-del-text",
  context: "text-tyba-text-muted",
};

/**
 * Seção Turno da tela de Git ("Ver diff do turno"): o que mudou entre o
 * prompt e o fim do último turno, lido das duas árvores que o core guardou.
 * Só leitura: a pasta de agora pode já ser outra, e preparar ou descartar é
 * da seção Alterações.
 */
export function GitTurnView({
  sessionId,
  turn,
  initialPath,
}: {
  sessionId: SessionId;
  turn: TurnDigest;
  /** Arquivo pedido de fora (clique na linha da aba Turno). */
  initialPath: string | null;
}) {
  const { t } = useTranslation();
  const { seq } = turn;
  const [files, setFiles] = useState<FileDiff[] | null>(null);
  // A lista não veio (as árvores sumiram com o turno seguinte, ou o git
  // falhou): a seção inteira avisa. Trechos que não vieram avisam só no
  // lado do arquivo, e a lista segue escolhível.
  const [listFailed, setListFailed] = useState(false);
  const [hunksFailed, setHunksFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [pick, setPick] = useState<FileDiff | null>(null);
  const [hunks, setHunks] = useState<FileHunks | null>(null);
  const wanted = useRef(initialPath);
  wanted.current = initialPath;

  useEffect(() => {
    let alive = true;
    setFiles(null);
    setListFailed(false);
    setPick(null);
    sessionTurnDiff(sessionId, seq)
      .then((list) => {
        if (!alive) return;
        setFiles(list);
        setPick(list.find((f) => f.path === wanted.current) ?? list[0] ?? null);
      })
      .catch(() => alive && setListFailed(true));
    return () => {
      alive = false;
    };
  }, [sessionId, seq, attempt]);

  // Arquivo pedido de fora com a seção já aberta: só troca a escolha. Pedido
  // nenhum (clique no trilho) não mexe no que o dono já escolheu.
  useEffect(() => {
    const asked = initialPath && files?.find((f) => f.path === initialPath);
    if (asked) setPick(asked);
  }, [initialPath, files]);

  useEffect(() => {
    setHunks(null);
    setHunksFailed(false);
    if (!pick) return;
    let alive = true;
    sessionTurnHunks(sessionId, seq, pick.path, pick.old_path)
      .then((h) => alive && setHunks(h))
      .catch(() => alive && setHunksFailed(true));
    return () => {
      alive = false;
    };
  }, [sessionId, seq, pick, attempt]);

  const retry = (
    <button
      type="button"
      onClick={() => setAttempt((n) => n + 1)}
      className="hover:underline"
    >
      {t("gitRefreshRetry")}
    </button>
  );

  if (listFailed) {
    return (
      <div className="flex flex-1 items-center justify-center gap-1.5 text-[12px] text-tyba-text-faint">
        {t("gitTurnFailed")} · {retry}
      </div>
    );
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      <div className="flex w-[360px] shrink-0 flex-col shadow-[inset_-1px_0_0_var(--tyba-line)]">
        <div className="min-h-0 flex-1 overflow-y-auto p-2">
          {files && (
            <div className="px-2 pb-1.5 pt-1.5">
              <span className="tyba-label">{t("gitTurnFiles", { count: files.length })}</span>
            </div>
          )}
          {files?.map((file) => {
            const { dir, name } = splitPath(file.path);
            const select = () => setPick(file);
            return (
              <div
                key={file.path}
                role="button"
                tabIndex={0}
                onClick={select}
                onKeyDown={(e) => {
                  if (e.key === "Enter") select();
                }}
                className={`flex cursor-pointer items-center gap-2 rounded-[6px] px-2 py-[5px] text-[12.5px] outline-none ${
                  pick?.path === file.path
                    ? "tyba-pill-on"
                    : "hover:bg-tyba-raised/60 focus-visible:bg-tyba-raised/60"
                }`}
              >
                <span className={`shrink-0 font-mono ${TURN_LETTER_COLOR[file.status]}`}>
                  {TURN_LETTER[file.status]}
                </span>
                <span className="min-w-0 flex-1 truncate" title={file.path}>
                  <span className="text-tyba-text">{name}</span>
                  {dir && (
                    <span className="pl-1.5 text-[11px] text-tyba-text-faint">
                      {dir.slice(0, -1)}
                    </span>
                  )}
                </span>
                <span className="shrink-0 font-mono text-[11px]">
                  {(file.insertions ?? 0) > 0 && (
                    <span className="text-tyba-green">+{file.insertions}</span>
                  )}{" "}
                  {(file.deletions ?? 0) > 0 && (
                    <span className="text-tyba-red">−{file.deletions}</span>
                  )}
                </span>
              </div>
            );
          })}
        </div>
      </div>

      <div className="flex min-w-0 flex-1 flex-col">
        {pick && (
          <>
            <div className="flex shrink-0 items-center gap-2 px-[18px] py-2 text-[12px] shadow-[inset_0_-1px_0_var(--tyba-line)]">
              <span className="min-w-0 flex-1 truncate" title={pick.path}>
                <span className="text-tyba-text">{splitPath(pick.path).name}</span>
                <span className="pl-1.5 text-[11px] text-tyba-text-faint">
                  {splitPath(pick.path).dir.slice(0, -1)}
                </span>
              </span>
            </div>
            <div className="min-h-0 flex-1 overflow-auto py-1.5 font-mono text-[12.5px] leading-[21px]">
              {hunksFailed && (
                <p className="px-[18px] py-3 font-sans text-[12px] text-tyba-text-faint">
                  {t("gitTurnFailed")} · {retry}
                </p>
              )}
              {hunks?.binary && (
                <p className="px-[18px] py-3 text-tyba-text-faint">{t("diffBinary")}</p>
              )}
              {hunks?.hunks.map((hunk, index) => (
                <div key={`${index}:${hunk.header}`} className="pb-3">
                  <div className="truncate px-[18px] py-0.5 text-[11px] text-tyba-violet">
                    {hunk.header}
                  </div>
                  {numberHunk(hunk).map((l) => (
                    <div key={l.idx} className={`flex whitespace-pre ${LINE[l.line.kind]}`}>
                      <span className="w-11 shrink-0 select-none pr-2.5 text-right text-tyba-text-ghost">
                        {l.oldNo ?? ""}
                      </span>
                      <span className="w-11 shrink-0 select-none pr-2.5 text-right text-tyba-text-ghost">
                        {l.newNo ?? ""}
                      </span>
                      <span className="w-4 shrink-0 select-none text-center">
                        {l.line.kind === "add" ? "+" : l.line.kind === "del" ? "−" : " "}
                      </span>
                      <span className="pr-4">{l.line.text}</span>
                    </div>
                  ))}
                </div>
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
