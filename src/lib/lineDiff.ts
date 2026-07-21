export type LineDiffKind = "equal" | "added" | "removed";

export interface LineDiffRow {
  kind: LineDiffKind;
  text: string;
}

/// Diff linha-a-linha via LCS, para a comparação "em edição × em disco" do
/// banner de conflito. `a` é o lado esquerdo (disco), `b` o direito (edição):
/// linhas só em `a` viram `removed`, só em `b` viram `added`. Função pura.
export function lineDiff(a: string, b: string): LineDiffRow[] {
  const left = a.split("\n");
  const right = b.split("\n");
  const n = left.length;
  const m = right.length;

  const lcs: number[][] = Array.from({ length: n + 1 }, () =>
    new Array<number>(m + 1).fill(0),
  );
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] =
        left[i] === right[j]
          ? lcs[i + 1][j + 1] + 1
          : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }

  const rows: LineDiffRow[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (left[i] === right[j]) {
      rows.push({ kind: "equal", text: left[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      rows.push({ kind: "removed", text: left[i] });
      i++;
    } else {
      rows.push({ kind: "added", text: right[j] });
      j++;
    }
  }
  while (i < n) {
    rows.push({ kind: "removed", text: left[i] });
    i++;
  }
  while (j < m) {
    rows.push({ kind: "added", text: right[j] });
    j++;
  }
  return rows;
}

export function hasLineDifferences(rows: LineDiffRow[]): boolean {
  return rows.some((row) => row.kind !== "equal");
}
