import { cn } from "@/lib/utils";
import { comboKeys } from "@/lib/keys";

// Tecla do Muriki: chapada, um véu do texto. Nos temas escuros fica o
// relevo de tecla de antes (fio e sombra embaixo).
function Kbd({ className, children }: { className?: string; children: React.ReactNode }) {
  return (
    <kbd
      className={cn(
        "inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-[4px] bg-tyba-text/[.06] px-1.5 font-mono text-[11px] leading-none font-medium text-tyba-text-faint dark:border dark:border-tyba-border-strong/60 dark:bg-tyba-text/[.05] dark:px-1 dark:text-[10px] dark:text-tyba-text-muted dark:[box-shadow:inset_0_-1px_0_rgba(0,0,0,0.35)]",
        className,
      )}
    >
      {children}
    </kbd>
  );
}

function Shortcut({
  combo,
  className,
  joined,
}: {
  combo: string;
  className?: string;
  /** Uma tecla só com o atalho inteiro ("⌘P"), como a barra de status do quadro Shell. */
  joined?: boolean;
}) {
  if (joined) return <Kbd className={className}>{comboKeys(combo).join("")}</Kbd>;
  return (
    <span className={cn("inline-flex items-center gap-0.5", className)}>
      {comboKeys(combo).map((key, i) => (
        <Kbd key={i}>{key}</Kbd>
      ))}
    </span>
  );
}

export { Kbd, Shortcut };
