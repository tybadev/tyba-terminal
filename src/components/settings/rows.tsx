/**
 * As peças das Configurações, no desenho dos quadros "Configurações · …" do
 * canvas: grupo = título em caixa alta que corre num filete até a nota; linha
 * = nome e descrição à esquerda, controle à direita, filete embaixo.
 */

export function SettingGroup({ title, note }: { title: string; note?: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2.5 px-[18px] pt-4 pb-1.5 text-[11px] tracking-[.06em] text-tyba-text-faint first:pt-1">
      <span className="uppercase">{title}</span>
      <span className="h-px min-w-0 flex-1 bg-tyba-border-soft" />
      {note && <span className="shrink-0 tracking-normal">{note}</span>}
    </div>
  );
}

export function SettingLine({
  label,
  hint,
  children,
  below,
}: {
  label: React.ReactNode;
  hint?: React.ReactNode;
  children?: React.ReactNode;
  /** Controle largo demais para a coluna da direita: vai embaixo, na largura toda. */
  below?: React.ReactNode;
}) {
  return (
    <div
      data-setting-row
      className="grid grid-cols-[1fr_auto] items-center gap-x-6 gap-y-3 border-b border-tyba-border-soft px-[18px] py-[11px]"
    >
      <div className="flex min-w-0 flex-col gap-[3px]">
        <span className="text-[13px] text-tyba-text">{label}</span>
        {hint && (
          <span className="break-words text-[11.5px] leading-4 text-tyba-text-faint">{hint}</span>
        )}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2.5">{children}</div>}
      {below && <div className="col-span-2">{below}</div>}
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
    <div className={`border-b border-tyba-border-soft px-[18px] py-2.5 text-[12px] leading-[18px] ${color}`}>
      {children}
    </div>
  );
}

/** "● rodando" ao lado de um interruptor: o estado de agora, não a escolha. */
export function SettingStatus({ tone, children }: { tone: "green" | "faint" | "amber" | "red"; children: React.ReactNode }) {
  const color = {
    green: "text-tyba-green",
    faint: "text-tyba-text-faint",
    amber: "text-tyba-amber",
    red: "text-tyba-red",
  }[tone];
  return <span className={`text-[11px] ${color}`}>● {children}</span>;
}
