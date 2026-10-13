//! Lanes do grafo do histórico, calculadas no core (spec: git-completo,
//! fatia 2). Nunca `git log --graph`: o ASCII dele é caro, não pagina e teria
//! de ser reparseado. Aqui cada linha recebe a coluna do nó, a cor e as
//! arestas que a atravessam, a partir só de `sha` + pais em ordem topológica.
//!
//! Incremental: o estado (que sha cada coluna espera e com que cor) viaja no
//! cursor do log, então a página 2 continua exatamente as linhas da página 1.
//!
//! Geometria de uma linha (o nó fica no meio da altura):
//! - `Pass`: coluna que atravessa a linha inteira sem tocar o nó;
//! - `In`: metade de cima, da coluna `from` no topo até o nó — filho chegando
//!   (com `from != lane`, é o ponto onde uma branch saiu desta linha);
//! - `Out`: metade de baixo, do nó até a coluna `to` na base — pai saindo
//!   (com `to != lane` num pai além do primeiro, é a outra ponta de um merge).

use serde::{Deserialize, Serialize};

/// Cor reservada para branch de agente (`tyba/*`): âmbar na UI.
pub const AGENT_COLOR: u16 = 0;
/// Cores comuns giram em `1..=PALETTE` (a UI tem uma paleta deste tamanho).
pub const PALETTE: u16 = 7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lane {
    /// Sha que esta coluna espera (um pai já visto, ainda não mostrado).
    pub sha: String,
    pub color: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphState {
    #[serde(default)]
    pub lanes: Vec<Option<Lane>>,
    #[serde(default)]
    pub next_color: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Edge {
    Pass { lane: u16, color: u16 },
    In { from: u16, color: u16 },
    Out { to: u16, color: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphRow {
    /// Coluna do nó.
    pub lane: u16,
    pub color: u16,
    pub edges: Vec<Edge>,
    /// Colunas ocupadas nesta linha (para a largura da coluna do grafo).
    pub width: u16,
}

fn idx(i: usize) -> u16 {
    u16::try_from(i).unwrap_or(u16::MAX)
}

impl GraphState {
    fn alloc_color(&mut self) -> u16 {
        let color = 1 + self.next_color % PALETTE;
        self.next_color = self.next_color.wrapping_add(1);
        color
    }

    fn free_slot(&mut self) -> usize {
        match self.lanes.iter().position(Option::is_none) {
            Some(i) => i,
            None => {
                self.lanes.push(None);
                self.lanes.len() - 1
            }
        }
    }

    fn lane_of(&self, sha: &str) -> Option<usize> {
        self.lanes
            .iter()
            .position(|l| l.as_ref().is_some_and(|l| l.sha == sha))
    }

    /// Shas esperados pelas colunas — a mesma fronteira do cursor do log.
    pub fn pending(&self) -> impl Iterator<Item = &str> {
        self.lanes.iter().flatten().map(|l| l.sha.as_str())
    }

    /// Uma linha do grafo. `agent_tip`: o commit carrega uma branch `tyba/*`
    /// — se ninguém o esperava (é ponta), a coluna nasce âmbar.
    pub fn row(&mut self, sha: &str, parents: &[String], agent_tip: bool) -> GraphRow {
        let width_before = self.lanes.len();
        let hits: Vec<usize> = self
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.as_ref().is_some_and(|l| l.sha == sha))
            .map(|(i, _)| i)
            .collect();

        let mut edges = Vec::new();
        for (i, lane) in self.lanes.iter().enumerate() {
            if let Some(l) = lane {
                if l.sha == sha {
                    edges.push(Edge::In {
                        from: idx(i),
                        color: l.color,
                    });
                } else {
                    edges.push(Edge::Pass {
                        lane: idx(i),
                        color: l.color,
                    });
                }
            }
        }

        let (col, color) = match hits.first() {
            Some(&first) => {
                let color = self.lanes[first].as_ref().map_or(0, |l| l.color);
                (first, color)
            }
            None => {
                let color = if agent_tip {
                    AGENT_COLOR
                } else {
                    self.alloc_color()
                };
                (self.free_slot(), color)
            }
        };
        for &i in &hits {
            self.lanes[i] = None;
        }

        let mut seen: Vec<&str> = Vec::with_capacity(parents.len());
        for (k, parent) in parents.iter().enumerate() {
            if seen.contains(&parent.as_str()) {
                continue;
            }
            seen.push(parent);
            // Primeiro pai já esperado numa coluna à DIREITA: esta linha segue
            // reta (as duas colunas esperam o mesmo sha e se juntam nele). Sem
            // isso, a main dobraria para a coluna da feature que chegou antes
            // ao mesmo pai — e ficaria lá pelo resto do histórico.
            let existing = self.lane_of(parent).filter(|&j| k > 0 || j < col);
            if let Some(j) = existing {
                // Pai já esperado em outra coluna: a linha converge para lá.
                // O primeiro pai leva a cor desta linha (é ela que acaba ali);
                // os outros, a da coluna que já existia.
                let c = if k == 0 {
                    color
                } else {
                    self.lanes[j].as_ref().map_or(color, |l| l.color)
                };
                edges.push(Edge::Out {
                    to: idx(j),
                    color: c,
                });
            } else if k == 0 {
                self.lanes[col] = Some(Lane {
                    sha: parent.clone(),
                    color,
                });
                edges.push(Edge::Out {
                    to: idx(col),
                    color,
                });
            } else {
                let c = self.alloc_color();
                let j = self.free_slot();
                self.lanes[j] = Some(Lane {
                    sha: parent.clone(),
                    color: c,
                });
                edges.push(Edge::Out {
                    to: idx(j),
                    color: c,
                });
            }
        }

        let width = width_before.max(self.lanes.len()).max(col + 1);
        while matches!(self.lanes.last(), Some(None)) {
            self.lanes.pop();
        }
        GraphRow {
            lane: idx(col),
            color,
            edges,
            width: idx(width),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    fn p(list: &[&str]) -> Vec<String> {
        list.iter().map(|x| s(x)).collect()
    }

    /// Roda o grafo numa lista `(sha, pais)` em ordem topológica.
    fn run(state: &mut GraphState, commits: &[(&str, &[&str])]) -> Vec<GraphRow> {
        commits
            .iter()
            .map(|(sha, parents)| state.row(sha, &p(parents), false))
            .collect()
    }

    fn outs(row: &GraphRow) -> Vec<u16> {
        row.edges
            .iter()
            .filter_map(|e| match e {
                Edge::Out { to, .. } => Some(*to),
                _ => None,
            })
            .collect()
    }

    fn ins(row: &GraphRow) -> Vec<u16> {
        row.edges
            .iter()
            .filter_map(|e| match e {
                Edge::In { from, .. } => Some(*from),
                _ => None,
            })
            .collect()
    }

    fn passes(row: &GraphRow) -> Vec<u16> {
        row.edges
            .iter()
            .filter_map(|e| match e {
                Edge::Pass { lane, .. } => Some(*lane),
                _ => None,
            })
            .collect()
    }

    /// Toda aresta que sai pela base de uma linha tem de entrar pelo topo da
    /// seguinte, na mesma coluna — o grafo não pode ter fio solto. (A cor
    /// pode mudar na junção: a linha que converge acaba ali.)
    fn assert_continuous(rows: &[GraphRow]) {
        for w in rows.windows(2) {
            let mut bottom: Vec<u16> = w[0]
                .edges
                .iter()
                .filter_map(|e| match *e {
                    Edge::Pass { lane, .. } => Some(lane),
                    Edge::Out { to, .. } => Some(to),
                    Edge::In { .. } => None,
                })
                .collect();
            let mut top: Vec<u16> = w[1]
                .edges
                .iter()
                .filter_map(|e| match *e {
                    Edge::Pass { lane, .. } => Some(lane),
                    Edge::In { from, .. } => Some(from),
                    Edge::Out { .. } => None,
                })
                .collect();
            bottom.sort_unstable();
            bottom.dedup();
            top.sort_unstable();
            assert_eq!(bottom, top, "fio solto entre {:?} e {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn linear_history_stays_in_one_lane_and_one_color() {
        let mut g = GraphState::default();
        let rows = run(&mut g, &[("c", &["b"]), ("b", &["a"]), ("a", &[])]);
        assert!(rows.iter().all(|r| r.lane == 0 && r.width == 1));
        assert!(rows.iter().all(|r| r.color == rows[0].color));
        assert_eq!(outs(&rows[0]), vec![0]);
        assert_eq!(ins(&rows[1]), vec![0]);
        assert!(outs(&rows[2]).is_empty(), "raiz não tem pai");
        assert!(g.lanes.is_empty(), "nada pendente depois da raiz");
        assert_continuous(&rows);
    }

    #[test]
    fn branch_and_merge_open_and_close_a_second_lane() {
        // M = merge(c2, x2); x2 -> x1 -> c1; c2 -> c1; c1 -> c0
        let mut g = GraphState::default();
        let rows = run(
            &mut g,
            &[
                ("m", &["c2", "x2"]),
                ("x2", &["x1"]),
                ("x1", &["c1"]),
                ("c2", &["c1"]),
                ("c1", &["c0"]),
                ("c0", &[]),
            ],
        );
        let main = rows[0].color;
        // merge: segue no 0 e abre o 1 para o segundo pai
        assert_eq!(outs(&rows[0]), vec![0, 1]);
        let feat = match rows[0].edges[1] {
            Edge::Out { color, .. } => color,
            _ => unreachable!(),
        };
        assert_ne!(feat, main);
        // a feature anda na coluna 1 com a cor dela; main passa reto
        assert_eq!((rows[1].lane, rows[1].color), (1, feat));
        assert_eq!(passes(&rows[1]), vec![0]);
        // x1: primeiro pai (c1) ainda não é esperado por ninguém -> segue no 1
        assert_eq!(outs(&rows[2]), vec![1]);
        // c2 (coluna 0) também tem c1 como pai, já esperado na coluna 1 (à
        // direita): a main segue reta e as duas colunas esperam c1
        assert_eq!(rows[3].lane, 0);
        assert_eq!(outs(&rows[3]), vec![0]);
        // c1: a feature fecha nele, na coluna da main, com a cor da main
        assert_eq!((rows[4].lane, rows[4].color), (0, main));
        assert_eq!(ins(&rows[4]), vec![0, 1]);
        assert_eq!(rows[4].width, 2);
        assert_continuous(&rows);
        assert!(g.lanes.is_empty());
    }

    #[test]
    fn fork_point_receives_both_children_lanes() {
        // dois filhos (b na col 0, f na col 1) apontam para a; a fecha as duas
        let mut g = GraphState::default();
        let rows = run(&mut g, &[("b", &["a"]), ("f", &["a"]), ("a", &[])]);
        assert_eq!(rows[1].lane, 1, "ponta nova sem filho abre coluna livre");
        // f converge para a coluna 0, que já espera `a`
        assert_eq!(outs(&rows[1]), vec![0]);
        assert_eq!(ins(&rows[2]), vec![0]);
        assert_continuous(&rows);
    }

    #[test]
    fn octopus_merge_opens_one_lane_per_extra_parent() {
        let mut g = GraphState::default();
        let rows = run(
            &mut g,
            &[
                ("o", &["a", "b", "c"]),
                ("c", &["base"]),
                ("b", &["base"]),
                ("a", &["base"]),
                ("base", &[]),
            ],
        );
        assert_eq!(outs(&rows[0]), vec![0, 1, 2]);
        let colors: std::collections::HashSet<u16> = rows[0]
            .edges
            .iter()
            .map(|e| match *e {
                Edge::Out { color, .. } | Edge::In { color, .. } | Edge::Pass { color, .. } => {
                    color
                }
            })
            .collect();
        assert_eq!(colors.len(), 3, "cada pai com sua cor");
        assert_eq!(rows[0].width, 3);
        assert_eq!(rows[1].lane, 2);
        assert_eq!(rows[2].lane, 1);
        // base: as três colunas convergem nele
        assert_eq!(rows[4].lane, 0);
        assert_continuous(&rows);
        assert!(g.lanes.is_empty());
    }

    #[test]
    fn criss_cross_merges_keep_every_edge_connected() {
        // m1 = merge(a, b), m2 = merge(b, a): os dois mergeiam os mesmos pais
        // em ordem trocada (o caso que gera merge-base múltiplo).
        let mut g = GraphState::default();
        let rows = run(
            &mut g,
            &[
                ("m2", &["b", "a"]),
                ("m1", &["a", "b"]),
                ("b", &["base"]),
                ("a", &["base"]),
                ("base", &[]),
            ],
        );
        // m1 é ponta (ninguém o espera): coluna livre nova
        assert_eq!(rows[1].lane, 2);
        // os dois pais de m1 já são esperados (b na 0, a na 1): só converge
        let mut o = outs(&rows[1]);
        o.sort_unstable();
        assert_eq!(o, vec![0, 1]);
        // b é esperado na coluna 0 por m2 e por m1 -> uma entrada só por coluna
        assert_eq!(rows[2].lane, 0);
        assert_continuous(&rows);
        assert!(g.lanes.is_empty());
    }

    #[test]
    fn lanes_continue_identically_across_a_page_boundary() {
        let commits: &[(&str, &[&str])] = &[
            ("m", &["c2", "x2"]),
            ("x2", &["x1"]),
            ("c2", &["c1"]),
            ("x1", &["c1"]),
            ("c1", &["c0"]),
            ("c0", &[]),
        ];
        let mut whole = GraphState::default();
        let expected = run(&mut whole, commits);
        for cut in 1..commits.len() {
            let mut first = GraphState::default();
            let mut rows = run(&mut first, &commits[..cut]);
            // o estado viaja no cursor: ida e volta por JSON, como no IPC
            let json = serde_json::to_string(&first).unwrap();
            let mut second: GraphState = serde_json::from_str(&json).unwrap();
            rows.extend(run(&mut second, &commits[cut..]));
            assert_eq!(rows, expected, "corte na linha {cut}");
        }
    }

    #[test]
    fn agent_tip_lane_is_amber_and_first_parent_inherits_the_color() {
        let mut g = GraphState::default();
        let main = g.row("c4", &p(&["c3"]), false);
        let agent = g.row("ag", &p(&["c4x"]), true);
        assert_eq!(agent.color, AGENT_COLOR);
        assert_ne!(main.color, AGENT_COLOR);
        let next = g.row("c4x", &p(&["c3"]), false);
        assert_eq!(next.color, AGENT_COLOR, "a coluna do agente segue âmbar");
    }

    #[test]
    fn freed_columns_are_reused_and_trailing_ones_trimmed() {
        let mut g = GraphState::default();
        run(&mut g, &[("m", &["a", "x"]), ("x", &["a"])]);
        assert_eq!(g.lanes.len(), 1, "coluna 1 fechou e saiu do fim");
        assert_eq!(g.pending().collect::<Vec<_>>(), vec!["a"]);
    }
}
