import type { ReactNode } from "react";

/** EmptyState do Muriki (bundle `.mk-empty`): ícone num quadrado azul suave,
 *  título, texto e ações opcionais, centralizado. */
export function EmptyState({
  icon,
  title,
  description,
  actions,
}: {
  icon: ReactNode;
  title: string;
  description?: string;
  actions?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center gap-3 px-6 py-8 text-center">
      <span className="mb-2 grid size-14 place-items-center rounded-[16px] bg-tyba-blue/10 text-tyba-blue [&_svg]:size-[26px]">
        {icon}
      </span>
      <h3 className="m-0 text-[18px] font-bold leading-6 tracking-[-0.01em] text-tyba-text">{title}</h3>
      {description && <p className="m-0 max-w-[380px] text-[14px] leading-[21px] text-tyba-text-muted">{description}</p>}
      {actions && <div className="mt-3 flex gap-2">{actions}</div>}
    </div>
  );
}
