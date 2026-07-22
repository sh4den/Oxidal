use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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

const REJECTION_MEMORY: Duration = Duration::from_secs(300);

fn prompts() -> &'static Prompts {
    PROMPTS.get_or_init(async_channel::unbounded)
}

pub fn requests() -> async_channel::Receiver<HostKeyRequest> {
    prompts().1.clone()
}

fn known_hosts_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh").join("known_hosts"))
}

fn rejected_since(id: &str, started: Instant) -> bool {
    REJECTIONS
        .get_or_init(Default::default)
        .lock()
        .ok()
        .and_then(|map| map.get(id).copied())
        .is_some_and(|at| at >= started)
}

fn remember_rejection(id: String) {
    let Ok(mut map) = REJECTIONS.get_or_init(Default::default).lock() else {
        return;
    };
    let now = Instant::now();
    map.retain(|_, at| now.duration_since(*at) < REJECTION_MEMORY);
    map.insert(id, now);
}

fn refused_message(host: &str, port: u16) -> String {
    format!("The host key for {host}:{port} was not trusted, so the connection was refused")
}

fn lookup(host: &str, port: u16, key: &PublicKey, path: &PathBuf) -> Result<bool, String> {
    match russh::keys::check_known_hosts_path(host, port, key, path) {
        Ok(known) => Ok(known),
        Err(russh::keys::Error::KeyChanged { line }) => Err(mismatch_message(host, port, line)),
        Err(e) => Err(format!("Could not read {}: {e}", path.display())),
    }
}

pub async fn verify(host: &str, port: u16, key: &PublicKey) -> Result<(), String> {
    let started = Instant::now();
    let Some(path) = known_hosts_path() else {
        return Err("Could not locate ~/.ssh/known_hosts to verify the host key".to_string());
    };

    if lookup(host, port, key, &path)? {
        return Ok(());
    }

    let _guard = PROMPT_LOCK.get_or_init(Default::default).lock().await;
    if lookup(host, port, key, &path)? {
        return Ok(());
    }

    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    let id = format!("{host}:{port}/{fingerprint}");
    if rejected_since(&id, started) {
        return Err(refused_message(host, port));
    }

    let (reply, answer) = async_channel::bounded(1);
    let request = HostKeyRequest {
        host: host.to_string(),
        port,
        algorithm: key.algorithm().to_string(),
        fingerprint,
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

fn mismatch_message(host: &str, port: u16, line: usize) -> String {
    let entry = if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    };
    format!(
        "Host key verification failed for {host}:{port}.\n\nThe server offered a key that does \
         not match the one stored on line {line} of ~/.ssh/known_hosts. This happens when a \
         server is rebuilt, and it also happens when a connection is being intercepted.\n\nIf \
         you are certain the change is expected, drop the stored key with:\n    ssh-keygen -R \
         \"{entry}\""
    )
}

pub fn open_prompt(request: HostKeyRequest, window: &mut Window, cx: &mut App) {
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
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .child(value.to_string()),
        )
}
