//! What the previewer shows of a file: the decoded samples, and the labels turned into rows of a transcript with a
//! time for every word. Nothing here knows the platform.

use bac::container::{Head, Stats, MAGIC};
use bac::labels::{Event, Labels, TIER_PHONE, TIER_TAG, TIER_TEXT, TIER_WORD};

/// A word of a row as it is shown, with the samples in which it is said.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub start: u64,
    pub end: u64,
    /// true: the time is from the file's `word` tier; false: worked out from the row's time and the word's length
    pub timed: bool,
}

/// A row of the transcript: one event of the `text` tier.
#[derive(Clone, Debug)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub speaker: Option<u32>,
    pub text: String,
    /// the labels of the tag events that overlap the row; none = neutral
    pub tags: Vec<String>,
    pub words: Vec<Word>,
    /// what search looks in
    pub lower: String,
}

pub struct Doc {
    pub path: String,
    pub name: String,
    pub file_bytes: usize,
    /// None for a WAV or FLAC file
    pub head: Option<Head>,
    pub stats: Option<Stats>,
    pub labels: Labels,
    pub rate: u32,
    pub channels: usize,
    pub bits: u8,
    /// samples per channel
    pub frames: u64,
    /// all channels, sample by sample, as the sound card takes them
    pub pcm: Vec<i16>,
    /// the channels mixed, for the pictures
    pub mono: Vec<i16>,
    pub segments: Vec<Segment>,
    /// every tag in the file once, in order of first use; "neutral" if a row has none
    pub tags: Vec<String>,
    /// whether the file has times for its words
    pub word_times: bool,
}

pub const NEUTRAL: &str = "neutral";

/// The samples of a BAC, WAV or FLAC file.
pub fn read_pcm(path: &str) -> Result<bac::Pcm, String> {
    let b = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let pcm = if b.starts_with(&MAGIC) {
        bac::decode(&b).map(|d| d.2)
    } else if b.starts_with(b"RIFF") {
        bac::pcm::read_wav(&b)
    } else {
        bac::flac::read_flac(&b)
    };
    pcm.map_err(|e| format!("{path}: {e}"))
}

impl Doc {
    pub fn open(path: &str) -> Result<Doc, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        Doc::from_bytes(path, &bytes).map_err(|e| format!("{path}: {e}"))
    }

    pub fn from_bytes(path: &str, b: &[u8]) -> Result<Doc, String> {
        let (pcm, labels, head, stats) = if b.starts_with(&MAGIC) {
            let (head, labels, pcm) = bac::decode(b).map_err(|e| e.to_string())?;
            let stats = bac::scan(b).map(|s| s.2).ok();
            (pcm, labels, Some(head), stats)
        } else if b.starts_with(b"RIFF") {
            (bac::pcm::read_wav(b).map_err(|e| e.to_string())?, Labels::default(), None, None)
        } else if b.starts_with(b"fLaC") || b.starts_with(b"ID3") {
            (bac::flac::read_flac(b).map_err(|e| e.to_string())?, Labels::default(), None, None)
        } else {
            return Err("not a BAC, WAV or FLAC file".into());
        };
        if pcm.ch.is_empty() || pcm.rate == 0 {
            return Err("no audio in the file".into());
        }
        let (frames, channels) = (pcm.len(), pcm.ch.len());
        let down = pcm.bits.saturating_sub(16);
        let mut inter = Vec::with_capacity(frames * channels);
        let mut mono = Vec::with_capacity(frames);
        for i in 0..frames {
            let mut sum = 0i32;
            for c in &pcm.ch {
                let v = (c[i] >> down).clamp(-32768, 32767);
                inter.push(v as i16);
                sum += v;
            }
            mono.push((sum / channels as i32) as i16);
        }
        let (segments, tags, word_times) = rows(&labels, frames as u64);
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
        Ok(Doc { path: path.into(), name, file_bytes: b.len(), head, stats, labels, rate: pcm.rate, channels, bits: pcm.bits, frames: frames as u64, pcm: inter, mono, segments, tags, word_times })
    }

    pub fn seconds(&self) -> f64 {
        self.frames as f64 / self.rate as f64
    }

    /// The row that holds sample `pos`: the last one that has started, if `pos` is before its end.
    pub fn segment_at(&self, pos: u64) -> Option<usize> {
        let i = self.segments.partition_point(|s| s.start <= pos).checked_sub(1)?;
        // rows may overlap (two speakers): prefer the latest started one that still runs
        (0..=i).rev().take(4).find(|&j| pos < self.segments[j].end)
    }

    pub fn codec(&self) -> String {
        match &self.head {
            Some(h) if h.params.lossless => "BAC lossless".into(),
            Some(h) => h.quality.map_or("BAC lossy".into(), |q| format!("BAC lossy, quality {q}")),
            None if self.path.to_ascii_lowercase().ends_with(".flac") => "FLAC".into(),
            None => "WAV".into(),
        }
    }

    /// (lowest, highest) sample of `mono` for each of `n` equal stretches of the file.
    pub fn peaks(&self, n: usize) -> Vec<(i16, i16)> {
        let len = self.mono.len();
        (0..n)
            .map(|i| {
                let (a, b) = (i * len / n.max(1), ((i + 1) * len / n.max(1)).max(i * len / n.max(1) + 1).min(len));
                self.mono.get(a..b).map_or((0, 0), |s| s.iter().fold((i16::MAX, i16::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v))))
            })
            .map(|(lo, hi)| if lo > hi { (0, 0) } else { (lo, hi) })
            .collect()
    }

    /// The spectrogram as `w` x `h` levels 0..255: time left to right, frequency bottom (0) to top (half the rate).
    pub fn spectrogram(&self, w: usize, h: usize) -> Vec<u8> {
        let size = if self.rate > 32_000 { 1024 } else { 512 };
        let win: Vec<f64> = (0..size).map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * (i as f64 + 0.5) / size as f64).cos()).collect();
        let mut out = vec![0u8; w * h];
        let (mut re, mut im) = (vec![0.0; size], vec![0.0; size]);
        for x in 0..w {
            let centre = (x as u64 * self.frames / w.max(1) as u64) as i64;
            for (i, (r, wv)) in re.iter_mut().zip(&win).enumerate() {
                let at = centre + i as i64 - size as i64 / 2;
                *r = if at >= 0 { self.mono.get(at as usize).copied().unwrap_or(0) as f64 / 32768.0 * wv } else { 0.0 };
            }
            im.fill(0.0);
            bac::metrics::fft(&mut re, &mut im);
            for y in 0..h {
                // the row's share of the bins, strongest bin of it
                let (a, b) = ((h - 1 - y) * (size / 2) / h, ((h - y) * (size / 2) / h).max((h - 1 - y) * (size / 2) / h + 1));
                let p = (a..b).map(|k| re[k] * re[k] + im[k] * im[k]).fold(0.0, f64::max);
                // -100 dB to -20 dB of a full-scale tone
                let db = 10.0 * (p / (size as f64 * size as f64 / 16.0) + 1e-12).log10();
                out[y * w + x] = (((db + 100.0) / 80.0).clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
        out
    }
}

/// Lower case letters and digits only: how a word is compared with a word of another tier.
fn plain(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

fn same_speaker(a: Option<u32>, b: Option<u32>) -> bool {
    a.is_none() || b.is_none() || a == b
}

/// The rows of the transcript, the file's tags, and whether words came with times.
fn rows(l: &Labels, frames: u64) -> (Vec<Segment>, Vec<String>, bool) {
    let is_tag = |e: &&Event| e.tier != TIER_TEXT && e.tier != TIER_WORD && e.tier != TIER_PHONE;
    let tag_events: Vec<&Event> = l.events.iter().filter(is_tag).collect();
    let label = |e: &Event| if e.tier == TIER_TAG { e.text.clone() } else { format!("{}: {}", e.tier, e.text) };
    let word_events: Vec<&Event> = l.tier(TIER_WORD).collect();
    let mut texts: Vec<Event> = l.tier(TIER_TEXT).cloned().collect();
    if texts.is_empty() {
        // no transcript: a file with tags only still gets a row per tag, so that they can be seen and jumped to
        texts = tag_events.iter().map(|e| Event { tier: TIER_TEXT.into(), text: String::new(), ..(*e).clone() }).collect();
    }
    let mut tags: Vec<String> = Vec::new();
    let mut any_neutral = false;
    let mut segments = Vec::with_capacity(texts.len());
    for t in &texts {
        let end = if t.end > t.start { t.end } else { frames.max(t.start) };
        let mine: Vec<String> = tag_events.iter().filter(|e| e.start < end && e.end.max(e.start + 1) > t.start && same_speaker(e.speaker, t.speaker)).map(|e| label(e)).collect();
        let mut row_tags: Vec<String> = Vec::new();
        for m in mine {
            if !row_tags.contains(&m) {
                row_tags.push(m);
            }
        }
        any_neutral |= row_tags.is_empty();
        for m in &row_tags {
            if !tags.contains(m) {
                tags.push(m.clone());
            }
        }
        let timed: Vec<&Event> = word_events.iter().copied().filter(|w| w.start >= t.start && w.start < end && same_speaker(w.speaker, t.speaker)).collect();
        let words = words_of(&t.text, t.start, end, &timed);
        segments.push(Segment { start: t.start, end, speaker: t.speaker, lower: t.text.to_lowercase(), text: t.text.clone(), tags: row_tags, words });
    }
    if any_neutral && !segments.is_empty() {
        tags.push(NEUTRAL.into());
    }
    (segments, tags, !word_events.is_empty())
}

/// The words of a row's text, each with its time. With as many timed words as words in the text they pair off in
/// order. Otherwise a word of the text takes the time of the next timed word that reads the same, and words left
/// without one share the gap between their neighbours by length. With no timed words at all, the row's own time is
/// shared out by length.
fn words_of(text: &str, start: u64, end: u64, timed: &[&Event]) -> Vec<Word> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut got: Vec<Option<(u64, u64)>> = vec![None; tokens.len()];
    if timed.len() == tokens.len() {
        for (g, w) in got.iter_mut().zip(timed) {
            *g = Some((w.start, w.end.max(w.start)));
        }
    } else {
        let mut next = 0;
        for (g, tok) in got.iter_mut().zip(&tokens) {
            let p = plain(tok);
            if let Some(j) = (next..timed.len().min(next + 4)).find(|&j| !p.is_empty() && plain(&timed[j].text) == p) {
                *g = Some((timed[j].start, timed[j].end.max(timed[j].start)));
                next = j + 1;
            }
        }
    }
    // fill the runs that have no time: between the neighbours' times, by the number of letters
    let mut words = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if let Some((a, b)) = got[i] {
            words.push(Word { text: tokens[i].into(), start: a, end: b, timed: true });
            i += 1;
            continue;
        }
        let run_end = (i..tokens.len()).find(|&j| got[j].is_some()).unwrap_or(tokens.len());
        let from = if i == 0 { start } else { words[i - 1].end };
        let to = got.get(run_end).copied().flatten().map_or(end, |g| g.0).max(from);
        let letters: usize = tokens[i..run_end].iter().map(|t| t.chars().count() + 1).sum();
        let mut at = from;
        let mut done = 0;
        for tok in &tokens[i..run_end] {
            done += tok.chars().count() + 1;
            let stop = from + (to - from) * done as u64 / letters.max(1) as u64;
            words.push(Word { text: (*tok).into(), start: at, end: stop, timed: false });
            at = stop;
        }
        i = run_end;
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(tier: &str, start: u64, end: u64, text: &str) -> Event {
        Event { tier: tier.into(), start, end, speaker: None, weight: None, text: text.into(), attrs: vec![] }
    }

    #[test]
    fn words_pair_off_with_their_times() {
        let mut l = Labels::default();
        l.add(ev(TIER_TEXT, 0, 3000, "okay so like, how"));
        for (i, w) in ["Okay,", "so", "like", "how"].iter().enumerate() {
            l.add(ev(TIER_WORD, i as u64 * 700, i as u64 * 700 + 500, w));
        }
        let (rows, tags, timed) = rows(&l, 3000);
        assert!(timed && tags == [NEUTRAL]);
        let w = &rows[0].words;
        assert_eq!(w.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["okay", "so", "like,", "how"]);
        assert_eq!((w[2].start, w[2].end, w[2].timed), (1400, 1900, true));
    }

    #[test]
    fn a_missing_word_takes_the_gap() {
        // the aligner merged "do not" into "don't": three timed words for four in the text
        let timed = [ev(TIER_WORD, 100, 200, "I"), ev(TIER_WORD, 200, 600, "don't"), ev(TIER_WORD, 900, 1200, "know")];
        let refs: Vec<&Event> = timed.iter().collect();
        let w = words_of("I do not know.", 0, 1300, &refs);
        assert_eq!(w.len(), 4);
        assert!(w[0].timed && w[3].timed && !w[1].timed && !w[2].timed);
        assert_eq!((w[0].start, w[3].start), (100, 900));
        assert!(w[1].start == 200 && w[1].end == w[2].start && w[2].end == 900);
    }

    #[test]
    fn without_word_times_the_row_is_shared_by_length() {
        let w = words_of("a longer word", 1000, 2000, &[]);
        assert_eq!((w[0].start, w[2].end), (1000, 2000));
        assert!(w[1].end - w[1].start > w[0].end - w[0].start && w.iter().all(|w| !w.timed));
        assert!(words_of("", 0, 10, &[]).is_empty());
    }

    #[test]
    fn tags_land_on_the_rows_they_overlap() {
        let mut l = Labels::default();
        l.add(ev(TIER_TEXT, 0, 1000, "one"));
        l.add(ev(TIER_TEXT, 1000, 2000, "two"));
        l.add(ev(TIER_TAG, 900, 1500, "whisper"));
        l.add(ev("emotion", 1200, 1300, "glad"));
        let (rows, tags, _) = rows(&l, 2000);
        assert_eq!(rows[0].tags, ["whisper"]);
        assert_eq!(rows[1].tags, ["whisper", "emotion: glad"]);
        assert_eq!(tags, ["whisper", "emotion: glad"]);
        // tags alone still make rows
        let mut only = Labels::default();
        only.add(ev(TIER_TAG, 5, 50, "laughing"));
        assert_eq!(super::rows(&only, 100).0.len(), 1);
    }

    #[test]
    fn a_file_opens_and_finds_its_row() {
        let n = 16_000;
        let pcm = bac::Pcm { rate: 16_000, bits: 16, ch: vec![(0..n).map(|i| (i % 100 - 50) * 200).collect()] };
        let mut l = Labels::default();
        l.add(ev(TIER_TEXT, 0, 8000, "first row"));
        l.add(ev(TIER_TEXT, 8000, 16_000, "second row"));
        let file = bac::encode(&pcm, &bac::EncOpts { lossless: true, ..Default::default() }, &l).unwrap();
        let d = Doc::from_bytes("x/test.bac", &file).unwrap();
        assert_eq!((d.name.as_str(), d.frames, d.channels, d.pcm.len()), ("test.bac", 16_000, 1, 16_000));
        assert_eq!((d.segment_at(0), d.segment_at(7999), d.segment_at(8000), d.segment_at(20_000)), (Some(0), Some(0), Some(1), None));
        assert_eq!(d.peaks(10).len(), 10);
        assert_eq!(d.spectrogram(20, 16).len(), 320);
        assert_eq!(d.codec(), "BAC lossless");
        assert!(Doc::from_bytes("x", b"nothing").is_err());
    }
}
