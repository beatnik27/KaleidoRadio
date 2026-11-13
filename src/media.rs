//! Native macOS media commands and output-route monitoring. Call on the UI thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Play,
    Stop,
    Toggle,
    Next,
    Previous,
    Disconnected,
    OutputChanged,
}

impl Command {
    #[cfg(any(target_os = "macos", test))]
    fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            1 => Self::Play,
            2 => Self::Stop,
            3 => Self::Toggle,
            4 => Self::Next,
            5 => Self::Previous,
            6 => Self::Disconnected,
            7 => Self::OutputChanged,
            _ => return None,
        })
    }
    pub fn resolve(self, active: bool) -> Self {
        if self == Self::Toggle {
            if active { Self::Stop } else { Self::Play }
        } else {
            self
        }
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::Command;
    use std::{
        ffi::{CString, c_char, c_void},
        sync::mpsc::{self, Receiver, Sender},
    };
    unsafe extern "C" {
        fn KRMediaCreate(
            callback: extern "C" fn(*mut c_void, i32),
            context: *mut c_void,
        ) -> *mut c_void;
        fn KRMediaUpdate(
            handle: *mut c_void,
            playing: bool,
            title: *const c_char,
            station: *const c_char,
            live: bool,
            elapsed: f64,
            duration: f64,
        );
        fn KRMediaDestroy(handle: *mut c_void);
    }
    extern "C" fn receive(context: *mut c_void, code: i32) {
        let Some(command) = Command::from_code(code) else {
            return;
        };
        // Context is owned by Bridge and stays alive until native callbacks are removed.
        let sender = unsafe { &*(context as *const Sender<Command>) };
        let _ = sender.send(command);
    }
    pub struct Bridge {
        handle: *mut c_void,
        _sender: Box<Sender<Command>>,
        receiver: Receiver<Command>,
    }
    impl Bridge {
        pub fn new() -> Self {
            let (tx, receiver) = mpsc::channel();
            let mut sender = Box::new(tx);
            let handle =
                unsafe { KRMediaCreate(receive, (&mut *sender as *mut Sender<Command>).cast()) };
            Self {
                handle,
                _sender: sender,
                receiver,
            }
        }
        pub fn commands(&self) -> Vec<Command> {
            self.receiver.try_iter().collect()
        }
        pub fn update(
            &self,
            playing: bool,
            title: &str,
            station: &str,
            live: bool,
            elapsed: f64,
            duration: f64,
        ) {
            let title = CString::new(title.replace('\0', "")).unwrap();
            let station = CString::new(station.replace('\0', "")).unwrap();
            unsafe {
                KRMediaUpdate(
                    self.handle,
                    playing,
                    title.as_ptr(),
                    station.as_ptr(),
                    live,
                    elapsed,
                    duration,
                );
            }
        }
    }
    impl Drop for Bridge {
        fn drop(&mut self) {
            unsafe {
                KRMediaDestroy(self.handle);
            }
        }
    }
}
#[cfg(target_os = "macos")]
pub use native::Bridge;
#[cfg(not(target_os = "macos"))]
pub struct Bridge;
#[cfg(not(target_os = "macos"))]
impl Bridge {
    pub fn new() -> Self {
        Self
    }
    pub fn commands(&self) -> Vec<Command> {
        vec![]
    }
    pub fn update(&self, _: bool, _: &str, _: &str, _: bool, _: f64, _: f64) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn headphones_toggle_play_and_stop_and_pause_is_stop() {
        assert_eq!(Command::from_code(2), Some(Command::Stop));
        assert_eq!(Command::Toggle.resolve(true), Command::Stop);
        assert_eq!(Command::Toggle.resolve(false), Command::Play);
        assert_eq!(Command::Play.resolve(true), Command::Play);
        assert_eq!(Command::from_code(99), None);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_route_loss_stops_without_switching_to_speakers() {
        unsafe extern "C" {
            fn KRMediaShouldStopRoute(
                playing: bool,
                monitored: u32,
                current: u32,
                alive: bool,
            ) -> bool;
        }
        unsafe {
            assert!(KRMediaShouldStopRoute(true, 42, 7, true));
            assert!(KRMediaShouldStopRoute(true, 42, 42, false));
            assert!(KRMediaShouldStopRoute(true, 42, 0, false));
            assert!(!KRMediaShouldStopRoute(true, 42, 42, true));
            assert!(!KRMediaShouldStopRoute(true, 0, 7, true));
            assert!(!KRMediaShouldStopRoute(false, 42, 7, false));
        }
    }
}
