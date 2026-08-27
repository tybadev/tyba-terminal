/**
 * Gera as linhas de `openssl` para a base de specs.
 *
 * O `openssl` **não existe** na base do Fig — conferido em 26/08/2026, nenhum
 * arquivo entre os 715. E ele é justamente o caso em que descoberta importa
 * mais: ninguém lembra se é `s_client` ou `sclient`, se cifrar é `enc` ou
 * `cipher`.
 *
 * Duas fontes, e a divisão é deliberada:
 *
 * - **os NOMES vêm dos binários** (`openssl help`), porque eles publicam a
 *   lista e assim ela não envelhece por transcrição errada;
 * - **as DESCRIÇÕES vêm escritas à mão**, porque o `openssl help` não as tem.
 *
 * Isto roda UMA VEZ, aqui, e o resultado é commitado. Não é o mesmo que os
 * `generators` do Fig, que o ADR recusou: aqueles executam comando **na máquina
 * do usuário, a cada tecla**. Aqui a execução acontece na máquina de quem gera,
 * o resultado é revisado, e o que chega ao usuário é dado inerte — o mesmo
 * modelo do artefato do Fig.
 *
 *   bun scripts/gen-openssl-spec.ts >> src-tauri/src/session/command_spec.tsv
 */
import { parseHelp, uniao, type Fonte } from "./opensslHelp";

/**
 * As duas implementações, e onde procurar cada uma.
 *
 * `openssl` é dois programas com o mesmo nome: a Apple embarca LibreSSL em
 * `/usr/bin`, o resto do mundo embarca OpenSSL. Gerar a partir de um só faria
 * a base mentir para metade dos usuários — e entre os 13 comandos que só o
 * OpenSSL tem está o `list`, que é o comando de descoberta dele.
 */
const IMPLEMENTACOES = [
  { rotulo: "LibreSSL", caminhos: ["/usr/bin/openssl"] },
  {
    rotulo: "OpenSSL",
    caminhos: [
      "/opt/homebrew/opt/openssl@3/bin/openssl",
      "/usr/local/opt/openssl@3/bin/openssl",
      "/usr/bin/openssl3",
    ],
  },
];

/**
 * O que cada subcomando faz, em uma linha.
 *
 * Escrito à mão porque o binário não fornece, e em pt-BR porque a descrição
 * aparece na lista de completação — que é superfície de UI, e a convenção do
 * repo põe UI em pt-BR.
 *
 * Descrição errada é pior que descrição ausente: ela faz alguém rodar a coisa
 * errada com confiança. O que não se soube afirmar foi conferido no `help` do
 * próprio comando antes de entrar aqui, e o que restou em dúvida ficou de fora.
 *
 * Nome de ALGORITMO não entra: são mais de cem, e "calcula SHA-256" repetido
 * cem vezes é ruído. Quem quer entender vai no `dgst` e no `enc`, que o próprio
 * `help` já aponta.
 */
const DESCRICOES: Record<string, string> = {
  asn1parse: "Inspeciona uma estrutura ASN.1, campo a campo",
  ca: "Opera uma autoridade certificadora: assina e revoga certificados",
  certhash: "Gera os links de hash de um diretório de certificados",
  ciphers: "Lista as suítes de cifra que esta build suporta",
  cmp: "Fala com uma autoridade certificadora pelo protocolo CMP (RFC 4210)",
  cms: "Assina, cifra e verifica mensagens CMS (sucessor do S/MIME)",
  configutl: "Lê e normaliza o arquivo de configuração do OpenSSL",
  crl: "Lê e converte listas de revogação",
  crl2pkcs7: "Empacota uma CRL num PKCS#7",
  dgst: "Calcula resumo (hash) e assina ou verifica com ele",
  dh: "Lê e converte parâmetros Diffie-Hellman",
  dhparam: "Gera parâmetros Diffie-Hellman",
  dsa: "Lê e converte chaves DSA",
  dsaparam: "Gera parâmetros DSA",
  ec: "Lê e converte chaves de curva elíptica",
  ecparam: "Gera parâmetros de curva elíptica",
  enc: "Cifra e decifra um arquivo com chave simétrica",
  engine: "Lista e inspeciona engines de criptografia (obsoleto: use providers)",
  errstr: "Traduz um código de erro do OpenSSL para texto",
  fipsinstall: "Prepara e valida o módulo FIPS",
  gendh: "Gera parâmetros Diffie-Hellman (obsoleto: use dhparam)",
  gendsa: "Gera uma chave DSA a partir de parâmetros",
  genpkey: "Gera uma chave privada de qualquer algoritmo suportado",
  genrsa: "Gera uma chave privada RSA",
  help: "Lista os comandos que esta instalação oferece",
  info: "Mostra caminhos e parâmetros de build desta instalação",
  kdf: "Deriva chave a partir de senha ou segredo, por KDF",
  list: "Lista o que esta build oferece: comandos, algoritmos e providers",
  mac: "Calcula um código de autenticação de mensagem (MAC)",
  nseq: "Converte entre sequência Netscape de certificados e PEM",
  ocsp: "Consulta ou responde status de certificado por OCSP",
  passwd: "Calcula o hash de uma senha no formato do sistema",
  pkcs12: "Lê e escreve arquivos PKCS#12 (.p12 / .pfx)",
  pkcs7: "Lê e converte estruturas PKCS#7",
  pkcs8: "Lê e converte chaves privadas em PKCS#8",
  pkey: "Lê, converte e inspeciona chaves de qualquer algoritmo",
  pkeyparam: "Lê e converte parâmetros de algoritmo",
  pkeyutl: "Assina, verifica, cifra e decifra com uma chave",
  prime: "Testa se um número é primo, ou gera um",
  rand: "Gera bytes aleatórios criptograficamente seguros",
  rehash: "Gera os links de hash de um diretório de certificados",
  req: "Cria e inspeciona pedidos de certificado (CSR)",
  rsa: "Lê, converte e inspeciona chaves RSA",
  rsautl: "Assina e cifra com RSA diretamente (obsoleto: use pkeyutl)",
  s_client: "Abre uma conexão TLS de cliente para diagnosticar um servidor",
  s_server: "Sobe um servidor TLS de teste",
  s_time: "Mede o desempenho de handshakes TLS",
  sess_id: "Lê e converte sessões TLS salvas",
  skeyutl: "Gera e administra chaves simétricas opacas de provider",
  smime: "Assina, cifra e verifica mensagens S/MIME",
  speed: "Mede a velocidade dos algoritmos desta build",
  spkac: "Lê e cria blocos SPKAC",
  srp: "Administra um arquivo de verificadores SRP",
  storeutl: "Lista o conteúdo de um armazenamento de objetos (URI store)",
  ts: "Cria e verifica carimbos de tempo (RFC 3161)",
  verify: "Verifica uma cadeia de certificados",
  version: "Mostra a versão do OpenSSL e como ele foi compilado",
  x509: "Lê, converte, assina e inspeciona certificados X.509",
};

/** TAB e quebra de linha são o formato; um dentro do texto o destruiria. */
const limpo = (t: string) => t.replace(/[\t\r\n]+/g, " ").trim();

const fontes: Fonte[] = [];
for (const impl of IMPLEMENTACOES) {
  const caminho = impl.caminhos.find((c) => Bun.file(c).size > 0);
  if (!caminho) {
    console.error(`sem ${impl.rotulo}: nenhum de ${impl.caminhos.join(", ")}`);
    continue;
  }
  const proc = Bun.spawnSync([caminho, "help"]);
  // O `help` do LibreSSL escreve a lista no stderr, avisa que não conhece o
  // comando e ainda sai com código de erro — é assim mesmo. Tratar isso como
  // falha deixaria a geração vazia.
  const saida = new TextDecoder().decode(proc.stderr.length ? proc.stderr : proc.stdout);
  fontes.push({ rotulo: impl.rotulo, achados: parseHelp(saida) });
  console.error(`  ${impl.rotulo.padEnd(9)} ${caminho}`);
}

const linhas = uniao(fontes).map((e) => {
  const base = e.padrao ? (DESCRICOES[e.nome] ?? "") : "";
  const marca = e.somenteEm ? `só no ${e.somenteEm}` : "";
  const descricao = [base, marca].filter(Boolean).join(base && marca ? " · " : "");
  // Terceiro tipo, e ele não é cosmético: `openssl` publica 113 nomes de
  // ALGORITMO junto dos comandos, e a consulta ordena por `kind DESC`. Sem
  // separar, digitar `openssl s` devolve trinta cifras e enterra `s_client`,
  // `smime` e `speed` — que é justamente o que a lista existe para mostrar.
  const kind = e.padrao ? "subcommand" : "algorithm";
  return ["openssl", e.nome, kind, limpo(descricao)].join("\t");
});

console.error(`\n${linhas.length} entradas`);
console.log(linhas.join("\n"));
