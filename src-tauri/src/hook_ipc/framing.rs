use std::io::{BufRead, Write};

use super::protocol::{hook_event_from_value, RequestEnvelope, ResponseEnvelope, PROTOCOL_VERSION};
use super::{Handler, HookAction};

pub(crate) const OVERLOADED_REASON: &str =
    "limite de aprovações simultâneas do Tyba atingido — resolva as pendentes e tente de novo";

fn build_response(action: HookAction) -> ResponseEnvelope {
    match action {
        HookAction::Allow { reason } => ResponseEnvelope {
            v: PROTOCOL_VERSION,
            action: "allow".into(),
            reason,
        },
        HookAction::Deny { reason } => ResponseEnvelope {
            v: PROTOCOL_VERSION,
            action: "deny".into(),
            reason: Some(reason),
        },
        HookAction::Ack => ResponseEnvelope {
            v: PROTOCOL_VERSION,
            action: "ack".into(),
            reason: None,
        },
    }
}

pub(crate) fn write_line<W: Write>(mut writer: W, response: &ResponseEnvelope) {
    let Ok(mut payload) = serde_json::to_vec(response) else {
        return;
    };
    payload.push(b'\n');
    let _ = writer.write_all(&payload);
    let _ = writer.flush();
}

/// Um pedido/resposta do gate: lê uma linha (RequestEnvelope), chama o handler,
/// escreve uma linha (ResponseEnvelope). Genérico sobre o transporte — Unix
/// socket ou named pipe do Windows entram como `reader`/`writer`.
pub(crate) fn serve_connection<Rd: BufRead, Wr: Write>(
    mut reader: Rd,
    writer: Wr,
    handler: &Handler,
) {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return,
        Ok(_) => {}
    }
    let Ok(request) = serde_json::from_str::<RequestEnvelope>(line.trim_end()) else {
        return;
    };
    let event = hook_event_from_value(request.event);
    let action = handler(event);
    write_line(writer, &build_response(action));
}

pub(crate) fn reject_overloaded<Wr: Write>(writer: Wr) {
    write_line(
        writer,
        &ResponseEnvelope {
            v: PROTOCOL_VERSION,
            action: "deny".into(),
            reason: Some(OVERLOADED_REASON.into()),
        },
    );
}

/// Lado cliente: escreve o RequestEnvelope, lê a resposta. Genérico sobre o transporte.
pub(crate) fn exchange<Rd: BufRead, Wr: Write>(
    mut reader: Rd,
    mut writer: Wr,
    event: &serde_json::Value,
) -> Option<ResponseEnvelope> {
    let request = RequestEnvelope {
        v: PROTOCOL_VERSION,
        event: event.clone(),
    };
    let mut payload = serde_json::to_vec(&request).ok()?;
    payload.push(b'\n');
    writer.write_all(&payload).ok()?;
    writer.flush().ok()?;

    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return None,
        Ok(_) => {}
    }
    serde_json::from_str::<ResponseEnvelope>(line.trim_end()).ok()
}
