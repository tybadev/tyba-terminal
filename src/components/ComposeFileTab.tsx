import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { CodeEditor } from "./CodeEditor";
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
  const [isDark, setIsDark] = useState(() => getEffectiveBase() === "dark");
  useEffect(() => onEffectiveBaseChange(() => setIsDark(getEffectiveBase() === "dark")), []);

  useEffect(() => {
    let alive = true;
    setFile(null);
    setError(null);
    void dockerComposeFile(project, index)
      .then((f) => alive && setFile(f))
      .catch((e) => alive && setError(translateError(e, t)));
    return () => {
      alive = false;
    };
  }, [project, index, t]);

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
        {error ? (
          <p role="alert" className="px-[18px] py-3 text-[12px] text-tyba-red">
            {error}
          </p>
        ) : !file ? (
          <p className="px-[18px] py-3 text-[12px] text-tyba-text-faint">{t("filesLoading")}</p>
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
