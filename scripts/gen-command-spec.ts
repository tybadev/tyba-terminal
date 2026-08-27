/**
 * Gera `src-tauri/src/session/command_spec.tsv` a partir dos specs do Fig.
 *
 * Roda à mão, não no build: a base é **artefato versionado**, não algo que se
 * baixa a cada compilação. Build que busca na rede quebra sem internet, muda de
 * resultado entre duas execuções e transforma um servidor de terceiro em
 * dependência de release. O TSV é commitado; este script existe para
 * regenerá-lo quando alguém decidir.
 *
 *   bun scripts/gen-command-spec.ts
 *
 * A escolha dos comandos está na tech-spec 03 e saiu de medição do histórico
 * real do dono, não de palpite: a base do Fig cobre 20 dos 22 mais usados, e
 * entram os que têm superfície de SUBCOMANDO — `cd`, `ls`, `cp` já têm
 * completação de caminho e quase não têm flag que se procure.
 */
import { extrai, type Linha } from "./figExtract";

/** Versão fixada. Base é artefato datado; "latest" faria o TSV mudar sozinho. */
const VERSAO = "2.692.3";

/** Ver a tech-spec 03 para o porquê de cada um — e de `sudo` ficar fora. */
const COMANDOS = [
  "git",
  "docker",
  "pnpm",
  "bun",
  "yarn",
  "npm",
  "cargo",
  "make",
  "brew",
  "ssh",
  "hx",
  "bat",
  "curl",
  "gh",
  "kubectl",
];

const alvo = new URL("../src-tauri/src/session/command_spec.tsv", import.meta.url);
const cache = new URL("../.fig-cache/", import.meta.url);

/** TAB e quebra de linha são o formato; um dentro do texto o destruiria. */
function limpo(texto: string): string {
  return texto.replace(/[\t\r\n]+/g, " ").trim();
}

async function baixa(cmd: string): Promise<string | null> {
  const arquivo = new URL(`${cmd}.js`, cache);
  const local = Bun.file(arquivo);
  if (await local.exists()) return arquivo.pathname;
  const url = `https://unpkg.com/@withfig/autocomplete@${VERSAO}/build/${cmd}.js`;
  const resposta = await fetch(url, { redirect: "follow" });
  if (!resposta.ok) return null;
  await Bun.write(arquivo, await resposta.text());
  return arquivo.pathname;
}

const linhas: Linha[] = [];
const faltando: string[] = [];

for (const cmd of COMANDOS) {
  const caminho = await baixa(cmd);
  if (!caminho) {
    faltando.push(cmd);
    continue;
  }
  const mod = await import(caminho);
  const doComando = extrai(mod.default);
  linhas.push(...doComando);
  console.log(`  ${cmd.padEnd(10)} ${String(doComando.length).padStart(5)} entradas`);
}

const tsv = linhas
  .map((l) =>
    [limpo(l.command), limpo(l.path), l.kind, limpo(l.description ?? "")].join("\t"),
  )
  .join("\n");

await Bun.write(alvo, tsv + "\n");

console.log(`\n${linhas.length} entradas · ${(tsv.length / 1024).toFixed(0)} KB`);
console.log(`fonte: @withfig/autocomplete@${VERSAO}`);
if (faltando.length) console.log(`sem spec na base: ${faltando.join(", ")}`);
