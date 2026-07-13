import type { ConflictState } from "./ipc";

const OPERATION_NAME: Record<ConflictState["operation"], string> = {
  merge: "merge",
  rebase: "rebase",
  cherry_pick: "cherry-pick",
};

export function buildConflictPrompt(state: ConflictState): string {
  const op = OPERATION_NAME[state.operation];
  const sides =
    state.ours && state.theirs ? ` (${state.ours} ← ${state.theirs})` : "";
  const files = state.files
    .map((f) => `- ${f.path} (${f.kind})`)
    .join("\n");
  const closing =
    state.operation === "merge"
      ? "Depois de resolver e dar `git add` em cada arquivo, PARE — não conclua o commit do merge. O dono revisa a resolução no painel e commita."
      : `Depois de resolver e dar \`git add\` nos arquivos de cada passo, rode \`git ${OPERATION_NAME[state.operation]} --continue\` até a operação terminar. Não crie commits além dos que a própria operação replica.`;
  return [
    `Tem um ${op} em andamento${sides} com ${state.files.length} arquivo(s) em conflito neste repositório:`,
    "",
    files,
    "",
    "Resolva os conflitos preservando a intenção dos dois lados: leia o contexto de cada arquivo, remova os marcadores de conflito (<<<<<<<, =======, >>>>>>>) e deixe o resultado coerente.",
    closing,
  ].join("\n");
}
