//! Se o sistema deixa o TYBA avisar (Configurações → Notificações, "Avisos do
//! macOS: Permitidos"). Só leitura: quem muda são os Ajustes do Sistema.
//!
//! O plugin de notificação responde "permitido" sempre no desktop; aqui se
//! pergunta ao `UNUserNotificationCenter` de verdade. Fora do macOS não há o
//! que perguntar (`None`, e a tela não mostra a linha).

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyPermission {
    Granted,
    Denied,
    /// O sistema ainda não perguntou ao dono.
    NotDetermined,
}

/// `UNAuthorizationStatus` → o que a tela mostra. Provisório e efêmero
/// entregam o aviso (quieto ou por um tempo): contam como permitido.
pub fn from_status(status: isize) -> NotifyPermission {
    match status {
        1 => NotifyPermission::Denied,
        2..=4 => NotifyPermission::Granted,
        _ => NotifyPermission::NotDetermined,
    }
}

/// O estado agora. `None` fora do macOS, fora de um `.app` (o centro de
/// avisos derruba o processo sem bundle) ou se o sistema não responder.
#[cfg(target_os = "macos")]
pub fn current() -> Option<NotifyPermission> {
    use std::ptr::NonNull;
    use std::sync::mpsc;
    use std::time::Duration;

    use block2::RcBlock;
    use objc2_foundation::NSBundle;
    use objc2_user_notifications::{UNNotificationSettings, UNUserNotificationCenter};

    NSBundle::mainBundle().bundleIdentifier()?;
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: o sistema entrega um `UNNotificationSettings` válido
        // durante a chamada do bloco.
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        let _ = tx.send(status.0);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getNotificationSettingsWithCompletionHandler(&block);
    rx.recv_timeout(Duration::from_secs(2))
        .ok()
        .map(from_status)
}

#[cfg(not(target_os = "macos"))]
pub fn current() -> Option<NotifyPermission> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_system_status() {
        assert_eq!(from_status(0), NotifyPermission::NotDetermined);
        assert_eq!(from_status(1), NotifyPermission::Denied);
        assert_eq!(from_status(2), NotifyPermission::Granted);
        assert_eq!(from_status(3), NotifyPermission::Granted);
        assert_eq!(from_status(4), NotifyPermission::Granted);
        assert_eq!(from_status(99), NotifyPermission::NotDetermined);
    }

    #[test]
    fn serializes_for_the_front() {
        assert_eq!(
            serde_json::to_string(&NotifyPermission::NotDetermined).unwrap(),
            "\"not_determined\""
        );
    }
}
