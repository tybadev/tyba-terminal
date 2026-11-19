import { Lock } from "@phosphor-icons/react";

/**
 * As peças das Configurações, no desenho do quadro "Configurações (aprovada)":
 * grupo = título curto em caixa normal, sem caixa em volta; linha = nome e
 * explicação à esquerda, controle sempre à vista à direita, divisória fina
 * embaixo (a última do grupo não tem).
 */

export function SettingGroup({
  title,
  note,
  children,
}: {
  title: string;
  note?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <section className="mt-9">
      <div className="mb-1 flex items-baseline gap-3">
        <h2 className="m-0 text-[15px] leading-[22px] font-bold tracking-[-.01em] text-tyba-text">
          {title}
        </h2>
        {note && <span className="ml-auto shrink-0 text-[12.5px] text-tyba-text-faint">{note}</span>}
      </div>
      {children !== undefined && <div className="[&>*:last-child]:border-b-0">{children}</div>}
    </section>
  );
}

export function SettingLine({
  label,
  hint,
  children,
  below,
}: {
  label?: React.ReactNode;
  hint?: React.ReactNode;
  children?: React.ReactNode;
  /** Controle largo demais para a coluna da direita: vai embaixo, na largura toda. */
  below?: React.ReactNode;
}) {
  return (
    <div
      data-setting-row
      className="flex flex-wrap items-center gap-x-8 gap-y-3 border-b border-[var(--tyba-line)] py-[13px]"
    >
      <div className="flex min-w-[240px] flex-1 basis-[240px] flex-col gap-0.5">
        {label && <span className="text-[14px] leading-5 font-semibold text-tyba-text">{label}</span>}
        {hint && (
          <span className="max-w-[420px] break-words text-[13px] leading-[19px] text-tyba-text-muted">
            {hint}
          </span>
        )}
      </div>
      {children && <div className="ml-auto flex shrink-0 items-center justify-end gap-2">{children}</div>}
      {below && <div className="w-full">{below}</div>}
    </div>
  );
}

/** Texto solto dentro de um grupo, na mesma margem das linhas. */
export function SettingNote({
  children,
  tone = "faint",
}: {
  children: React.ReactNode;
  tone?: "faint" | "muted" | "amber";
}) {
  const color =
    tone === "amber" ? "text-tyba-amber" : tone === "muted" ? "text-tyba-text-muted" : "text-tyba-text-faint";
  return (
    <div className={`border-b border-[var(--tyba-line)] py-2.5 text-[12.5px] leading-[18px] ${color}`}>
      {children}
    </div>
  );
}

/** "● Conectado" ao lado de um controle: o estado de agora, não a escolha. */
export function SettingStatus({ tone, children }: { tone: "green" | "faint" | "amber" | "red"; children: React.ReactNode }) {
  const color = {
    green: "text-tyba-green",
    faint: "text-tyba-text-faint",
    amber: "text-tyba-amber",
    red: "text-tyba-red",
  }[tone];
  return (
    <span className={`inline-flex items-center gap-[7px] text-[13px] font-semibold ${color}`}>
      <i className="block size-2 rounded-full bg-current" aria-hidden />
      {children}
    </span>
  );
}

/** Regra que nenhuma configuração desliga: cadeado + "Fixo", sem controle. */
export function SettingFixed({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-[13px] font-semibold text-tyba-text-faint">
      <Lock size={14} />
      {children}
    </span>
  );
}

/** Controle segmentado do quadro: trilho de tinta, placa acesa no escolhido. */
export function SettingSegment<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: { value: T; label: React.ReactNode }[];
  value: T | null;
  onChange: (value: T) => void;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="tyba-segment rounded-full! p-1!">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          onClick={() => onChange(o.value)}
          className={`flex h-[30px] items-center gap-2 rounded-full px-4 text-[13px] font-semibold whitespace-nowrap transition-colors focus-visible:outline-none focus-visible:[box-shadow:var(--tyba-focus-ring)] ${
            o.value === value ? "tyba-pill-on text-tyba-text" : "text-tyba-text-muted hover:text-tyba-text"
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

const BUTTON_TONE = {
  glass:
    "bg-[var(--tyba-glass-piece)] text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-line)] hover:bg-tyba-hover",
  ghost: "text-tyba-text-muted hover:bg-tyba-hover hover:text-tyba-text",
  danger:
    "bg-[var(--tyba-glass-piece)] text-tyba-red shadow-[inset_0_0_0_1px_var(--tyba-line)] hover:bg-tyba-red-tint",
} as const;

/** Botão pequeno do quadro (glass, ghost ou danger), com ícone opcional. */
export function SettingButton({
  tone = "glass",
  icon,
  className = "",
  children,
  ...props
}: React.ComponentProps<"button"> & {
  tone?: keyof typeof BUTTON_TONE;
  icon?: React.ReactNode;
}) {
  return (
    <button
      type="button"
      {...props}
      className={`inline-flex h-[34px] shrink-0 items-center justify-center gap-2 rounded-[10px] px-3.5 text-[13px] font-semibold whitespace-nowrap transition-colors outline-none focus-visible:[box-shadow:var(--tyba-focus-ring)] disabled:cursor-not-allowed disabled:opacity-45 ${BUTTON_TONE[tone]} ${className}`}
    >
      {icon}
      {children}
    </button>
  );
}

/** Campo de texto e select do quadro: 230px, cantos de 10px, fio fino. */
export const SETTING_FIELD =
  "h-9 w-[230px] rounded-[10px] border-transparent bg-[var(--tyba-glass-piece)] shadow-[0_0_0_1px_var(--tyba-line)] hover:border-transparent";
