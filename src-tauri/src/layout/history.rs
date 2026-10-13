//! Histórico de abas por workspace — o "voltar" do navegador.
//!
//! A pilha guarda as abas que estiveram ativas antes da atual, da mais antiga
//! para a mais recente. Ela não persiste: é memória da sessão de uso, igual ao
//! histórico de uma janela de navegador.

use super::TabId;

/// Teto da pilha. Ninguém volta trinta e duas abas; o teto só impede que uma
/// janela aberta por semanas acumule ids mortos sem fim.
pub const HISTORY_CAP: usize = 32;

/// A aba ativa trocou de `from` para `to`. `from` vai para o topo da pilha (sem
/// duplicar) e `to` sai dela: a aba atual nunca é destino de "voltar".
pub fn record(history: &mut Vec<TabId>, from: Option<TabId>, to: TabId) {
    let Some(from) = from else {
        history.retain(|t| *t != to);
        return;
    };
    if from == to {
        return;
    }
    history.retain(|t| *t != from && *t != to);
    history.push(from);
    if history.len() > HISTORY_CAP {
        let excess = history.len() - HISTORY_CAP;
        history.drain(..excess);
    }
}

/// Desempilha o destino de "voltar": a aba mais recente que ainda existe e não
/// é a atual. Ids de abas fechadas no caminho são descartados.
pub fn pop_back(
    history: &mut Vec<TabId>,
    current: Option<TabId>,
    exists: impl Fn(TabId) -> bool,
) -> Option<TabId> {
    while let Some(candidate) = history.pop() {
        if Some(candidate) != current && exists(candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Quem assume quando a aba ativa fecha: a usada antes dela (histórico); sem
/// histórico, a vizinha — a que ocupou o mesmo índice, ou a última.
pub fn successor_after_close(
    history: &mut Vec<TabId>,
    closed: TabId,
    remaining: &[TabId],
    closed_index: usize,
) -> Option<TabId> {
    history.retain(|t| *t != closed);
    if let Some(prev) = pop_back(history, None, |t| remaining.contains(&t)) {
        return Some(prev);
    }
    if remaining.is_empty() {
        return None;
    }
    Some(remaining[closed_index.min(remaining.len() - 1)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ids(n: usize) -> Vec<TabId> {
        (0..n).map(|_| Uuid::new_v4()).collect()
    }

    #[test]
    fn record_pushes_previous_and_never_keeps_current() {
        let t = ids(3);
        let mut h = Vec::new();
        record(&mut h, Some(t[0]), t[1]);
        record(&mut h, Some(t[1]), t[2]);
        assert_eq!(h, vec![t[0], t[1]]);
        // Voltar para uma aba que já estava na pilha a tira de lá.
        record(&mut h, Some(t[2]), t[0]);
        assert_eq!(h, vec![t[1], t[2]]);
    }

    #[test]
    fn record_same_tab_is_noop_and_dedupes() {
        let t = ids(2);
        let mut h = vec![t[0]];
        record(&mut h, Some(t[1]), t[1]);
        assert_eq!(h, vec![t[0]]);
        record(&mut h, Some(t[0]), t[1]);
        record(&mut h, Some(t[1]), t[0]);
        record(&mut h, Some(t[0]), t[1]);
        assert_eq!(h, vec![t[0]], "alternar entre duas não empilha cópias");
    }

    #[test]
    fn record_respects_cap() {
        let t = ids(HISTORY_CAP + 5);
        let mut h = Vec::new();
        for w in t.windows(2) {
            record(&mut h, Some(w[0]), w[1]);
        }
        assert_eq!(h.len(), HISTORY_CAP);
        assert_eq!(h.last(), Some(&t[t.len() - 2]));
    }

    #[test]
    fn pop_back_walks_like_a_browser_and_skips_closed_tabs() {
        let t = ids(4);
        let mut h = vec![t[0], t[1], t[2]];
        let alive = [t[0], t[2], t[3]];
        assert_eq!(
            pop_back(&mut h, Some(t[3]), |x| alive.contains(&x)),
            Some(t[2])
        );
        // t[1] foi fechada: pula para t[0].
        assert_eq!(
            pop_back(&mut h, Some(t[2]), |x| alive.contains(&x)),
            Some(t[0])
        );
        assert_eq!(pop_back(&mut h, Some(t[0]), |x| alive.contains(&x)), None);
    }

    #[test]
    fn close_goes_to_previously_used_tab_not_neighbor() {
        let t = ids(4);
        // Usou t0, depois t3, depois t1 (ativa) — fecha t1: volta para t3.
        let mut h = vec![t[0], t[3]];
        let remaining = [t[0], t[2], t[3]];
        assert_eq!(
            successor_after_close(&mut h, t[1], &remaining, 1),
            Some(t[3])
        );
        assert_eq!(h, vec![t[0]]);
    }

    #[test]
    fn close_without_history_falls_back_to_neighbor() {
        let t = ids(3);
        let mut h = Vec::new();
        assert_eq!(
            successor_after_close(&mut h, t[2], &[t[0], t[1]], 2),
            Some(t[1])
        );
        assert_eq!(successor_after_close(&mut h, t[0], &[], 0), None);
    }
}
