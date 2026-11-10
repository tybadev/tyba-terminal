import { PLATE, PLATE_ACRYLIC, PLATE_LINE } from "@/components/plate";

// Gaveta do Orbit (quadro GavetasOrbit): a placa da opção G, igual às da
// faixa, flutuando 8 px abaixo dela (faixa em top-3 com h-11) e a 12 px da
// borda direita, por cima do canvas e sem véu — o canvas segue à vista e
// clicável. Uma por vez, no mesmo lugar.
export const DRAWER = `absolute top-16 right-3 z-[6] flex max-w-[calc(100%-24px)] flex-col overflow-hidden ${PLATE} ${PLATE_LINE} ${PLATE_ACRYLIC}`;

/** Integrações e Papéis: 620 de largura e altura cheia até o pé do canvas. */
export const DRAWER_WIDE = `${DRAWER} bottom-3 w-[620px]`;

/** Fio da direita da biblioteca, na cor do fio da G. */
export const LINE_R = "shadow-[inset_-1px_0_0_var(--tyba-line)]";
