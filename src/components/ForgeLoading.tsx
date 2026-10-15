import { useTranslation } from "react-i18next";

import { LoadingState, type LoadingStateProps } from "./ui/loading";

/**
 * Carregamento da forja (PRs, CI, comentários): é sempre um `gh`/`glab` na
 * rede, então a demora tem o mesmo motivo provável em todo lugar.
 */
export function ForgeLoading(props: Omit<LoadingStateProps, "op" | "slowCause">) {
  const { t } = useTranslation();
  return <LoadingState op="forge" slowCause={t("loadForgeSlow")} {...props} />;
}
