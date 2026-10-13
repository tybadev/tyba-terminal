/**
 * Arquivo que costuma carregar segredo. O repo do dono é público: um `.env`
 * num commit é publicação. Casa pelo NOME do arquivo, sem diferenciar caixa.
 * Usado pelo painel Git (não entra no commit sem confirmar) e pelo painel
 * Arquivos (marca "protegido").
 */
const SENSITIVE: RegExp[] = [
  /^\.env(\..*)?$/,
  /\.pem$/,
  /\.key$/,
  /^id_rsa/,
  /credentials/,
  /\.p12$/,
  /^\.npmrc$/,
  /^\.pypirc$/,
];

export function isSensitivePath(path: string): boolean {
  const name = (path.split("/").pop() ?? path).toLowerCase();
  return SENSITIVE.some((re) => re.test(name));
}
