//! `tyba events`: o que muda no TYBA aberto, uma linha por mudança.
//!
//! Do lado da CLI, sem canal novo no app: pergunta o mesmo `ls` (já
//! redigido) a cada [`POLL_INTERVAL`] e imprime a diferença entre duas
//! leituras. Nada de novo sai do app além do que o `tyba ls` já mostra.
//! Começa do agora: o estado inicial é o `tyba ls`. Uma mudança que vai e
//! volta entre duas leituras não aparece.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{AgentEntry, ControlRequest, Listing, EXIT_FAILURE, EXIT_OK, PROTOCOL_VERSION};

pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Uma linha do `tyba events --json`. Campos estáveis: só se acrescenta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    PaneOpened {
        pane: String,
        workspace: String,
        label: String,
        kind: String,
        agent: Option<AgentEntry>,
    },
    PaneClosed {
        pane: String,
    },
    /// O agente do pane mudou: apareceu, trocou de estado ou saiu (`null`).
    Agent {
        pane: String,
        agent: Option<AgentEntry>,
    },
}

/// O que mudou de `before` para `after`: fechados primeiro, depois abertos e
/// agentes na ordem da tela.
pub fn diff(before: &Listing, after: &Listing) -> Vec<Event> {
    let panes = |listing: &Listing| {
        listing
            .workspaces
            .iter()
            .flat_map(|ws| ws.panes.iter().map(move |p| (ws.id.clone(), p.clone())))
            .collect::<Vec<_>>()
    };
    let (old, new) = (panes(before), panes(after));
    let mut events: Vec<Event> = old
        .iter()
        .filter(|(_, p)| !new.iter().any(|(_, n)| n.id == p.id))
        .map(|(_, p)| Event::PaneClosed { pane: p.id.clone() })
        .collect();
    for (workspace, pane) in &new {
        match old.iter().find(|(_, o)| o.id == pane.id) {
            None => events.push(Event::PaneOpened {
                pane: pane.id.clone(),
                workspace: workspace.clone(),
                label: pane.label.clone(),
                kind: pane.kind.clone(),
                agent: pane.agent.clone(),
            }),
            Some((_, o)) if o.agent != pane.agent => events.push(Event::Agent {
                pane: pane.id.clone(),
                agent: pane.agent.clone(),
            }),
            Some(_) => {}
        }
    }
    events
}

pub fn render(event: &Event) -> String {
    let short = |id: &str| {
        let s: String = id.chars().filter(|c| *c != '-').collect();
        s[s.len().saturating_sub(8)..].to_string()
    };
    let agent = |a: &Option<AgentEntry>| match a {
        Some(a) => format!("{} · {}", a.name, super::state_label(a.state.as_deref())),
        None => "sem agente".into(),
    };
    match event {
        Event::PaneOpened {
            pane, label, kind, ..
        } => format!("abriu    {}  {label}  [{kind}]", short(pane)),
        Event::PaneClosed { pane } => format!("fechou   {}", short(pane)),
        Event::Agent { pane, agent: a } => format!("agente   {}  {}", short(pane), agent(a)),
    }
}

fn fetch(socket: &Path) -> Result<Listing, (i32, String)> {
    let request = ControlRequest {
        v: PROTOCOL_VERSION,
        op: "ls".into(),
        ..Default::default()
    };
    let response = super::exchange(socket, &request)?;
    if !response.ok {
        return Err((
            response.code.unwrap_or(EXIT_FAILURE),
            response
                .error
                .unwrap_or_else(|| "o TYBA recusou o pedido".into()),
        ));
    }
    serde_json::from_value(response.result.unwrap_or(serde_json::Value::Null))
        .map_err(|e| (EXIT_FAILURE, format!("resposta inesperada do TYBA: {e}")))
}

/// Roda até o app fechar (sai com 4) ou a saída fechar (`| head`: sai com 0).
pub fn watch(socket: &Path, json: bool, out: &mut impl Write, err: &mut impl Write) -> i32 {
    let mut before = match fetch(socket) {
        Ok(listing) => listing,
        Err((code, message)) => {
            let _ = writeln!(err, "tyba: {message}");
            return code;
        }
    };
    loop {
        std::thread::sleep(POLL_INTERVAL);
        let after = match fetch(socket) {
            Ok(listing) => listing,
            Err((code, message)) => {
                let _ = writeln!(err, "tyba: {message}");
                return code;
            }
        };
        for event in diff(&before, &after) {
            let line = if json {
                serde_json::to_string(&event).unwrap_or_default()
            } else {
                render(&event)
            };
            if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                return EXIT_OK;
            }
        }
        before = after;
    }
}
