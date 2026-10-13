import { describe, expect, it } from "bun:test";

import { formatSize, pathCrumbs } from "./containerFs";

describe("formatSize", () => {
  it("mostra bytes inteiros e uma casa só abaixo de 10", () => {
    expect(formatSize(0)).toBe("0 B");
    expect(formatSize(812)).toBe("812 B");
    expect(formatSize(1024)).toBe("1 KB");
    expect(formatSize(1229)).toBe("1.2 KB");
    expect(formatSize(35_651_584)).toBe("34 MB");
    expect(formatSize(3 * 1024 ** 3)).toBe("3 GB");
  });
});

describe("pathCrumbs", () => {
  it("quebra o caminho em migalhas navegáveis", () => {
    expect(pathCrumbs("/")).toEqual([{ label: "/", path: "/" }]);
    expect(pathCrumbs("/srv/app")).toEqual([
      { label: "/", path: "/" },
      { label: "srv", path: "/srv" },
      { label: "app", path: "/srv/app" },
    ]);
  });
});
