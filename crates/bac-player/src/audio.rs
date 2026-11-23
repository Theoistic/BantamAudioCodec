//! Sound out: a thread that feeds the samples to the default sound device (waveOut) in buffers of 30 ms, and says
//! which sample is being heard right now. The window only sets wishes (play, pause, jump, volume) and reads the
//! position. Without a sound device the position runs by the clock, so that everything else still works.

use crate::win32::*;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Instant;

struct Shared {
    pcm: Vec<i16>,
    channels: usize,
    rate: u32,
    frames: u64,
    /// the window's wishes
    playing: AtomicBool,
    /// a frame to jump to, or -1
    seek: AtomicI64,
    /// 0..1000
    volume: AtomicU32,
    quit: AtomicBool,
    /// the thread's answers: the frame being heard, whether a device is open
    pos: AtomicU64,
    device: AtomicBool,
    /// feed zeros to the device (tests that must not make a sound)
    mute: bool,
}

pub struct Player {
    s: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Player {
    pub fn new(pcm: Vec<i16>, channels: usize, rate: u32, mute: bool) -> Player {
        let frames = (pcm.len() / channels.max(1)) as u64;
        let s = Arc::new(Shared {
            pcm,
            channels,
            rate,
            frames,
            playing: AtomicBool::new(false),
            seek: AtomicI64::new(-1),
            volume: AtomicU32::new(1000),
            quit: AtomicBool::new(false),
            pos: AtomicU64::new(0),
            device: AtomicBool::new(false),
            mute,
        });
        let t = s.clone();
        let thread = std::thread::Builder::new().name("bac-audio".into()).spawn(move || run(&t)).ok();
        Player { s, thread }
    }

    pub fn playing(&self) -> bool {
        self.s.playing.load(Relaxed)
    }

    /// Starts or stops. At the end of the file, play starts over.
    pub fn set_playing(&self, on: bool) {
        if on && self.position() >= self.s.frames {
            self.seek(0);
        }
        self.s.playing.store(on, Relaxed);
    }

    pub fn seek(&self, frame: u64) {
        let f = frame.min(self.s.frames);
        self.s.seek.store(f as i64, Relaxed);
        self.s.pos.store(f, Relaxed);
    }

    /// The frame being heard.
    pub fn position(&self) -> u64 {
        let s = self.s.seek.load(Relaxed);
        if s >= 0 {
            s as u64
        } else {
            self.s.pos.load(Relaxed)
        }
    }

    pub fn set_volume(&self, v: f32) {
        self.s.volume.store((v.clamp(0.0, 1.0) * 1000.0) as u32, Relaxed);
    }

    pub fn has_device(&self) -> bool {
        self.s.device.load(Relaxed)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.s.quit.store(true, Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

const BUFFERS: usize = 3;

fn run(s: &Shared) {
    let per = (s.rate as usize * 30 / 1000).max(64);
    let fmt = WAVEFORMATEX {
        wFormatTag: 1,
        nChannels: s.channels as u16,
        nSamplesPerSec: s.rate,
        nAvgBytesPerSec: s.rate * s.channels as u32 * 2,
        nBlockAlign: (s.channels * 2) as u16,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    unsafe {
        let event = CreateEventW(std::ptr::null(), 0, 0, std::ptr::null());
        let mut dev: HANDLE = std::ptr::null_mut();
        let open = waveOutOpen(&mut dev, WAVE_MAPPER, &fmt, event as usize, 0, CALLBACK_EVENT) == 0;
        s.device.store(open, Relaxed);
        if !open {
            by_the_clock(s);
            CloseHandle(event);
            return;
        }
        let mut data: Vec<Vec<i16>> = (0..BUFFERS).map(|_| vec![0i16; per * s.channels]).collect();
        let mut hdrs: Vec<WAVEHDR> = data
            .iter_mut()
            .map(|d| WAVEHDR { lpData: d.as_mut_ptr() as *mut u8, dwBufferLength: 0, dwBytesRecorded: 0, dwUser: 0, dwFlags: 0, dwLoops: 0, lpNext: std::ptr::null_mut(), reserved: 0 })
            .collect();
        let size = std::mem::size_of::<WAVEHDR>() as u32;
        // `next`: the frame the next buffer starts with; `base`: the frame the device's position 0 stands for
        let (mut next, mut base) = (0u64, 0u64);
        let mut running = true;
        loop {
            WaitForSingleObject(event, 8);
            if s.quit.load(Relaxed) {
                break;
            }
            let jump = s.seek.swap(-1, Relaxed);
            if jump >= 0 {
                // drops what is queued and sets the device's position back to 0. Whether a paused device stays
                // paused is not promised, so the state is set again below
                waveOutReset(dev);
                next = jump as u64;
                base = next;
                s.pos.store(next, Relaxed);
            }
            let want = s.playing.load(Relaxed);
            if want != running || jump >= 0 {
                if want {
                    waveOutRestart(dev);
                } else {
                    waveOutPause(dev);
                }
                running = want;
            }
            let mut queued = false;
            for (h, d) in hdrs.iter_mut().zip(data.iter_mut()) {
                if h.dwFlags & WHDR_INQUEUE != 0 {
                    queued = true;
                    continue;
                }
                if !want || next >= s.frames {
                    continue;
                }
                if h.dwFlags != 0 {
                    waveOutUnprepareHeader(dev, h, size);
                }
                let n = (per as u64).min(s.frames - next) as usize;
                let vol = if s.mute { 0 } else { s.volume.load(Relaxed) as i32 };
                let src = &s.pcm[next as usize * s.channels..(next as usize + n) * s.channels];
                for (o, &v) in d.iter_mut().zip(src) {
                    *o = (v as i32 * vol / 1000) as i16;
                }
                h.dwBufferLength = (n * s.channels * 2) as u32;
                h.dwFlags = 0;
                waveOutPrepareHeader(dev, h, size);
                waveOutWrite(dev, h, size);
                next += n as u64;
                queued = true;
            }
            let mut t = MMTIME { wType: TIME_SAMPLES, value: 0, rest: 0 };
            if s.seek.load(Relaxed) < 0 && waveOutGetPosition(dev, &mut t, std::mem::size_of::<MMTIME>() as u32) == 0 {
                let played = match t.wType {
                    TIME_SAMPLES => t.value as u64,
                    TIME_BYTES => t.value as u64 / (s.channels as u64 * 2),
                    _ => 0,
                };
                s.pos.store((base + played).min(next), Relaxed);
            }
            if want && !queued && next >= s.frames && s.seek.load(Relaxed) < 0 {
                // the end: stop there
                s.pos.store(s.frames, Relaxed);
                s.playing.store(false, Relaxed);
            }
        }
        waveOutReset(dev);
        for h in hdrs.iter_mut() {
            if h.dwFlags != 0 {
                waveOutUnprepareHeader(dev, h, size);
            }
        }
        waveOutClose(dev);
        CloseHandle(event);
    }
}

/// No sound device: the position follows the clock while playing.
fn by_the_clock(s: &Shared) {
    let mut last = Instant::now();
    let mut pos = 0f64;
    while !s.quit.load(Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(8));
        let now = Instant::now();
        let jump = s.seek.swap(-1, Relaxed);
        if jump >= 0 {
            pos = jump as f64;
        } else if s.playing.load(Relaxed) {
            pos += (now - last).as_secs_f64() * s.rate as f64;
            if pos >= s.frames as f64 {
                pos = s.frames as f64;
                s.playing.store(false, Relaxed);
            }
        }
        last = now;
        s.pos.store(pos as u64, Relaxed);
    }
}
