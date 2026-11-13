//! Keep visualizer lifecycle independent of the player's root window.
use eframe::egui::{self, ViewportCommand, ViewportId};

pub fn id() -> ViewportId {
    ViewportId::from_hash_of("kaleido-visualizer")
}

/// Runs from App::logic, including while macOS fullscreen occludes the player.
/// Returns true only when a visualizer close was consumed.
pub fn handle_close(ctx: &egui::Context, open: &mut bool) -> bool {
    if !*open
        || !ctx.input(|input| {
            input
                .raw
                .viewports
                .get(&id())
                .is_some_and(|viewport| viewport.close_requested())
        })
    {
        return false;
    }
    *open = false;
    // Consume the child close explicitly. Stop declaring the child viewport on
    // the next root pass to dispose of it; never send Close to the root window.
    ctx.send_viewport_cmd_to(id(), ViewportCommand::CancelClose);
    ctx.send_viewport_cmd_to(id(), ViewportCommand::Fullscreen(false));
    // A fullscreen child can leave the player occluded during Space teardown.
    // Cancel a propagated close for this pass and bring the player back.
    for command in [
        ViewportCommand::CancelClose,
        ViewportCommand::Visible(true),
        ViewportCommand::Minimized(false),
        ViewportCommand::Focus,
    ] {
        ctx.send_viewport_cmd_to(ViewportId::ROOT, command);
    }
    ctx.request_repaint_of(ViewportId::ROOT);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(child_close: bool, root_close: bool, fullscreen: bool) -> egui::RawInput {
        let mut input = egui::RawInput::default();
        input.viewports.get_mut(&ViewportId::ROOT).unwrap().occluded = Some(fullscreen);
        if root_close {
            input
                .viewports
                .get_mut(&ViewportId::ROOT)
                .unwrap()
                .events
                .push(egui::ViewportEvent::Close);
        }
        input.viewports.insert(
            id(),
            egui::ViewportInfo {
                parent: Some(ViewportId::ROOT),
                fullscreen: Some(fullscreen),
                events: if child_close {
                    vec![egui::ViewportEvent::Close]
                } else {
                    vec![]
                },
                ..Default::default()
            },
        );
        input
    }

    #[test]
    fn closing_fullscreen_visualizer_restores_player_even_without_a_ui_pass() {
        for fullscreen in [false, true] {
            let ctx = egui::Context::default();
            let mut open = true;
            // Include a propagated root close to cover the app-exit regression.
            let output = ctx.run_logic(&input(true, true, fullscreen), |ctx| {
                assert!(handle_close(ctx, &mut open));
            });
            assert!(!open);
            let root = &output.viewport_commands[&ViewportId::ROOT];
            assert!(root.contains(&ViewportCommand::CancelClose));
            assert!(root.contains(&ViewportCommand::Visible(true)));
            assert!(root.contains(&ViewportCommand::Focus));
            assert!(!root.contains(&ViewportCommand::Close));
            let child = &output.viewport_commands[&id()];
            assert!(child.contains(&ViewportCommand::CancelClose));
            assert!(child.contains(&ViewportCommand::Fullscreen(false)));
        }
    }

    #[test]
    fn actual_player_close_remains_allowed() {
        let ctx = egui::Context::default();
        let mut open = true;
        let output = ctx.run_logic(&input(false, true, true), |ctx| {
            assert!(!handle_close(ctx, &mut open));
        });
        assert!(open);
        assert!(output.viewport_commands.is_empty());
    }

    #[test]
    fn child_close_is_consumed_once() {
        let ctx = egui::Context::default();
        let mut open = true;
        let input = input(true, false, true);
        let _ = ctx.run_logic(&input, |ctx| {
            assert!(handle_close(ctx, &mut open));
        });
        let output = ctx.run_logic(&input, |ctx| {
            assert!(!handle_close(ctx, &mut open));
        });
        assert!(output.viewport_commands.is_empty());
    }
}
