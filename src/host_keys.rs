use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use gpui::{App, Hsla, IntoElement, ParentElement as _, Styled as _, Window, div, px};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    h_flex, v_flex,
};
use russh::keys::ssh_key::{HashAlg, PublicKey};

const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

pub struct HostKeyRequest {
    host: String,
    port: u16,
    algorithm: String,
    fingerprint: String,
    stored: Vec<String>,
    reply: async_channel::Sender<bool>,
}

impl HostKeyRequest {
    fn answer(&self, trusted: bool) {
        let _ = self.reply.try_send(trusted);
    }
}

type Prompts = (
    async_channel::Sender<HostKeyRequest>,
    async_channel::Receiver<HostKeyRequest>,
);

static PROMPTS: OnceLock<Prompts> = OnceLock::new();
static PROMPT_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static REJECTIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn prompts() -> &'static Prompts {
    PROMPTS.get_or_init(async_channel::unbounded)
}

pub fn requests() -> async_channel::Receiver<HostKeyRequest> {
    prompts().1.clone()
}

fn known_hosts_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh").join("known_hosts"))
}

fn was_rejected(id: &str) -> bool {
    REJECTIONS
        .get_or_init(Default::default)
        .lock()
        .is_ok_and(|rejected| rejected.contains(id))
}

fn remember_rejection(id: String) {
    if let Ok(mut rejected) = REJECTIONS.get_or_init(Default::default).lock() {
        rejected.insert(id);
    }
}

fn refused_message(host: &str, port: u16) -> String {
    format!("The host key for {host}:{port} was not trusted, so the connection was refused")
}

enum KeyStatus {
    Trusted,
    Unknown,
    Conflicting(Vec<String>),
}

fn assess(host: &str, port: u16, key: &PublicKey, path: &PathBuf) -> Result<KeyStatus, String> {
    let recorded = russh::keys::known_hosts::known_host_keys_path(host, port, path)
        .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    if recorded.iter().any(|(_, stored)| stored == key) {
        return Ok(KeyStatus::Trusted);
    }
    if recorded.is_empty() {
        return Ok(KeyStatus::Unknown);
    }
    Ok(KeyStatus::Conflicting(
        recorded
            .iter()
            .map(|(_, stored)| {
                format!(
                    "{} {}",
                    stored.algorithm(),
                    stored.fingerprint(HashAlg::Sha256)
                )
            })
            .collect(),
    ))
}

pub async fn verify(host: &str, port: u16, key: &PublicKey) -> Result<(), String> {
    let Some(path) = known_hosts_path() else {
        return Err("Could not locate ~/.ssh/known_hosts to verify the host key".to_string());
    };

    if matches!(assess(host, port, key, &path)?, KeyStatus::Trusted) {
        return Ok(());
    }

    let _guard = PROMPT_LOCK.get_or_init(Default::default).lock().await;
    let stored = match assess(host, port, key, &path)? {
        KeyStatus::Trusted => return Ok(()),
        KeyStatus::Unknown => Vec::new(),
        KeyStatus::Conflicting(stored) => stored,
    };

    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    let id = format!("{host}:{port}/{fingerprint}");
    if was_rejected(&id) {
        return Err(refused_message(host, port));
    }

    let (reply, answer) = async_channel::bounded(1);
    let request = HostKeyRequest {
        host: host.to_string(),
        port,
        algorithm: key.algorithm().to_string(),
        fingerprint,
        stored,
        reply,
    };
    if prompts().0.send(request).await.is_err() {
        return Err("Could not ask about the unknown host key".to_string());
    }

    let trusted = matches!(
        tokio::time::timeout(PROMPT_TIMEOUT, answer.recv()).await,
        Ok(Ok(true))
    );
    if !trusted {
        remember_rejection(id);
        return Err(refused_message(host, port));
    }

    russh::keys::known_hosts::learn_known_hosts_path(host, port, key, &path)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))
}

pub fn open_prompt(request: HostKeyRequest, window: &mut Window, cx: &mut App) {
    if request.stored.is_empty() {
        open_unknown_prompt(request, window, cx)
    } else {
        open_mismatch_prompt(request, window, cx)
    }
}

fn open_unknown_prompt(request: HostKeyRequest, window: &mut Window, cx: &mut App) {
    let request = Rc::new(request);

    window.open_dialog(cx, move |dialog, _window, cx| {
        let request = request.clone();
        let muted = cx.theme().muted_foreground;

        dialog
            .w(px(520.))
            .title("Unknown host key")
            .child(
                v_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_start()
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .small()
                                    .text_color(cx.theme().warning),
                            )
                            .child(div().flex_1().min_w_0().text_sm().child(format!(
                                "{}:{} has never been connected to before, so its identity \
                                 cannot be confirmed.",
                                request.host, request.port
                            ))),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .p_3()
                            .rounded_md()
                            .bg(cx.theme().muted)
                            .child(detail("Algorithm", &request.algorithm, muted))
                            .child(detail("Fingerprint", &request.fingerprint, muted)),
                    )
                    .child(div().text_xs().text_color(muted).child(
                        "Continue only if this fingerprint matches the server you expect. \
                         Trusting it records the key in ~/.ssh/known_hosts, and a later \
                         mismatch will be refused.",
                    )),
            )
            .footer(
                DialogFooter::new()
                    .child(Button::new("reject").label("Reject").on_click({
                        let request = request.clone();
                        move |_, window, cx| {
                            request.answer(false);
                            window.close_dialog(cx);
                        }
                    }))
                    .child(
                        Button::new("trust")
                            .primary()
                            .label("Trust and save")
                            .on_click({
                                let request = request.clone();
                                move |_, window, cx| {
                                    request.answer(true);
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
    });
}

fn open_mismatch_prompt(request: HostKeyRequest, window: &mut Window, cx: &mut App) {
    let request = Rc::new(request);

    window.open_dialog(cx, move |dialog, _window, cx| {
        let request = request.clone();
        let muted = cx.theme().muted_foreground;

        dialog
            .w(px(560.))
            .title("Host key does not match")
            .child(
                v_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_start()
                            .p_2p5()
                            .rounded_md()
                            .border_1()
                            .border_color(cx.theme().danger.opacity(0.5))
                            .bg(cx.theme().danger.opacity(0.1))
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .small()
                                    .text_color(cx.theme().danger)
                                    .flex_none(),
                            )
                            .child(div().flex_1().min_w_0().text_sm().child(format!(
                                "{}:{} presented a key that matches none of the keys stored \
                                 for it. If something is intercepting this connection, this \
                                 is exactly what a machine-in-the-middle attack looks like.",
                                request.host, request.port
                            ))),
                    )
                    .child(div().flex_1().min_w_0().text_sm().child(
                        "It can also be legitimate: the same address can serve more than one \
                         system, such as a dropbear initramfs that unlocks an encrypted disk \
                         before the installed server boots.",
                    ))
                    .child(
                        v_flex()
                            .gap_1()
                            .p_3()
                            .rounded_md()
                            .bg(cx.theme().muted)
                            .child(detail("Offered", &request.algorithm, muted))
                            .child(detail("Fingerprint", &request.fingerprint, muted)),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .p_3()
                            .rounded_md()
                            .bg(cx.theme().muted)
                            .children(request.stored.iter().enumerate().map(|(ix, stored)| {
                                detail(if ix == 0 { "Stored" } else { "" }, stored, muted)
                            })),
                    )
                    .child(div().text_xs().text_color(muted).child(
                        "Verify the offered fingerprint against the server through another \
                         channel before accepting. Accepting adds this key alongside the \
                         stored ones in ~/.ssh/known_hosts, and both identities will be \
                         accepted from then on.",
                    )),
            )
            .footer(
                DialogFooter::new()
                    .child(Button::new("reject").primary().label("Reject").on_click({
                        let request = request.clone();
                        move |_, window, cx| {
                            request.answer(false);
                            window.close_dialog(cx);
                        }
                    }))
                    .child(
                        Button::new("trust-anyway")
                            .danger()
                            .label("I know what I'm doing")
                            .on_click({
                                let request = request.clone();
                                move |_, window, cx| {
                                    request.answer(true);
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
    });
}

fn detail(label: &'static str, value: &str, muted: Hsla) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_2()
        .items_start()
        .child(
            div()
                .w(px(76.))
                .flex_none()
                .text_xs()
                .text_color(muted)
                .child(label),
        )
        .child(div().flex_1().min_w_0().text_xs().child(value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
    const SECOND_KEY: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHA\
                              yNTYAAABBBEmKSENjQEezOmxkZMy7opKgwFB9nkt5YRrYMjNuG5N87uRgg6CLrb\
                              o5wAdT/y6v0mKV0U2w0WZ2YB/++Tpockg=";

    fn key(openssh: &str) -> PublicKey {
        PublicKey::from_openssh(openssh).expect("a well formed public key")
    }

    #[test]
    fn a_host_can_carry_more_than_one_trusted_key() {
        let dir = crate::tempdir::private_dir("oxidal-hosts").expect("dir");
        let path = dir.join("known_hosts");
        std::fs::write(&path, format!("vault.test {FIRST_KEY}\n")).expect("file");

        let sshd = key(FIRST_KEY);
        let dropbear = key(SECOND_KEY);

        assert!(
            matches!(assess("vault.test", 22, &sshd, &path), Ok(KeyStatus::Trusted)),
            "the recorded key must connect without a prompt"
        );
        assert!(
            matches!(assess("other.test", 22, &sshd, &path), Ok(KeyStatus::Unknown)),
            "a host with no entries is unknown, not conflicting"
        );
        match assess("vault.test", 22, &dropbear, &path) {
            Ok(KeyStatus::Conflicting(stored)) => {
                assert_eq!(stored.len(), 1, "every stored key is shown for comparison");
                assert!(stored[0].starts_with("ssh-ed25519 SHA256:"));
            }
            _ => panic!("a second identity must surface as a conflict, not an error"),
        }

        russh::keys::known_hosts::learn_known_hosts_path("vault.test", 22, &dropbear, &path)
            .expect("append");

        assert!(
            matches!(
                assess("vault.test", 22, &dropbear, &path),
                Ok(KeyStatus::Trusted)
            ),
            "once accepted, the second identity connects without a prompt"
        );
        assert!(
            matches!(assess("vault.test", 22, &sshd, &path), Ok(KeyStatus::Trusted)),
            "accepting the second identity must not evict the first"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_nonstandard_port_keeps_its_own_identities() {
        let dir = crate::tempdir::private_dir("oxidal-ports").expect("dir");
        let path = dir.join("known_hosts");
        std::fs::write(
            &path,
            format!("vault.test {FIRST_KEY}\n[vault.test]:2222 {SECOND_KEY}\n"),
        )
        .expect("file");

        let sshd = key(FIRST_KEY);
        let dropbear = key(SECOND_KEY);

        assert!(matches!(
            assess("vault.test", 2222, &dropbear, &path),
            Ok(KeyStatus::Trusted)
        ));
        assert!(
            matches!(
                assess("vault.test", 2222, &sshd, &path),
                Ok(KeyStatus::Conflicting(_))
            ),
            "port 22's key does not vouch for port 2222"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
