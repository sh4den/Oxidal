use keyring::Entry;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;
use zeroize::Zeroize as _;

const SERVICE: &str = "Oxidal";

fn entry(id: Uuid) -> Option<Entry> {
    Entry::new(SERVICE, &id.to_string()).ok()
}

fn passphrase_entry(id: Uuid) -> Option<Entry> {
    Entry::new(SERVICE, &format!("{id}:key-passphrase")).ok()
}

fn proxy_entry(id: Uuid) -> Option<Entry> {
    Entry::new(SERVICE, &format!("{id}:proxy-password")).ok()
}

fn store(entry: Option<Entry>, secret: &SecretString) {
    let Some(entry) = entry else { return };
    let secret = secret.expose_secret();
    if secret.is_empty() {
        let _ = entry.delete_credential();
    } else {
        let _ = entry.set_password(secret);
    }
}

fn load(entry: Option<Entry>) -> Option<SecretString> {
    let mut raw = entry?.get_password().ok()?;
    let secret = SecretString::from(raw.as_str());
    raw.zeroize();
    Some(secret)
}

pub fn store_password(id: Uuid, password: &SecretString) {
    store(entry(id), password);
}

pub fn load_password(id: Uuid) -> Option<SecretString> {
    load(entry(id))
}

pub fn store_key_passphrase(id: Uuid, passphrase: &SecretString) {
    store(passphrase_entry(id), passphrase);
}

pub fn load_key_passphrase(id: Uuid) -> Option<SecretString> {
    load(passphrase_entry(id))
}

pub fn store_proxy_password(id: Uuid, password: &SecretString) {
    store(proxy_entry(id), password);
}

pub fn load_proxy_password(id: Uuid) -> Option<SecretString> {
    load(proxy_entry(id))
}

pub fn delete_password(id: Uuid) {
    for entry in [entry(id), passphrase_entry(id), proxy_entry(id)]
        .into_iter()
        .flatten()
    {
        let _ = entry.delete_credential();
    }
}
