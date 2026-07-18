//! Secure storage for session passwords.
//!
//! Passwords are never written to `sessions.json`. They live in the operating
//! system's credential vault via the `keyring` crate: Windows Credential
//! Manager (DPAPI-encrypted), the macOS Keychain, or the Linux Secret
//! Service. Each session's password is keyed by its UUID, so renames and
//! other edits don't orphan the stored secret.

use keyring::Entry;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;
use zeroize::Zeroize as _;

const SERVICE: &str = "Oxidal";

fn entry(id: Uuid) -> Option<Entry> {
    Entry::new(SERVICE, &id.to_string()).ok()
}

/// Store (or clear, when empty) the password for a session. Best effort:
/// a locked or unavailable vault must not block saving the session itself.
pub fn store_password(id: Uuid, password: &SecretString) {
    let Some(entry) = entry(id) else { return };
    let password = password.expose_secret();
    if password.is_empty() {
        let _ = entry.delete_credential();
    } else {
        let _ = entry.set_password(password);
    }
}

pub fn load_password(id: Uuid) -> Option<SecretString> {
    let mut raw = entry(id)?.get_password().ok()?;
    let secret = SecretString::from(raw.as_str());
    // The vault hands back a plain String; wipe it once it's wrapped.
    raw.zeroize();
    Some(secret)
}

pub fn delete_password(id: Uuid) {
    if let Some(entry) = entry(id) {
        let _ = entry.delete_credential();
    }
}
