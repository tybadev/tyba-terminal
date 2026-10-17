import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { DotsSixVertical, Plus, PlugsConnected, X } from "@phosphor-icons/react";

import {
  INTEGRATION_DRAG_TYPE,
  deleteIntegration,
  integrationNode,
  newIntegration,
  saveIntegration,
  type Integration,
  type OrbitNode,
} from "@/lib/orbit";
import { ConfirmDelete } from "./ConfirmDelete";
import { IntegrationEditor } from "./IntegrationEditor";

/**
 * Gaveta de Integrações (quadro OrbitIntegracao): a biblioteca à esquerda,
 * o editor da selecionada à direita. Arrastar um item para o canvas cria o nó
 * (quem recebe o soltar é a tela). Gravar é do core (`orbit_save_integration`).
 */
export function IntegrationsPanel({
  integrations,
  nodes,
  onSaved,
  onDeleted,
  onTest,
  onClose,
}: {
  integrations: readonly Integration[];
  /** Nós do canvas: o "Testar" usa o `in/` do nó que tem a integração. */
  nodes: readonly OrbitNode[];
  onSaved: (integration: Integration) => void;
  onDeleted: (id: string) => void;
  onTest: (integrationId: string, nodeId: string) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<Integration | null>(() => integrations[0] ?? null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [askDelete, setAskDelete] = useState(false);

  // Lista recarregada (outra gravação, importação): sem nada aberto, abre a primeira.
  useEffect(() => {
    if (!draft && integrations[0]) setDraft(integrations[0]);
  }, [draft, integrations]);

  const saved = draft ? integrations.find((i) => i.id === draft.id) : undefined;
  const dirty = !!draft && JSON.stringify(saved) !== JSON.stringify(draft);
  const testNode = draft ? integrationNode(nodes, draft.id) : null;

  const select = (next: Integration) => {
    setDraft(next);
    setError(null);
  };

  const save = async (): Promise<boolean> => {
    if (!draft) return false;
    setSaving(true);
    try {
      await saveIntegration(draft);
      onSaved(draft);
      setError(null);
      return true;
    } catch (e) {
      setError(t("orbitSaveError", { error: String(e) }));
      return false;
    } finally {
      setSaving(false);
    }
  };

  const test = async () => {
    if (!draft || !testNode) return;
    // O core testa a integração gravada: o que está no editor vai antes.
    if (dirty && !(await save())) return;
    try {
      await onTest(draft.id, testNode.id);
      setError(null);
    } catch (e) {
      setError(t("orbitRunError", { error: String(e) }));
    }
  };

  const item =
    "relative flex flex-col gap-[5px] rounded-[7px] border border-transparent py-2 pr-2.5 pl-6 text-left text-[12px] leading-4 text-tyba-text hover:bg-tyba-raised aria-[current=true]:border-tyba-border-strong aria-[current=true]:bg-tyba-raised";

  return (
    <aside
      id="orbit-integrations"
      data-orbit-sheet
      role="dialog"
      aria-labelledby="orbit-int-title"
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
      }}
      className="absolute top-0 right-0 bottom-0 z-[11] flex w-[580px] max-w-full flex-col border-l border-tyba-border-strong bg-tyba-surface font-mono shadow-[-30px_0_60px_rgba(0,0,0,0.55)]"
    >
      <div className="flex h-14 shrink-0 items-center gap-3 border-b border-tyba-border pr-2.5 pl-5">
        <span className="flex size-6 shrink-0 items-center justify-center rounded-[6px] bg-tyba-blue-tint text-tyba-blue">
          <PlugsConnected size={14} weight="bold" />
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span id="orbit-int-title" className="text-[13.5px] font-medium text-tyba-text">
            {t("orbitIntegrations")}
          </span>
          <span className="text-[11px] text-tyba-text-faint">{t("orbitIntSubtitle")}</span>
        </span>
        <button
          type="button"
          aria-label={t("orbitIntClose")}
          onClick={onClose}
          className="rounded-[5px] p-1.5 text-tyba-text-faint hover:bg-tyba-raised hover:text-tyba-text"
        >
          <X size={14} />
        </button>
      </div>
      <div className="flex min-h-0 flex-1">
        <nav
          aria-label={t("orbitIntList")}
          className="flex w-44 shrink-0 flex-col gap-[3px] overflow-y-auto border-r border-tyba-border px-2 py-3"
        >
          <span className="px-2.5 pb-1 text-[10px] tracking-[0.08em] text-tyba-text-faint uppercase">
            {t("orbitIntLibrary")}
          </span>
          {integrations.map((i) => (
            <button
              key={i.id}
              type="button"
              draggable
              aria-current={draft?.id === i.id}
              title={t("orbitIntDragHint")}
              onClick={() => select(i)}
              onDragStart={(e) => {
                e.dataTransfer.setData(INTEGRATION_DRAG_TYPE, i.id);
                e.dataTransfer.effectAllowed = "copy";
              }}
              className={`${item} cursor-grab active:cursor-grabbing`}
            >
              <DotsSixVertical size={12} className="absolute top-2.5 left-2 text-tyba-text-ghost" />
              <span className="truncate">{i.name}</span>
              <span className="text-[10.5px] text-tyba-text-faint">
                {i.trigger.type === "manual"
                  ? t("orbitIntManualShort")
                  : t(`orbitEdge_${i.trigger.kind}`)}{" "}
                → {t(`orbitEdge_${i.output}`)}
              </span>
            </button>
          ))}
          {draft && !saved && (
            <button type="button" aria-current className={item}>
              <span className="truncate">{draft.name || t("orbitKindIntegration")}</span>
              <span className="text-[10.5px] text-tyba-amber">{t("orbitIntUnsaved")}</span>
            </button>
          )}
          <button
            type="button"
            onClick={() => select(newIntegration(crypto.randomUUID(), t("orbitIntNewName")))}
            className="flex items-center gap-2 rounded-[7px] px-2.5 py-2 text-left text-[12px] text-tyba-text-muted hover:bg-tyba-raised hover:text-tyba-text"
          >
            <Plus size={12} />
            {t("orbitIntNew")}
          </button>
          <span className="flex-1" />
          <div className="flex flex-col gap-1 px-2.5 text-[10.5px] leading-[15px]">
            <span className="text-tyba-cyan">{t("orbitIntDragTitle")}</span>
            <span className="text-tyba-text-faint">{t("orbitIntDragBody")}</span>
          </div>
        </nav>
        {draft ? (
          <IntegrationEditor
            key={draft.id}
            draft={draft}
            onChange={setDraft}
            saving={saving}
            dirty={dirty}
            testNodeTitle={testNode?.title ?? null}
            error={error}
            onSave={() => void save()}
            onTest={() => void test()}
            onDelete={saved ? () => setAskDelete(true) : null}
          />
        ) : (
          <div className="flex flex-1 items-center justify-center px-6 text-center text-[12px] text-tyba-text-faint">
            {t("orbitIntEmpty")}
          </div>
        )}
      </div>
      {askDelete && saved && (
        <ConfirmDelete
          title={t("orbitIntDeleteTitle", { name: saved.name })}
          body={t("orbitIntDeleteBody")}
          onConfirm={async () => {
            await deleteIntegration(saved.id);
            onDeleted(saved.id);
            setDraft(null);
          }}
          onClose={() => setAskDelete(false)}
        />
      )}
    </aside>
  );
}
