import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { CodeEditor } from "./CodeEditor";
import { LoadingState } from "./ui/loading";
import { getEffectiveBase, onEffectiveBaseChange } from "@/theme";
import { translateError } from "../lib/errors";
import { dockerComposeFile } from "../lib/ipc";

/**
 * Aba só leitura da tela de Arquivos com um arquivo compose de qualquer
 * projeto (Ver compose.yml, seção Compose do Docker). Nada se grava: o
 * arquivo pode estar fora do worktree da sessão.
 */
export function ComposeFileTab({ project, index }: { project: string; index: number }) {
  const { t } = useTranslation();
  const [file, setFile] = useState<{ path: string; text: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [startedAt, setStartedAt] = useState(() => Date.now());
  // "Tentar de novo" só muda o nonce: o efeito relê do começo.
  const [reload, setReload] = useState(0);
  const [isDark, setIsDark] = useState(() => getEffectiveBase() === "dark");
  useEffect(() => onEffectiveBaseChange(() => setIsDark(getEffectiveBase() === "dark")), []);

  useEffect(() => {
    let alive = true;
    setFile(null);
    setError(null);
    setStartedAt(Date.now());
    void dockerComposeFile(project, index)
      .then((f) => alive && setFile(f))
      .catch((e) => alive && setError(translateError(e, t)));
    return () => {
      alive = false;
    };
  }, [project, index, t, reload]);

  return (
    <div className="absolute inset-0 flex flex-col">
      <div className="flex shrink-0 items-center gap-2.5 border-b border-[var(--tyba-border-soft)] px-[18px] py-1.5 text-[11px] text-tyba-text-faint">
        <span className="min-w-0 flex-1 truncate font-mono" title={file?.path}>
          {file?.path ?? project}
        </span>
        <span className="shrink-0 rounded-[4px] border border-tyba-border-strong px-[5px] text-tyba-text-muted">
          {t("composeFileReadOnly")}
        </span>
      </div>
      <div className="relative min-h-0 flex-1">
        {!file ? (
          <LoadingState
            op="editor"
            step={t("loadComposeFile", { project })}
            em={[project]}
            startedAt={startedAt}
            slowCause={t("loadFilesSlow")}
            error={error}
            onRetry={() => setReload((n) => n + 1)}
            shape="lines"
            rows={8}
            className="absolute inset-0"
          />
        ) : (
          <div className="absolute inset-0">
            <CodeEditor
              doc={file.text}
              filename={file.path}
              dark={isDark}
              markers={[]}
              readOnly
              onDirtyChange={() => {}}
              onSave={() => {}}
            />
          </div>
        )}
      </div>
    </div>
  );
}
