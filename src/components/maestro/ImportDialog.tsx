import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { FolderSimple, Warning } from "@phosphor-icons/react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  importConfirm,
  importItems,
  missingImportDirs,
  rememberDir,
  revealHidden,
  sourceDetail,
  unusedImportIntegrations,
  type ImportDir,
  type ImportItem,
  type ImportPreview,
  type Integration,
} from "@/lib/maestro";

/** Texto que vai rodar: inteiro, com o invisível à vista. Nunca resumo. */
function Code({ children }: { children: string }) {
  const { t } = useTranslation();
  return (
    <pre className="max-h-72 overflow-auto rounded-[5px] border border-tyba-border bg-tyba-sunken px-3 py-2 font-mono text-[11.5px] leading-[17px] whitespace-pre-wrap break-all text-tyba-text">
      {children === "" ? (
        <span className="text-tyba-text-ghost">{t("maestroImportEmptyText")}</span>
      ) : (
        revealHidden(children)
      )}
    </pre>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-[10px] tracking-[0.08em] text-tyba-text-faint uppercase">{title}</h3>
      {children}
    </section>
  );
}

function IntegrationBody({ integration }: { integration: Integration }) {
  const { t } = useTranslation();
  const trigger =
    integration.trigger.type === "manual"
      ? t("maestroIntManual")
      : t("maestroIntOnInput", { kind: t(`maestroEdge_${integration.trigger.kind}`) });
  return (
    <>
      <span className="text-[11px] text-tyba-text-faint">
        {revealHidden(integration.name)} · {trigger} · {t("maestroOutput")}:{" "}
        {t(`maestroEdge_${integration.output}`)}
        {integration.file ? ` (${revealHidden(integration.file)})` : ""}
      </span>
      <Code>{integration.script}</Code>
    </>
  );
}

/**
 * Onde o nó vai rodar: a pasta do arquivo, ou a escolhida. A que não existe
 * nesta máquina fica em âmbar até o dono escolher uma.
 */
function DirRow({
  info,
  chosen,
  onPick,
}: {
  info: ImportDir;
  chosen: string | undefined;
  onPick: (dir: string) => void;
}) {
  const { t } = useTranslation();
  const missing = !info.exists && !chosen;
  const pick = async () => {
    const dir = await openDialog({
      directory: true,
      multiple: false,
      title: t("maestroNodeDirPick"),
    }).catch(() => null);
    if (typeof dir === "string") onPick(dir);
  };
  return (
    <span
      className={`flex items-center gap-1.5 text-[11.5px] ${
        missing ? "text-tyba-amber" : "text-tyba-text-muted"
      }`}
    >
      <FolderSimple size={12} className="shrink-0" />
      <span className="text-tyba-text-faint">{t("maestroImportDir")}</span>
      <span className="min-w-0 break-all">
        {chosen ?? revealHidden(info.dir || "—")}
        {!info.exists && !chosen && ` · ${t("maestroImportDirMissing")}`}
      </span>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => void pick()}
        className="ml-auto h-6 shrink-0 px-2 text-[11px]"
      >
        {missing ? t("maestroImportDirPick") : t("maestroImportDirChange")}
      </Button>
    </span>
  );
}

function ItemView({ item, dir }: { item: ImportItem; dir: ReactNode }) {
  const { t } = useTranslation();
  const head = (kind: string, extra?: string) => (
    <span className="flex items-baseline gap-2 text-[12px] text-tyba-text">
      <span className="font-medium">{revealHidden(item.node.title)}</span>
      <span className="text-[11px] text-tyba-text-faint">
        {kind}
        {extra ? ` · ${extra}` : ""}
      </span>
    </span>
  );
  switch (item.type) {
    case "agent":
      return (
        <div className="flex flex-col gap-1.5">
          {head(item.runner === "claude" ? "Claude" : "Codex", revealHidden(item.role))}
          {dir}
          <Code>{item.prompt}</Code>
        </div>
      );
    case "process":
      return (
        <div className="flex flex-col gap-1.5">
          {head(
            t("maestroKindProcess"),
            `${t("maestroOutput")}: ${t(`maestroEdge_${item.output}`)}` +
              (item.file ? ` (${revealHidden(item.file)})` : ""),
          )}
          {dir}
          <Code>{item.command}</Code>
        </div>
      );
    case "note":
      return (
        <div className="flex flex-col gap-1.5">
          {head(t("maestroKindNote"))}
          <Code>{item.markdown}</Code>
        </div>
      );
    case "integration":
      return (
        <div className="flex flex-col gap-1.5">
          {head(t("maestroKindIntegration"))}
          {dir}
          {item.integration ? (
            <IntegrationBody integration={item.integration} />
          ) : (
            <span className="text-[11.5px] text-tyba-amber">{t("maestroIntegrationMissing")}</span>
          )}
        </div>
      );
    case "browser":
      return (
        <div className="flex flex-col gap-1.5">
          {head(t("maestroKindBrowser"))}
          <Code>{item.url}</Code>
        </div>
      );
    case "service":
      return (
        <div className="flex flex-col gap-1.5">
          {head(t("maestroKindService"), t(`maestroSource_${item.source.type}`))}
          <Code>{sourceDetail(item.source)}</Code>
        </div>
      );
  }
}

/**
 * A tela de importação é o consentimento: mostra TUDO o que o arquivo traz
 * para rodar — prompts, comandos, scripts inteiros, variáveis e a pasta de
 * cada nó — e só grava no Confirmar. O core confere que o arquivo é o mesmo
 * da prévia (hash) e nada roda aqui. Pasta que não existe nesta máquina o
 * dono escolhe antes: sem isso o Confirmar fica desligado (e o core recusa).
 * `defaultDir` vira a pasta padrão do grafo.
 */
export function ImportDialog({
  path,
  defaultDir,
  preview,
  onCancel,
  onImported,
}: {
  path: string;
  defaultDir: string;
  preview: ImportPreview;
  onCancel: () => void;
  onImported: (graphId: string) => void;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { graph } = preview;
  const items = importItems(preview);
  const unused = unusedImportIntegrations(preview);
  const variables = Object.entries(graph.variables);
  const [chosen, setChosen] = useState<Record<string, string>>({});
  const pending = missingImportDirs(preview.dirs, chosen);
  const dirOf = new Map(preview.dirs.map((d) => [d.nodeId, d]));

  const confirm = async () => {
    setBusy(true);
    try {
      onImported(await importConfirm(path, preview.hash, defaultDir, chosen));
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && !busy && onCancel()}>
      <DialogContent className="flex max-h-[86vh] max-w-[720px] flex-col gap-0 rounded-[6px] border-tyba-border-strong bg-tyba-surface p-0 font-mono shadow-2xl sm:max-w-[720px]">
        <DialogHeader className="tyba-divide-b px-4 py-3">
          <DialogTitle className="text-[13px] text-tyba-text">{t("maestroImportTitle")}</DialogTitle>
          <DialogDescription className="text-[12px] text-tyba-text-faint">
            {t("maestroImportBody")}
          </DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto px-4 py-4">
          <Section title={t("maestroImportGraph")}>
            <dl className="grid grid-cols-[max-content_1fr] gap-x-3 gap-y-1 text-[12px]">
              <dt className="text-tyba-text-faint">{t("maestroImportName")}</dt>
              <dd className="break-all text-tyba-text">{revealHidden(graph.name)}</dd>
              <dt className="text-tyba-text-faint">{t("maestroImportFile")}</dt>
              <dd className="break-all text-tyba-text-muted">{path}</dd>
              <dt className="text-tyba-text-faint">SHA-256</dt>
              <dd className="break-all text-tyba-text-muted">{preview.hash}</dd>
            </dl>
            {(preview.replacesGraph || preview.renamedIntegrations.length > 0) && (
              <p className="flex items-start gap-2 rounded-[5px] bg-tyba-amber-tint px-3 py-2 text-[11.5px] text-tyba-amber">
                <Warning size={13} className="mt-px shrink-0" />
                <span>
                  {preview.replacesGraph && t("maestroImportReplacesGraph")}{" "}
                  {preview.renamedIntegrations.length > 0 &&
                    t("maestroImportRenamesIntegrations", {
                      count: preview.renamedIntegrations.length,
                    })}
                </span>
              </p>
            )}
          </Section>

          <Section title={t("maestroVariables")}>
            {variables.length === 0 ? (
              <span className="text-[11.5px] text-tyba-text-faint">{t("maestroImportNoVars")}</span>
            ) : (
              <dl className="grid grid-cols-[max-content_1fr] gap-x-3 gap-y-1 text-[12px]">
                {variables.map(([name, value]) => (
                  <div key={name} className="contents">
                    <dt className="text-tyba-text">{revealHidden(name)}</dt>
                    <dd className="break-all whitespace-pre-wrap text-tyba-text-muted">
                      {value === "" ? (
                        <span className="text-tyba-text-ghost">{t("maestroImportEmptyText")}</span>
                      ) : (
                        revealHidden(value)
                      )}
                    </dd>
                  </div>
                ))}
              </dl>
            )}
          </Section>

          <Section title={t("maestroImportNodes", { count: items.length })}>
            {items.map((item) => {
              const info = dirOf.get(item.node.id);
              return (
                <ItemView
                  key={item.node.id}
                  item={item}
                  dir={
                    info && (
                      <DirRow
                        info={info}
                        chosen={chosen[info.nodeId]}
                        onPick={(dir) => {
                          rememberDir(dir);
                          setChosen((c) => ({ ...c, [info.nodeId]: dir }));
                        }}
                      />
                    )
                  }
                />
              );
            })}
          </Section>

          {unused.length > 0 && (
            <Section title={t("maestroImportUnused")}>
              {unused.map((i) => (
                <div key={i.id} className="flex flex-col gap-1.5">
                  <IntegrationBody integration={i} />
                </div>
              ))}
            </Section>
          )}
        </div>
        {error && (
          <p
            role="alert"
            className="tyba-divide-b flex items-start gap-2 bg-tyba-red-tint px-4 py-2 text-[11.5px] text-tyba-red"
          >
            <Warning size={13} className="mt-px shrink-0" />
            {error}
          </p>
        )}
        <div className="flex items-center justify-end gap-2 px-4 py-3">
          <span
            className={`mr-auto text-[11px] ${
              pending.length > 0 ? "text-tyba-amber" : "text-tyba-text-faint"
            }`}
          >
            {pending.length > 0
              ? t("maestroImportDirsPending", { count: pending.length })
              : t("maestroImportNothingRuns")}
          </span>
          <Button variant="ghost" size="sm" disabled={busy} onClick={onCancel}>
            {t("cancel")}
          </Button>
          <Button
            size="sm"
            disabled={busy || pending.length > 0}
            onClick={() => void confirm()}
          >
            {t("confirm")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
