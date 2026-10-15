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
import { enxerta, extrai, semRepetir, type Linha } from "./figExtract";

/**
 * Versão fixada. Base é artefato datado; "latest" faria o TSV mudar sozinho.
 *
 * Licença MIT, Copyright (c) 2021 Hercules Labs Inc. (Fig) — o aviso viaja
 * junto em `THIRD_PARTY_NOTICES.md`, como a licença exige.
 */
const VERSAO = "2.692.3";

/**
 * Ver a tech-spec 03 para o porquê de cada um — e de `sudo` ficar fora.
 *
 * `ls`, `cat`, `grep` e `find` entraram em 2026-10-12, depois de o dono dizer
 * que faltavam comandos: eles têm completação de caminho, mas as flags (`ls
 * -lah`, `grep -rn`, `find -name`) são justamente o que se procura.
 */
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
  "ls",
  "cat",
  "grep",
  "find",
  // Sistema e ferramentas do dia a dia (2026-10-14, S2 da v2): o dono pediu
  // ajuda também fora do git — `tar -xzf`, `rsync -avz`, `chmod +x`, `kill -9`
  // são exatamente as flags que ninguém lembra.
  "rm",
  "cp",
  "mv",
  "mkdir",
  "touch",
  "ln",
  "chmod",
  "chown",
  "tar",
  "zip",
  "unzip",
  "du",
  "df",
  "ps",
  "kill",
  "pkill",
  "killall",
  "lsof",
  "head",
  "tail",
  "less",
  "sed",
  "sort",
  "wc",
  "diff",
  "xargs",
  "tree",
  "which",
  "open",
  "rsync",
  "scp",
  "ssh-keygen",
  "ping",
  "dig",
  "wget",
  "jq",
  "code",
  "node",
  "npx",
  "python",
  "python3",
  "pip",
  "pip3",
  "uv",
  "go",
  "rustup",
  "launchctl",
  "defaults",
];

/**
 * Specs que entram PENDURADOS em outro comando: `[spec, comando, subcomando]`.
 * Ver `enxerta`.
 */
const ENXERTOS: [string, string, string][] = [["docker-compose", "docker", "compose"]];

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

for (const [spec, comando, sob] of ENXERTOS) {
  const caminho = await baixa(spec);
  if (!caminho) {
    faltando.push(spec);
    continue;
  }
  const mod = await import(caminho);
  const enxertadas = enxerta(extrai(mod.default), comando, sob);
  linhas.push(...enxertadas);
  console.log(`  ${`${comando} ${sob}`.padEnd(10)} ${String(enxertadas.length).padStart(5)} entradas (de ${spec})`);
}

// Linhas de OUTRAS fontes ficam: o `openssl` não vem do Fig (ver
// `gen-openssl-spec.ts`), e regenerar não pode apagá-lo em silêncio.
const geradas = new Set(linhas.map((l) => l.command));
const anterior = Bun.file(alvo);
const preservadas: string[] = (await anterior.exists())
  ? (await anterior.text())
      .split("\n")
      .filter((l) => l && !geradas.has(l.split("\t")[0]))
  : [];

const tsv = semRepetir(linhas)
  .map((l) =>
    [limpo(l.command), limpo(l.path), l.kind, limpo(l.description ?? "")].join("\t"),
  )
  .concat(preservadas)
  .join("\n");

await Bun.write(alvo, tsv + "\n");
if (preservadas.length) console.log(`  (${preservadas.length} linhas de outras fontes preservadas)`);

console.log(`\n${linhas.length} entradas · ${(tsv.length / 1024).toFixed(0)} KB`);
console.log(`fonte: @withfig/autocomplete@${VERSAO}`);
if (faltando.length) console.log(`sem spec na base: ${faltando.join(", ")}`);
