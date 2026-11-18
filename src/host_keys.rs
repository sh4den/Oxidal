use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{App, Hsla, IntoElement, ParentElement as _, Styled as _, Window, div, px};
use russh::keys::ssh_key::{HashAlg, PublicKey};

const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);
const REJECTION_TTL: Duration = Duration::from_secs(8);

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
static REJECTIONS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

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
        .is_ok_and(|rejected| {
            rejected
                .get(id)
                .is_some_and(|at| at.elapsed() < REJECTION_TTL)
        })
}

fn remember_rejection(id: String) {
    if let Ok(mut rejected) = REJECTIONS.get_or_init(Default::default).lock() {
        rejected.retain(|_, at| at.elapsed() < REJECTION_TTL);
        rejected.insert(id, Instant::now());
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

pub fn open_prompt(request: HostKeyRequest, _window: &mut Window, cx: &mut App) {
    if request.stored.is_empty() {
        open_unknown_prompt(request, cx)
    } else {
        open_mismatch_prompt(request, cx)
    }
}

fn open_unknown_prompt(request: HostKeyRequest, cx: &mut App) {
    let request = Rc::new(request);

    crate::dialog_window::open(
        "Unknown host key",
        px(520.),
        cx,
        move |dialog, _window, cx| {
            let request = request.clone();
            let muted = cx.theme().muted_foreground;

            dialog
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
                .button(Button::new("reject").label("Reject").on_click({
                    let request = request.clone();
                    move |_, window, _| {
                        request.answer(false);
                        window.remove_window();
                    }
                }))
                .button(
                    Button::new("trust")
                        .primary()
                        .label("Trust and save")
                        .on_click({
                            let request = request.clone();
                            move |_, window, _| {
                                request.answer(true);
                                window.remove_window();
                            }
                        }),
                )
        },
    );
}

fn open_mismatch_prompt(request: HostKeyRequest, cx: &mut App) {
    let request = Rc::new(request);

    crate::dialog_window::open(
        "Host key does not match",
        px(560.),
        cx,
        move |dialog, _window, cx| {
            let request = request.clone();
            let muted = cx.theme().muted_foreground;

            dialog
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
                .button(Button::new("reject").primary().label("Reject").on_click({
                    let request = request.clone();
                    move |_, window, _| {
                        request.answer(false);
                        window.remove_window();
                    }
                }))
                .button(
                    Button::new("trust-anyway")
                        .danger()
                        .label("I know what I'm doing")
                        .on_click({
                            let request = request.clone();
                            move |_, window, _| {
                                request.answer(true);
                                window.remove_window();
                            }
                        }),
                )
        },
    );
}

pub(crate) fn detail(label: &'static str, value: &str, muted: Hsla) -> impl IntoElement {
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
#[path = "../tests/host_keys.rs"]
mod tests;
