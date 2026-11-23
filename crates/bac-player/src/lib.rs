//! The BAC previewer: a window that plays a file and shows what the file says about its audio. The waveform or
//! spectrogram with the playing position, the speakers, the tags, and the transcript with the word being said lit
//! up. It also opens WAV and FLAC files, which have no labels.
//!
//! | File | What it holds |
//! |---|---|
//! | `doc.rs` | a file opened: samples, and the labels as rows with a time for every word |
//! | `view.rs` | the state, the drawing of every frame, what a click or key does |
//! | `canvas.rs` | software drawing of shapes; a PNG writer for snapshots |
//! | `icon.rs` | the program's icon, drawn by the canvas, as `.ico` and `.res` |
//! | `surface.rs` | the pixels with the system's text on them |
//! | `audio.rs` | sound out on its own thread, and the playing position |
//! | `window.rs`, `win32.rs` | the window and its menus and dialogs; the Win32 declarations |
//!
//! No dependencies but the `bac` crate. Windows only so far: `doc` and `canvas` are free of the platform.

// drawing code reads best with the index in sight
#![allow(clippy::needless_range_loop)]

pub mod canvas;
pub mod doc;
pub mod icon;

#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod surface;
#[cfg(windows)]
mod view;
#[cfg(windows)]
mod win32;
#[cfg(windows)]
mod window;

/// How to start the previewer.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// the file to open
    pub file: Option<String>,
    /// start playing at once
    pub autoplay: bool,
    /// send silence to the sound device (for tests that must not make a sound)
    pub mute: bool,
    /// close by itself after this many seconds and print where the playing position was
    pub quit_after: Option<f64>,
}

/// What a snapshot shows.
#[derive(Clone, Debug)]
pub struct Shot {
    pub file: Option<String>,
    /// the playing position in seconds
    pub at: f64,
    pub width: i32,
    pub height: i32,
    pub scale: f32,
    pub spectrogram: bool,
    pub search: String,
    /// show the file's details in place of the transcript
    pub details: bool,
    /// where the mouse rests, in pixels at scale 1
    pub mouse: Option<(f32, f32)>,
}

impl Default for Shot {
    fn default() -> Self {
        Shot { file: None, at: 0.0, width: 1090, height: 820, scale: 1.0, spectrogram: false, search: String::new(), details: false, mouse: None }
    }
}

/// Opens the window and returns when it is closed.
pub fn run(o: Options) -> Result<(), String> {
    #[cfg(windows)]
    return window::run(o);
    #[cfg(not(windows))]
    {
        let _ = o;
        Err("the previewer runs on Windows only so far".into())
    }
}

/// Draws the previewer as it would look and writes the picture as a PNG file. No window is opened.
pub fn snapshot(shot: &Shot, out: &str) -> Result<(), String> {
    #[cfg(windows)]
    return window::snapshot(shot, out);
    #[cfg(not(windows))]
    {
        let _ = (shot, out);
        Err("the previewer runs on Windows only so far".into())
    }
}

/// A program that is both a command-line tool and a window, started by a double click, gets a console of its own:
/// this closes it. A console shared with a shell is left alone. Returns whether it closed one.
pub fn leave_own_console() -> bool {
    #[cfg(windows)]
    unsafe {
        let mut pids = [0u32; 2];
        if !win32::GetConsoleWindow().is_null() && win32::GetConsoleProcessList(pids.as_mut_ptr(), 2) == 1 {
            return win32::FreeConsole() != 0;
        }
    }
    false
}
