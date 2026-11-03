import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy, IdentificationBadge, Plus, Trash, Warning, X } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  MAX_ROLE_INSTRUCTIONS_CHARS,
  MAX_ROLE_NAME_CHARS,
  copyName,
  deleteRole,
  renameRole,
  roleProblem,
  roleUsage,
  saveRole,
  type Graph,
  type OrbitNode,
  type Role,
} from "@/lib/orbit";
import { ConfirmDelete } from "./ConfirmDelete";

/** O papel em edição e, se já gravado, o nome com que está no banco. */
interface Draft {
  role: Role;
  savedName: string | null;
}

const label = "block text-[11px] text-tyba-text-faint";
const field =
  "w-full rounded-[6px] border border-tyba-border-strong bg-tyba-sunken px-2.5 py-[6px] font-mono text-[12px] text-tyba-text outline-none focus:border-tyba-text-faint";

/**
 * Gaveta de Papéis (quadro OrbitPapeis B), ao lado de Integrações: a
 * biblioteca desta máquina à esquerda (com o uso em nós), o editor da
 * selecionada à direita. Renomear leva os nós de todos os grafos junto;
 * apagar deixa o rótulo nos nós. Gravar é do core (`orbit_save_role`,
 * `orbit_rename_role`).
 */
export function RolesPanel({
  roles,
  graphs,
  currentGraphId,
  currentNodes,
  start,
  onSaved,
  onRenamed,
  onDeleted,
  onClose,
}: {
  roles: readonly Role[];
  /** Todos os grafos desta máquina: o uso é por máquina. */
  graphs: readonly Graph[];
  /** O grafo aberto, com os nós como estão no canvas (podem diferir do salvo). */
  currentGraphId: string;
  currentNodes: readonly OrbitNode[];
  /** "Novo papel…" do card: abre já num papel novo com esse nome. */
  start: { newName?: string } | null;
  onSaved: (role: Role) => void;
  onRenamed: (from: string, to: string) => void;
  onDeleted: (name: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<Draft | null>(() => {
    if (start?.newName !== undefined) {
      return {
        role: { name: start.newName || t("orbitRoleNewName"), instructions: "" },
        savedName: null,
      };
    }
    return roles[0] ? { role: roles[0], savedName: roles[0].name } : null;
  });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [askDelete, setAskDelete] = useState(false);

  // Lista recarregada (outra gravação, importação): sem nada aberto, abre a primeira.
  useEffect(() => {
    if (!draft && roles[0]) setDraft({ role: roles[0], savedName: roles[0].name });
  }, [draft, roles]);

  const saved = draft?.savedName ? roles.find((r) => r.name === draft.savedName) : undefined;
  const dirty = !!draft && JSON.stringify(saved) !== JSON.stringify(draft.role);
  const trimmedName = draft?.role.name.trim() ?? "";
  const nameTaken =
    !!draft && roles.some((r) => r.name === trimmedName && r.name !== draft.savedName);
  const problem = draft ? (roleProblem(draft.role) ?? (nameTaken ? "nameTaken" : null)) : null;
  const usageGraphs = [
    ...graphs.filter((g) => g.id !== currentGraphId),
    { nodes: currentNodes },
  ];
  const usage = (name: string) => roleUsage(usageGraphs, name);

  const select = (next: Draft) => {
    setDraft(next);
    setError(null);
  };

  const save = async () => {
    if (!draft || problem) return;
    const role: Role = { name: trimmedName, instructions: draft.role.instructions };
    setSaving(true);
    try {
      if (draft.savedName && draft.savedName !== role.name) {
        await renameRole(draft.savedName, role.name);
        onRenamed(draft.savedName, role.name);
      }
      await saveRole(role);
      onSaved(role);
      setDraft({ role, savedName: role.name });
      setError(null);
    } catch (e) {
      setError(t("orbitSaveError", { error: String(e) }));
    } finally {
      setSaving(false);
    }
  };

  const duplicate = () => {
    if (!draft) return;
    select({
      role: {
        name: copyName(trimmedName || t("orbitRoleNewName"), roles.map((r) => r.name)),
        instructions: draft.role.instructions,
      },
      savedName: null,
    });
  };

  const item =
    "flex flex-col gap-[5px] rounded-[7px] border border-transparent px-2.5 py-2 text-left text-[12px] leading-4 text-tyba-text hover:bg-tyba-raised aria-[current=true]:border-tyba-border-strong aria-[current=true]:bg-tyba-raised";
  const count = [...(draft?.role.instructions ?? "")].length;
  const used = draft?.savedName ? usage(draft.savedName) : { nodes: 0, graphs: 0 };

  return (
    <aside
      id="orbit-roles"
      data-orbit-sheet
      role="dialog"
      aria-labelledby="orbit-roles-title"
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
      }}
      className="absolute top-0 right-0 bottom-0 z-[11] flex w-[580px] max-w-full flex-col border-l border-tyba-border-strong bg-tyba-surface font-mono shadow-[-30px_0_60px_rgba(0,0,0,0.55)]"
    >
      <div className="flex h-14 shrink-0 items-center gap-3 border-b border-tyba-border pr-2.5 pl-5">
        <span className="flex size-6 shrink-0 items-center justify-center rounded-[6px] bg-tyba-violet/15 text-tyba-violet">
          <IdentificationBadge size={14} weight="bold" />
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span id="orbit-roles-title" className="text-[13.5px] font-medium text-tyba-text">
            {t("orbitRoles")}
          </span>
          <span className="truncate text-[11px] text-tyba-text-faint">{t("orbitRolesSubtitle")}</span>
        </span>
        <button
          type="button"
          aria-label={t("orbitRolesClose")}
          onClick={onClose}
          className="rounded-[5px] p-1.5 text-tyba-text-faint hover:bg-tyba-raised hover:text-tyba-text"
        >
          <X size={14} />
        </button>
      </div>
      <div className="flex min-h-0 flex-1">
        <nav
          aria-label={t("orbitRolesList")}
          className="flex w-44 shrink-0 flex-col gap-[3px] overflow-y-auto border-r border-tyba-border px-2 py-3"
        >
          <span className="px-2.5 pb-1 text-[10px] tracking-[0.08em] text-tyba-text-faint uppercase">
            {t("orbitIntLibrary")}
          </span>
          {roles.map((r) => {
            const u = usage(r.name);
            return (
              <button
                key={r.name}
                type="button"
                aria-current={draft?.savedName === r.name}
                onClick={() => select({ role: r, savedName: r.name })}
                className={item}
              >
                <span className="truncate">{r.name}</span>
                <span className="text-[10.5px] text-tyba-text-faint">
                  {u.nodes === 0 ? t("orbitRoleUnused") : t("orbitRoleNodes", { count: u.nodes })}
                </span>
              </button>
            );
          })}
          {draft && !saved && (
            <button type="button" aria-current className={item}>
              <span className="truncate">{draft.role.name || t("orbitRoleNewName")}</span>
              <span className="text-[10.5px] text-tyba-amber">{t("orbitRoleUnsaved")}</span>
            </button>
          )}
          <button
            type="button"
            onClick={() =>
              select({
                role: { name: t("orbitRoleNewName"), instructions: "" },
                savedName: null,
              })
            }
            className="flex items-center gap-2 rounded-[7px] px-2.5 py-2 text-left text-[12px] text-tyba-text-muted hover:bg-tyba-raised hover:text-tyba-text"
          >
            <Plus size={12} />
            {t("orbitRoleNew")}
          </button>
        </nav>
        {draft ? (
          <form
            key={draft.savedName ?? "new"}
            aria-label={t("orbitRoleEdit", { name: draft.role.name })}
            onSubmit={(e) => {
              e.preventDefault();
              void save();
            }}
            className="flex min-w-0 flex-1 flex-col gap-3.5 overflow-y-auto px-[18px] py-4 font-mono"
          >
            <div className="flex flex-col gap-1.5">
              <label className={label} htmlFor="orbit-role-name">
                {t("orbitRoleName", { max: MAX_ROLE_NAME_CHARS })}
              </label>
              <input
                id="orbit-role-name"
                type="text"
                spellCheck={false}
                autoCorrect="off"
                autoCapitalize="off"
                value={draft.role.name}
                onChange={(e) =>
                  setDraft({ ...draft, role: { ...draft.role, name: e.target.value } })
                }
                className={field}
              />
            </div>
            <div className="flex min-h-0 flex-1 flex-col gap-1.5">
              <label className={label} htmlFor="orbit-role-instructions">
                {t("orbitRoleInstructions")}
              </label>
              <textarea
                id="orbit-role-instructions"
                spellCheck={false}
                autoCorrect="off"
                autoCapitalize="off"
                value={draft.role.instructions}
                onChange={(e) =>
                  setDraft({ ...draft, role: { ...draft.role, instructions: e.target.value } })
                }
                className={`${field} min-h-[200px] flex-1 resize-none leading-[1.6]`}
              />
              <span className="flex items-baseline justify-between gap-2 text-[10.5px] text-tyba-text-faint">
                <span className={count > MAX_ROLE_INSTRUCTIONS_CHARS ? "text-tyba-red" : ""}>
                  {t("orbitRoleCounter", { count, max: MAX_ROLE_INSTRUCTIONS_CHARS })}
                </span>
                <span>
                  {draft.savedName === null || used.nodes === 0
                    ? t("orbitRoleUnused")
                    : t("orbitRoleUsedIn", {
                        nodes: t("orbitRoleNodes", { count: used.nodes }),
                        graphs: t("orbitRoleGraphs", { count: used.graphs }),
                      })}
                </span>
              </span>
            </div>
            {(problem || error) && (
              <p
                role="alert"
                className="flex items-start gap-2 rounded-[6px] border border-tyba-red/50 bg-tyba-red-tint px-3 py-2 text-[11.5px] text-tyba-red"
              >
                <Warning size={13} className="mt-px shrink-0" />
                {problem
                  ? t(`orbitRoleProblem_${problem}`, {
                      max:
                        problem === "instructionsTooLong"
                          ? MAX_ROLE_INSTRUCTIONS_CHARS
                          : MAX_ROLE_NAME_CHARS,
                    })
                  : error}
              </p>
            )}
            <div className="mt-auto flex items-center gap-2 pt-1">
              <button
                type="button"
                onClick={duplicate}
                className="inline-flex items-center gap-[7px] rounded-[6px] border border-tyba-border-strong px-3 py-[7px] text-[12px] whitespace-nowrap text-tyba-text hover:border-tyba-text-muted"
              >
                <Copy size={12} />
                {t("orbitRoleDuplicate")}
              </button>
              {saved && (
                <button
                  type="button"
                  onClick={() => setAskDelete(true)}
                  className="inline-flex items-center gap-[7px] rounded-[6px] px-2.5 py-[7px] text-[12px] whitespace-nowrap text-tyba-text-muted hover:text-tyba-red"
                >
                  <Trash size={12} />
                  {t("orbitRoleDelete")}
                </button>
              )}
              <span className="flex-1" />
              <Button type="submit" size="sm" disabled={!!problem || saving || !dirty}>
                {t("orbitRoleSave")}
              </Button>
            </div>
          </form>
        ) : (
          <div className="flex flex-1 items-center justify-center px-6 text-center text-[12px] text-tyba-text-faint">
            {t("orbitRolesEmpty")}
          </div>
        )}
      </div>
      {askDelete && saved && (
        <ConfirmDelete
          title={t("orbitRoleDeleteTitle", { name: saved.name })}
          body={t("orbitRoleDeleteBody")}
          onConfirm={async () => {
            await deleteRole(saved.name);
            onDeleted(saved.name);
            setDraft(null);
          }}
          onClose={() => setAskDelete(false)}
        />
      )}
    </aside>
  );
}
