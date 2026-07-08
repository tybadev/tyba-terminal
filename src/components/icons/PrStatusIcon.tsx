import {
  GitMerge,
  GitPullRequest,
  Prohibit,
  type Icon,
} from "@phosphor-icons/react";

import { cn } from "@/lib/utils";

export type PrStatus = "draft" | "open" | "merged" | "closed";

const STATUS_ICONS: Record<PrStatus, { icon: Icon; className: string }> = {
  draft: { icon: GitPullRequest, className: "text-tyba-text-muted" },
  open: { icon: GitPullRequest, className: "text-tyba-green" },
  merged: { icon: GitMerge, className: "text-tyba-violet" },
  closed: { icon: Prohibit, className: "text-tyba-red" },
};

interface Props {
  status: PrStatus;
  size?: number;
  className?: string;
}

export function PrStatusIcon({ status, size = 16, className }: Props) {
  const { icon: StatusIcon, className: statusClass } = STATUS_ICONS[status];
  return <StatusIcon size={size} className={cn(statusClass, className)} />;
}
