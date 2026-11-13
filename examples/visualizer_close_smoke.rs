//! Manual native regression: open the child, enter fullscreen, then close it.
//! The player window must remain usable. Does not load or change player settings.
#[path = "../src/visualizer_window.rs"]
mod visualizer_window;
use eframe::egui;
struct Smoke {
    open: bool,
}
impl eframe::App for Smoke {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        ctx.input(|i| {
            for (id, info) in &i.raw.viewports {
                if info.close_requested() {
                    eprintln!(
                        "close {id:?} fullscreen={:?} focused={:?}",
                        info.fullscreen, info.focused
                    );
                }
            }
        });
        if visualizer_window::handle_close(ctx, &mut self.open) {
            eprintln!("handled visualizer close");
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        ui.heading("Player stays open");
        ui.label(if self.open {
            "Visualizer open"
        } else {
            "Visualizer closed; player alive"
        });
        if ui.button("Open visualizer").clicked() {
            self.open = true;
        }
        if self.open {
            ui.ctx().show_viewport_immediate(visualizer_window::id(),
                egui::ViewportBuilder::default().with_title("Fullscreen close regression").with_inner_size([600., 400.]),
                |ctx, _| {
                    if visualizer_window::handle_close(ctx, &mut self.open) { return; }
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.heading("Visualizer");
                        ui.label("Enter fullscreen with the native green window button, then close this window.");
                        if ui.button("Enter fullscreen").clicked() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(true)); }
                        if ui.button("Close visualizer").clicked() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close); }
                    });
                });
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        eprintln!("PLAYER EXIT");
    }
}
fn main() -> eframe::Result {
    use std::os::fd::AsRawFd;
    let log = std::fs::File::create("/tmp/kaleido-fullscreen-close.log").unwrap();
    unsafe {
        libc::dup2(log.as_raw_fd(), libc::STDERR_FILENO);
    }
    eframe::run_native(
        "Visualizer close test player",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([420., 240.]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(Smoke { open: true }))),
    )
}
