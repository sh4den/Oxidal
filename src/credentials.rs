//! Secure storage for session passwords.
//!
//! Passwords are never written to `sessions.json`. They live in the operating
//! system's credential vault via the `keyring` crate: Windows Credential
//! Manager (DPAPI-encrypted), the macOS Keychain, or the Linux Secret
//! Service. Each session's password is keyed by its UUID, so renames and
//! other edits don't orphan the stored secret.

use keyring::Entry;
use uuid::Uuid;

const SERVICE: &str = "Oxidal";

fn entry(id: Uuid) -> Option<Entry> {
    Entry::new(SERVICE, &id.to_string()).ok()
}

/// Store (or clear, when empty) the password for a session. Best effort:
/// a locked or unavailable vault must not block saving the session itself.
pub fn store_password(id: Uuid, password: &str) {
    let Some(entry) = entry(id) else { return };
    if password.is_empty() {
        let _ = entry.delete_credential();
    } else {
        let _ = entry.set_password(password);
    }
}

pub fn load_password(id: Uuid) -> Option<String> {
    entry(id)?.get_password().ok()
}

pub fn delete_password(id: Uuid) {
    if let Some(entry) = entry(id) {
        let _ = entry.delete_credential();
    }
}
