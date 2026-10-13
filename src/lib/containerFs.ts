// Recorte de tela do painel de arquivos do container: tamanho legível e o
// caminho em migalhas. A listagem (`ls -lAn`) é lida no core.

/** `812 B`, `1.2 KB`, `34 MB`. */
export function formatSize(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let n = bytes;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i += 1;
  }
  if (i === 0) return `${n} B`;
  return `${n < 10 ? n.toFixed(1).replace(/\.0$/, "") : Math.round(n)} ${units[i]}`;
}

/** `/srv/app` → `/`, `srv`, `app`, cada um com o caminho até ele. */
export function pathCrumbs(path: string): { label: string; path: string }[] {
  const parts = path.split("/").filter(Boolean);
  const out = [{ label: "/", path: "/" }];
  let acc = "";
  for (const part of parts) {
    acc += `/${part}`;
    out.push({ label: part, path: acc });
  }
  return out;
}
