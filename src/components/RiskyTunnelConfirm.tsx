import { useTranslation } from "react-i18next";
import { ShieldWarning } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import type { Tunnel } from "@/lib/ipc";

interface Props {
  tunnels: Tunnel[];
  host: string;
  confirmLabel: string;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function RiskyTunnelConfirm({
  tunnels,
  host,
  confirmLabel,
  busy,
  onConfirm,
  onCancel,
}: Props) {
  const { t } = useTranslation();
  return (
    <div className="rounded-[5px] border border-tyba-red/50 bg-tyba-red/5 p-2">
      <div className="flex items-center gap-1.5">
        <ShieldWarning size={13} className="shrink-0 text-tyba-red" />
        <span className="text-[12px] text-tyba-text">
          {t("tunnelsConfirmTitle")}
        </span>
      </div>
      {tunnels.map((tn, i) => (
        <p
          key={`${tn.kind}-${tn.listen_port}-${i}`}
          className="mt-1 text-[11px] text-tyba-text-muted"
        >
          {tn.kind === "remote"
            ? t("tunnelsConfirmRemote", { host, port: tn.listen_port })
            : t("tunnelsConfirmDynamic", { host })}
        </p>
      ))}
      <div className="mt-2 flex justify-end gap-1.5">
        <Button size="sm" variant="ghost" onClick={onCancel} disabled={busy}>
          {t("tunnelsConfirmNo")}
        </Button>
        <Button
          size="sm"
          variant="destructive"
          onClick={onConfirm}
          disabled={busy}
        >
          {confirmLabel}
        </Button>
      </div>
    </div>
  );
}
