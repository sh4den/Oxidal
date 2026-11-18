use gpui_kit::{Context, Window};

use gpui_kit::component::{WindowExt as _, notification::Notification};

use super::{OxidalApp, UpdateState};

impl OxidalApp {
    pub(super) fn start_update_download(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Available(found) = &self.update_state else {
            return;
        };
        let found = found.clone();
        let download = crate::update::download(found.clone());
        self.update_state = UpdateState::Downloading(found);
        cx.notify();
        cx.spawn(async move |this, cx| {
            if let Ok(result) = download.recv().await {
                let _ = this.update(cx, |app, cx| {
                    let previous = std::mem::replace(&mut app.update_state, UpdateState::Idle);
                    app.update_state = match (result, previous) {
                        (Ok(path), _) => UpdateState::Ready(path),
                        (Err(_), UpdateState::Downloading(found)) => UpdateState::Available(found),
                        (Err(_), other) => other,
                    };
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn restart_to_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let UpdateState::Ready(path) = &self.update_state else {
            return;
        };
        match crate::update::apply_and_restart(path) {
            Ok(()) => cx.quit(),
            Err(e) => {
                window.push_notification(Notification::error(format!("Update failed: {e}")), cx);
            }
        }
    }
}
