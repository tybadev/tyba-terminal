import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CaretDown, Check, GearSix, Plus } from "@phosphor-icons/react";

import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { roleSummary, type Role } from "@/lib/orbit";

type Item =
  | { kind: "role"; role: Role }
  /** O que foi digitado e não está na biblioteca: vira só rótulo, como antes. */
  | { kind: "label"; name: string }
  | { kind: "none" }
  | { kind: "new" }
  | { kind: "manage" };

/**
 * O papel do nó de agente (quadro OrbitPapeis A), no lugar do campo de texto
 * de antes: a escolha na biblioteca desta máquina, com "Sem papel", "Novo
 * papel…" e "Gerenciar papéis…" no fim. O nome é a chave; um nome digitado
 * que não está na biblioteca fica só como rótulo. Trocar o papel de nó
 * rodando marca "↻ para aplicar" (o core compara o nó salvo).
 */
export function RolePicker({
  value,
  roles,
  onChange,
  onNew,
  onManage,
}: {
  value: string;
  roles: readonly Role[];
  onChange: (role: string) => void;
  /** "Novo papel…": abre o painel já com o que foi digitado como nome. */
  onNew: (name: string) => void;
  onManage: () => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const current = value.trim();
  const inLibrary = roles.some((r) => r.name === current);
  const q = query.trim();
  const lower = q.toLowerCase();
  const list = useMemo(
    () =>
      roles.filter(
        (r) =>
          !lower ||
          r.name.toLowerCase().includes(lower) ||
          roleSummary(r.instructions).toLowerCase().includes(lower),
      ),
    [roles, lower],
  );
  const items = useMemo((): Item[] => {
    const typed = q !== "" && !roles.some((r) => r.name.toLowerCase() === lower);
    return [
      ...list.map((role): Item => ({ kind: "role", role })),
      ...(typed ? [{ kind: "label", name: q } as Item] : []),
      { kind: "none" },
      { kind: "new" },
      { kind: "manage" },
    ];
  }, [list, roles, q, lower]);
  const idx = Math.min(active, items.length - 1);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setActive(0);
    const id = window.setTimeout(() => input.current?.focus(), 0);
    return () => window.clearTimeout(id);
  }, [open]);
  useEffect(() => setActive(0), [query]);

  const pick = (item: Item) => {
    switch (item.kind) {
      case "role":
        onChange(item.role.name);
        break;
      case "label":
        onChange(item.name);
        break;
      case "none":
        onChange("");
        break;
      case "new":
        onNew(q);
        break;
      case "manage":
        onManage();
        break;
    }
    setOpen(false);
  };

  const row = (i: number, extra = "") =>
    `flex w-full items-start gap-2 rounded-[6px] px-2.5 py-[7px] text-left ${
      i === idx ? "bg-tyba-violet/10 text-tyba-text" : "text-tyba-text-muted"
    } ${extra}`;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={t("orbitRole")}
          aria-haspopup="listbox"
          aria-expanded={open}
          title={current && !inLibrary ? t("orbitRoleLabelOnly") : undefined}
          className={`nodrag inline-flex h-6 max-w-[150px] min-w-0 items-center gap-1.5 rounded-[5px] border px-2 font-mono text-[11px] ${
            current
              ? "border-tyba-violet/55 bg-tyba-violet/15 text-tyba-violet"
              : "border-tyba-border text-tyba-text-faint hover:text-tyba-text"
          }`}
        >
          <span className="truncate">{current || t("orbitRole")}</span>
          <CaretDown size={10} className="shrink-0" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        sideOffset={6}
        onOpenAutoFocus={(e) => e.preventDefault()}
        className="w-[380px] overflow-hidden rounded-[12px] border-tyba-border-strong bg-tyba-raised p-0 font-mono text-[12px] text-tyba-text shadow-[var(--tyba-shadow-lg)]"
      >
        <div className="flex items-center justify-between border-b border-tyba-border px-3.5 py-2.5">
          <span className="text-[10px] tracking-[0.08em] text-tyba-text-faint uppercase">
            {t("orbitRoleLibraryTitle")}
          </span>
          <span className="text-[11px] text-tyba-text-faint">{roles.length}</span>
        </div>
        <div className="border-b border-tyba-border px-3 py-2">
          <input
            ref={input}
            aria-label={t("orbitRoleSearch")}
            placeholder={t("orbitRoleSearch")}
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setActive((idx + 1) % items.length);
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setActive((idx - 1 + items.length) % items.length);
              } else if (e.key === "Enter") {
                e.preventDefault();
                const item = items[idx];
                if (item) pick(item);
              }
            }}
            className="w-full rounded-[6px] border border-tyba-violet/60 bg-tyba-sunken px-2.5 py-1.5 text-tyba-text outline-none placeholder:text-tyba-text-ghost"
          />
        </div>
        <div
          role="listbox"
          aria-label={t("orbitRoleLibraryTitle")}
          className="flex max-h-[260px] flex-col gap-0.5 overflow-y-auto px-1.5 py-1"
        >
          {items.map((item, i) => {
            switch (item.kind) {
              case "role": {
                const selected = item.role.name === current;
                return (
                  <button
                    key={`r-${item.role.name}`}
                    type="button"
                    role="option"
                    aria-selected={selected}
                    onMouseEnter={() => setActive(i)}
                    onClick={() => pick(item)}
                    className={row(i)}
                  >
                    <span className="flex w-4 shrink-0 justify-center pt-[3px] text-tyba-violet">
                      {selected && <Check size={11} weight="bold" />}
                    </span>
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span className="truncate text-tyba-text">{item.role.name}</span>
                      <span className="truncate text-[11px] text-tyba-text-faint">
                        {roleSummary(item.role.instructions)}
                      </span>
                    </span>
                  </button>
                );
              }
              case "label":
                return (
                  <button
                    key="label"
                    type="button"
                    role="option"
                    aria-selected={false}
                    onMouseEnter={() => setActive(i)}
                    onClick={() => pick(item)}
                    className={row(i, "border-t border-tyba-border")}
                  >
                    <span className="w-4 shrink-0" />
                    <span className="min-w-0 flex-1 truncate">
                      {t("orbitRoleUseLabel", { name: item.name })}
                    </span>
                  </button>
                );
              case "none":
                return (
                  <button
                    key="none"
                    type="button"
                    role="option"
                    aria-selected={current === ""}
                    onMouseEnter={() => setActive(i)}
                    onClick={() => pick(item)}
                    className={row(i, "border-t border-tyba-border")}
                  >
                    <span className="flex w-4 shrink-0 justify-center pt-[3px] text-tyba-violet">
                      {current === "" && <Check size={11} weight="bold" />}
                    </span>
                    <span className="min-w-0 flex-1 truncate">{t("orbitRoleNone")}</span>
                  </button>
                );
              case "new":
                return (
                  <button
                    key="new"
                    type="button"
                    role="option"
                    aria-selected={false}
                    onMouseEnter={() => setActive(i)}
                    onClick={() => pick(item)}
                    className={row(i)}
                  >
                    <span className="flex w-4 shrink-0 justify-center pt-[3px]">
                      <Plus size={11} />
                    </span>
                    <span className="min-w-0 flex-1 truncate">{t("orbitRoleNewEllipsis")}</span>
                  </button>
                );
              case "manage":
                return (
                  <button
                    key="manage"
                    type="button"
                    role="option"
                    aria-selected={false}
                    onMouseEnter={() => setActive(i)}
                    onClick={() => pick(item)}
                    className={row(i)}
                  >
                    <span className="flex w-4 shrink-0 justify-center pt-[3px]">
                      <GearSix size={11} />
                    </span>
                    <span className="min-w-0 flex-1 truncate">{t("orbitRoleManage")}</span>
                  </button>
                );
            }
          })}
        </div>
        <div className="border-t border-tyba-border px-3.5 py-2.5 text-[11px] leading-[16px] text-tyba-text-faint">
          {t("orbitRoleHint")}
        </div>
      </PopoverContent>
    </Popover>
  );
}
