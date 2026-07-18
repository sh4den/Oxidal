use keyring::Entry;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;
use zeroize::Zeroize as _;

const SERVICE: &str = "Oxidal";

fn entry(id: Uuid) -> Option<Entry> {
    Entry::new(SERVICE, &id.to_string()).ok()
}

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
    raw.zeroize();
    Some(secret)
}

pub fn delete_password(id: Uuid) {
    if let Some(entry) = entry(id) {
        let _ = entry.delete_credential();
    }
}
