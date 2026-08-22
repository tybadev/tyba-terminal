/**
 * Navegação por seta entre as duas opções do seletor de modo de shell.
 *
 * Existe fora do componente porque é a única parte com regra: um `radiogroup`
 * move a seleção com as setas, e não só com clique — quem chega pelo teclado
 * não tem outro caminho. O resto do seletor é desenho.
 *
 * `true` é a linha do TYBA (primeira opção), `false` é o prompt do shell.
 *
 * `null` significa "esta tecla não é minha": o componente não chama
 * `preventDefault` e a tecla segue para quem cuida dela — é o que mantém o Tab
 * saindo do grupo em vez de ser engolido.
 *
 * As quatro setas, e não só as horizontais: os cartões ficam lado a lado em
 * tela larga e empilhados em tela estreita, e a seta que o usuário aperta segue
 * o que ele VÊ. Tratar só ←/→ deixa o grupo mudo justamente no layout estreito.
 *
 * Seta **circula**, que é o comportamento de `radiogroup` — e com duas opções
 * circular é o mesmo que alternar, nos dois sentidos. `Home`/`End` são
 * absolutos, e é por isso que não podem ser escritos como alternância: eles
 * teriam de mudar de destino conforme a seleção atual, que é o oposto do que
 * significam.
 */
export function nextPromptMode(current: boolean, key: string): boolean | null {
  switch (key) {
    case "ArrowLeft":
    case "ArrowUp":
    case "ArrowRight":
    case "ArrowDown":
      return !current;
    case "Home":
      return true;
    case "End":
      return false;
    default:
      return null;
  }
}
