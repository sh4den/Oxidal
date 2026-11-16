use gpui_kit::component::dialog::Confirm;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{App, Entity, Window};

/// Text inputs keep Enter for themselves, so a dialog never sees it. This hands
/// it on as the dialog's confirm action, which runs its `on_ok`.
pub fn confirm_on_enter(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    window
        .subscribe(input, cx, |_, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
            }
        })
        .detach();
}
