import type { RepoStatus } from "@/lib/ipc";

interface Props {
  status: RepoStatus;
  size?: "sm" | "md";
}

export function DiffStat({ status, size = "sm" }: Props) {
  const { changed, insertions, deletions } = status;
  const text = size === "sm" ? "text-[9px]" : "text-[10px]";
  const hasLines = insertions > 0 || deletions > 0;

  return (
    <span className={`flex shrink-0 items-center gap-1 font-mono ${text} leading-none`}>
      <span className="text-tyba-amber">{changed}</span>
      {hasLines && (
        <>
          <span className="text-tyba-green">+{insertions}</span>
          <span className="text-tyba-red">-{deletions}</span>
        </>
      )}
    </span>
  );
}
