//! O alvo de um comando da CLI (`tyba capture <alvo>`): qual pane o dono quis
//! dizer. Contrato copiado do Rex: ID → rótulo → posição → sufixo, nessa
//! ordem; o primeiro critério que acha alguém decide. Mais de um pane no
//! critério que decidiu é ambíguo — sai com código 3 e nada muda.
//!
//! Pura sobre a lista: quem chama monta a lista do layout na hora.

/// O que o parser precisa saber de um pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneRef<'a> {
    pub id: &'a str,
    pub label: &'a str,
    /// 1-based, na ordem do `tyba ls`.
    pub position: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    /// Alvo vazio: uso inválido (código 2).
    Empty,
    NotFound,
    /// Os ids dos panes que empataram, para a mensagem.
    Ambiguous(Vec<String>),
}

/// Índice em `panes` do pane que `target` nomeia.
pub fn resolve(panes: &[PaneRef<'_>], target: &str) -> Result<usize, TargetError> {
    let target = target.trim();
    if target.is_empty() {
        return Err(TargetError::Empty);
    }
    let lower = target.to_ascii_lowercase();

    // 1. ID inteiro (com ou sem hífens).
    let by_id = matching(panes, |p| {
        p.id.eq_ignore_ascii_case(target) || simple(p.id) == simple(&lower)
    });
    if let Some(decided) = decide(panes, by_id) {
        return decided;
    }

    // 2. Rótulo exato.
    if let Some(decided) = decide(panes, matching(panes, |p| p.label == target)) {
        return decided;
    }

    // 3. Posição no `tyba ls`.
    if let Ok(position) = target.parse::<usize>() {
        if let Some(decided) = decide(panes, matching(panes, |p| p.position == position)) {
            return decided;
        }
    }

    // 4. Sufixo do ID (o que sobra de um id curto copiado do `ls`).
    let suffix = simple(&lower);
    if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Some(decided) = decide(panes, matching(panes, |p| simple(p.id).ends_with(&suffix))) {
            return decided;
        }
    }
    Err(TargetError::NotFound)
}

fn simple(id: &str) -> String {
    id.chars()
        .filter(|c| *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn matching(panes: &[PaneRef<'_>], pred: impl Fn(&PaneRef<'_>) -> bool) -> Vec<usize> {
    panes
        .iter()
        .enumerate()
        .filter(|(_, p)| pred(p))
        .map(|(i, _)| i)
        .collect()
}

fn decide(panes: &[PaneRef<'_>], hits: Vec<usize>) -> Option<Result<usize, TargetError>> {
    match hits.as_slice() {
        [] => None,
        [one] => Some(Ok(*one)),
        many => Some(Err(TargetError::Ambiguous(
            many.iter().map(|i| panes[*i].id.to_string()).collect(),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0a1b2c3d-0000-4000-8000-00000000aaa1";
    const B: &str = "0a1b2c3d-0000-4000-8000-00000000bbb2";
    const C: &str = "0a1b2c3d-0000-4000-8000-00000000ccb2";

    fn panes() -> Vec<PaneRef<'static>> {
        vec![
            PaneRef {
                id: A,
                label: "api",
                position: 1,
            },
            PaneRef {
                id: B,
                label: "build",
                position: 2,
            },
            PaneRef {
                id: C,
                label: "build",
                position: 3,
            },
        ]
    }

    #[test]
    fn full_id_wins_with_or_without_hyphens_and_any_case() {
        assert_eq!(resolve(&panes(), A), Ok(0));
        assert_eq!(resolve(&panes(), &A.to_uppercase()), Ok(0));
        assert_eq!(resolve(&panes(), &A.replace('-', "")), Ok(0));
    }

    #[test]
    fn unique_label_resolves() {
        assert_eq!(resolve(&panes(), "api"), Ok(0));
    }

    #[test]
    fn duplicated_label_is_ambiguous_and_names_both() {
        assert_eq!(
            resolve(&panes(), "build"),
            Err(TargetError::Ambiguous(vec![B.into(), C.into()]))
        );
    }

    #[test]
    fn position_resolves_when_no_label_matches() {
        assert_eq!(resolve(&panes(), "2"), Ok(1));
        assert_eq!(resolve(&panes(), "3"), Ok(2));
    }

    #[test]
    fn label_beats_position() {
        let mut list = panes();
        list[2].label = "1";
        assert_eq!(resolve(&list, "1"), Ok(2), "rótulo vem antes da posição");
    }

    #[test]
    fn position_beats_suffix() {
        // "1" também é sufixo do id A ("…aaa1"), mas a posição 1 decide antes.
        let mut list = panes();
        list[0].position = 9;
        list[1].position = 1;
        assert_eq!(resolve(&list, "1"), Ok(1));
    }

    #[test]
    fn unique_suffix_resolves() {
        assert_eq!(resolve(&panes(), "aaa1"), Ok(0));
        assert_eq!(resolve(&panes(), "BBB2"), Ok(1));
    }

    #[test]
    fn shared_suffix_is_ambiguous() {
        assert_eq!(
            resolve(&panes(), "b2"),
            Err(TargetError::Ambiguous(vec![B.into(), C.into()]))
        );
    }

    #[test]
    fn out_of_range_position_falls_to_suffix_then_not_found() {
        assert_eq!(resolve(&panes(), "42"), Err(TargetError::NotFound));
        assert_eq!(resolve(&panes(), "0"), Err(TargetError::NotFound));
    }

    #[test]
    fn non_hex_text_that_matches_nothing_is_not_found() {
        assert_eq!(resolve(&panes(), "zz"), Err(TargetError::NotFound));
    }

    #[test]
    fn empty_or_blank_target_is_usage_error() {
        assert_eq!(resolve(&panes(), ""), Err(TargetError::Empty));
        assert_eq!(resolve(&panes(), "   "), Err(TargetError::Empty));
    }

    #[test]
    fn empty_list_is_not_found() {
        assert_eq!(resolve(&[], "1"), Err(TargetError::NotFound));
    }
}
