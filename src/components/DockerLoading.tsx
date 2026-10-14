import { useTranslation } from "react-i18next";

import { LoadingState, type SkeletonShape } from "./ui/loading";
import { dockerOpenDesktop } from "../lib/ipc";

/**
 * Carregamento do Docker: a etapa é o comando que o core está rodando, e onde.
 * Local e demorando: o Docker Desktop costuma estar subindo — abrir ajuda.
 * No servidor: é a conexão, e a ação é tentar de novo.
 */
export function DockerLoading({
  host,
  cmd,
  startedAt,
  error,
  onRetry,
  shape = "list",
  rows,
  className,
}: {
  host: string | null;
  /** O que vem depois de `docker` (`ps -a`, `image ls`…). */
  cmd: string;
  startedAt: number;
  error?: string | null;
  onRetry?: () => void;
  shape?: SkeletonShape | null;
  rows?: number;
  className?: string;
}) {
  const { t } = useTranslation();
  return (
    <LoadingState
      op="docker"
      remote={host !== null}
      step={host ? t("loadDockerRemote", { host, cmd }) : t("loadDockerLocal", { cmd })}
      em={host ? [host, `docker ${cmd}`] : [`docker ${cmd}`]}
      startedAt={startedAt}
      slowCause={host ? t("loadRemoteSlow", { host }) : t("loadDockerSlowLocal")}
      slowAction={
        host
          ? undefined
          : {
              label: t("dockerPanelOpenDesktop"),
              onClick: () => void dockerOpenDesktop().catch(() => {}),
            }
      }
      error={error}
      onRetry={onRetry}
      shape={shape}
      rows={rows}
      className={className}
    />
  );
}
