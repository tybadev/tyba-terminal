import { cn } from "@/lib/utils";
import { comboKeys } from "@/lib/keys";

function Kbd({ className, children }: { className?: string; children: React.ReactNode }) {
  return (
    <kbd
      className={cn(
        "inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-[4px] border border-tyba-border-strong/60 bg-tyba-text/[.05] px-1 font-mono text-[10px] leading-none text-tyba-text-muted [box-shadow:inset_0_-1px_0_rgba(0,0,0,0.35)]",
        className,
      )}
    >
      {children}
    </kbd>
  );
}

function Shortcut({ combo, className }: { combo: string; className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-0.5", className)}>
      {comboKeys(combo).map((key, i) => (
        <Kbd key={i}>{key}</Kbd>
      ))}
    </span>
  );
}

export { Kbd, Shortcut };
