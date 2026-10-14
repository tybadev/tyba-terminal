import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowBendDownRight,
  ArrowUp,
  CaretDown,
  CaretRight,
  FolderSimple,
  X,
} from "@phosphor-icons/react";

import {
  dockerContainerDetail,
  dockerFsList,
  dockerFsRead,
  exportDownload,
  exportRedacted,
  type ContainerFsEntry,
  type ContainerFsFile,
} from "@/lib/ipc";
import { containerName, rememberContainerName } from "@/lib/contentTabs";
import { writeClipboardText } from "@/lib/clipboard";
import { formatSize, pathCrumbs } from "@/lib/containerFs";
import { pushToast, toastError } from "@/lib/toast";
import { PANEL_HEADER } from "./SidePanelBar";

interface Props {
  containerId: string;
  onClose: () => void;
}

function join(dir: string, name: string): string {
  return dir === "/" ? `/${name}` : `${dir}/${name}`;
}

function parentOf(dir: string): string {
  const i = dir.lastIndexOf("/");
  return i <= 0 ? "/" : dir.slice(0, i);
}

/**
 * Arquivos do container, só leitura: navegar, espiar, copiar e baixar
 * (mascarados). Editar fica de fora de propósito — o que se escreve aqui some
 * no próximo deploy.
 */
export function ContainerFilesPanel({ containerId, onClose }: Props) {
  const { t } = useTranslation();
  const [root, setRoot] = useState<string | null>(null);
  const [entries, setEntries] = useState<Record<string, ContainerFsEntry[]>>({});
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [file, setFile] = useState<ContainerFsFile | null>(null);

  useEffect(() => {
    let alive = true;
    void dockerContainerDetail(containerId)
      .then((d) => {
        if (!alive) return;
        rememberContainerName(containerId, d.name);
        setRoot(d.workdir ?? "/");
      })
      .catch(() => {
        if (alive) setRoot("/");
      });
    return () => {
      alive = false;
    };
  }, [containerId]);

  const list = useCallback(
    (dir: string) => {
      void dockerFsList(containerId, dir)
        .then((items) => {
          setEntries((prev) => ({ ...prev, [dir]: items }));
          setError(null);
        })
        .catch((e) => setError(String(e)));
    },
    [containerId],
  );

  useEffect(() => {
    if (!root) return;
    setEntries({});
    setOpen(new Set());
    list(root);
  }, [root, list]);

  const toggle = (dir: string) => {
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(dir)) next.delete(dir);
      else {
        next.add(dir);
        if (!entries[dir]) list(dir);
      }
      return next;
    });
  };

  const select = (path: string) => {
    setSelected(path);
    setFile(null);
    void dockerFsRead(containerId, path)
      .then(setFile)
      .catch((e) => toastError(t("containerFileReadError"), e));
  };

  const copy = async () => {
    if (!file) return;
    try {
      await writeClipboardText(await exportRedacted([file.text]));
      pushToast({ title: t("containerFileCopied") });
    } catch (e) {
      toastError(t("logsCopyError"), e);
    }
  };

  const download = async () => {
    if (!file || !selected) return;
    try {
      const name = selected.slice(selected.lastIndexOf("/") + 1);
      const path = await exportDownload(`${containerName(containerId)}-${name}`, [file.text]);
      pushToast({ title: t("logsDownloaded"), detail: path });
    } catch (e) {
      toastError(t("logsDownloadError"), e);
    }
  };

  const renderDir = (dir: string, depth: number): React.ReactNode =>
    (entries[dir] ?? []).map((entry) => {
      const path = join(dir, entry.name);
      return (
        <div key={path}>
          <button
            type="button"
            onClick={() => (entry.is_dir ? toggle(path) : select(path))}
            // Mockup ContainerShell: linha de 24px, 12,5px, cantos de 5px, sem
            // ícone de arquivo; tamanho em 11px apagado.
            className={`flex h-6 w-full items-center gap-1 rounded-[5px] pr-2 text-left text-[12.5px] ${
              selected === path
                ? "bg-tyba-raised text-tyba-text shadow-[var(--tyba-row-raised)]"
                : entry.is_dir
                  ? "text-tyba-text-muted hover:bg-tyba-text/[.04]"
                  : "text-tyba-text hover:bg-tyba-text/[.04]"
            }`}
            style={{ paddingLeft: 4 + depth * 15 }}
          >
            <span className="flex size-4 shrink-0 items-center justify-center text-tyba-text-faint">
              {entry.is_dir ? (
                open.has(path) ? (
                  <CaretDown size={10} weight="bold" />
                ) : (
                  <CaretRight size={10} weight="bold" />
                )
              ) : null}
            </span>
            <span className="min-w-0 truncate">{entry.name}</span>
            {entry.link_target !== null && (
              <span
                title={t("containerFilesLink", { target: entry.link_target })}
                className="flex min-w-0 shrink items-center gap-0.5 font-mono text-[10px] text-tyba-text-faint"
              >
                <ArrowBendDownRight size={10} className="shrink-0" />
                <span className="truncate">{entry.link_target}</span>
              </span>
            )}
            {entry.size !== null && (
              <span className="ml-auto shrink-0 pl-2 font-mono text-[11px] tabular-nums text-tyba-text-faint">
                {formatSize(entry.size)}
              </span>
            )}
          </button>
          {entry.is_dir && open.has(path) && renderDir(path, depth + 1)}
        </div>
      );
    });

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {/* Fundo do painel lateral que o contém; cabeçalho como o dos mockups
          Painel*: `padding: 12px 16px`, ícone 16, título 13px. */}
      <header className={PANEL_HEADER}>
        <FolderSimple size={16} className="shrink-0 text-tyba-blue" />
        <span className="min-w-0 truncate text-[13px] text-tyba-text">
          {t("containerFilesTitle")}
        </span>
        <span
          title={t("containerFilesNote")}
          className="shrink-0 rounded-[3px] bg-tyba-text/[.06] px-1.5 py-0.5 text-[10px] text-tyba-text-muted"
        >
          {t("containerFilesReadOnlyChip")}
        </span>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onClose}
          aria-label={t("tunnelsClose")}
          className="text-tyba-text-faint hover:text-tyba-text"
        >
          <X size={14} />
        </button>
      </header>
      <div className="flex shrink-0 items-center gap-2 border-b border-[var(--tyba-border-soft)] px-4 py-2.5 font-mono text-[11px] text-tyba-text-faint">
        <nav
          aria-label={t("containerFilesPath")}
          title={`${containerName(containerId)}:${root ?? ""}`}
          className="flex min-w-0 flex-1 items-center overflow-hidden whitespace-nowrap"
        >
          <span className="shrink-0 text-tyba-text-muted">{containerName(containerId)}:</span>
          {root &&
            pathCrumbs(root).map((c, i, all) => (
              <span key={c.path} className={`flex items-center ${i === all.length - 1 ? "min-w-0" : "shrink-0"}`}>
                {i > 1 && <span className="px-px">/</span>}
                {i === all.length - 1 ? (
                  <span className="truncate text-tyba-text-muted">{c.label}</span>
                ) : (
                  <button
                    type="button"
                    onClick={() => setRoot(c.path)}
                    className="hover:text-tyba-text"
                  >
                    {c.label}
                  </button>
                )}
              </span>
            ))}
        </nav>
        {root && root !== "/" && (
          <button
            type="button"
            onClick={() => setRoot(parentOf(root))}
            aria-label={t("containerFilesUp")}
            title={t("containerFilesUp")}
            className="hover:text-tyba-text"
          >
            <ArrowUp size={12} />
          </button>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-2 py-2.5">
        {error && <div className="px-3 py-2 text-[11px] text-tyba-red">{error}</div>}
        {root && renderDir(root, 0)}
      </div>
      {selected && (
        <div className="mx-4 my-1.5 flex max-h-[45%] min-h-0 flex-col overflow-hidden rounded-[8px] border border-tyba-text/[.08]">
          <div className="flex shrink-0 items-center gap-2 bg-tyba-raised px-2.5 py-1.5 text-[11px] text-tyba-text-muted">
            <span className="min-w-0 flex-1 truncate">
              {t("containerFileReadOnly", { name: selected.slice(selected.lastIndexOf("/") + 1) })}
            </span>
            <button
              type="button"
              disabled={!file || file.binary}
              onClick={() => void download()}
              className="hover:text-tyba-text disabled:opacity-50"
            >
              {t("logsDownload")}
            </button>
            <button
              type="button"
              disabled={!file || file.binary}
              onClick={() => void copy()}
              className="hover:text-tyba-text disabled:opacity-50"
            >
              {t("logsCopy")}
            </button>
          </div>
          <pre className="min-h-0 flex-1 overflow-auto px-2.5 py-2 font-mono text-[12px] leading-[19px] text-tyba-text-muted">
            {!file
              ? t("filesLoading")
              : file.binary
                ? t("containerFileBinary")
                : file.text}
            {file?.truncated && `\n… ${t("containerFileTruncated")}`}
          </pre>
        </div>
      )}
      <p className="px-3 py-2.5 text-[11px] leading-[17px] text-tyba-text-faint">
        {t("containerFilesNote")}
      </p>
    </div>
  );
}
