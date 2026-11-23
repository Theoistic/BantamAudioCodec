//! The container: a file or stream is the four bytes "BAC1" and then units.
//!
//! ```text
//! unit = 0xBA  type  length (varint)  payload  CRC-32 of type, length and payload (little endian)
//! ```
//!
//! | type | unit   | payload |
//! |-----:|--------|---------|
//! |    1 | HEAD   | what a decoder must know (`Params`), the number of samples if known |
//! |    2 | META   | attributes and speakers (`labels`) |
//! |    3 | EVENTS | events on the time line: transcript, alignments, range tags (`labels`) |
//! |    4 | AUDIO  | a packet: some frames of every channel under one range coder |
//! |    5 | INDEX  | where the label units and the key packets are |
//! | 0x7F | END    | where the INDEX is, and the number of samples; always the last 23 bytes of a file |
//!
//! A reader takes units as they come and needs nothing from the end of the file: a file plays while it downloads. A
//! unit of a type it does not know is skipped. After damage, or when joining a live stream, it looks for the next
//! 0xBA whose checksum holds and starts decoding at the next key packet. The INDEX and END units serve readers that
//! can seek. Label units may come anywhere and add up: a finished file has them before the audio, a live stream
//! sends them as things are said.

use crate::codec::{Decoder, Params, MAX_PACKET_FRAMES};
use crate::encoder::{EncOpts, Encoder};
use crate::error::{bad, Error, Result};
use crate::labels::Labels;
use crate::pcm::Pcm;
use crate::wire::{crc32, put_varint, Cursor};
use std::io::{Read, Seek, SeekFrom, Write};

pub const MAGIC: [u8; 4] = *b"BAC1";
pub const SYNC: u8 = 0xBA;
pub const UNIT_HEAD: u8 = 1;
pub const UNIT_META: u8 = 2;
pub const UNIT_EVENTS: u8 = 3;
pub const UNIT_AUDIO: u8 = 4;
pub const UNIT_INDEX: u8 = 5;
pub const UNIT_END: u8 = 0x7F;
/// no payload is longer than this: bounds what a damaged length can make a reader wait for
pub const MAX_UNIT: usize = 4 << 20;
/// the whole END unit
const END_LEN: u64 = 2 + 1 + 16 + 4;
/// AUDIO flags: the packet starts from the reset state; its last frame has fewer samples than a frame
const AUDIO_KEY: u8 = 1;
const AUDIO_SHORT: u8 = 2;
/// the format version this code writes and the highest it reads
pub const VERSION: u8 = 0;

/// One unit as bytes.
pub fn unit_bytes(ty: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 12);
    v.push(SYNC);
    v.push(ty);
    put_varint(&mut v, payload.len() as u64);
    v.extend_from_slice(payload);
    let crc = crc32(&v[1..]);
    v.extend_from_slice(&crc.to_le_bytes());
    v
}

/// The HEAD unit.
#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    pub params: Params,
    /// samples per channel, when the writer knew it at the start
    pub total: Option<u64>,
    /// the encoder's quality setting, for display
    pub quality: Option<f32>,
}

impl Head {
    fn payload(&self) -> Vec<u8> {
        let p = &self.params;
        let mut b = vec![VERSION, 0];
        put_varint(&mut b, p.sample_rate as u64);
        b.extend_from_slice(&[p.channels, p.bits, p.lossless as u8]);
        put_varint(&mut b, p.frame_len as u64);
        b.push(p.order);
        put_varint(&mut b, p.lag_min as u64);
        put_varint(&mut b, p.lag_max as u64);
        put_varint(&mut b, self.total.map_or(0, |t| t + 1));
        b.push(self.quality.map_or(255, |q| (q.clamp(0.0, 10.0) * 10.0).round() as u8));
        b
    }

    fn parse(b: &[u8]) -> Result<Head> {
        let mut c = Cursor::new(b);
        let (version, profile) = (c.u8()?, c.u8()?);
        if version > VERSION || profile != 0 {
            return Err(Error::Unsupported(format!("BAC version {version}, profile {profile}")));
        }
        let num = |c: &mut Cursor| -> Result<u32> { u32::try_from(c.varint()?).map_err(|_| bad("HEAD: a number above 32 bits")) };
        let sample_rate = num(&mut c)?;
        let (channels, bits, flags) = (c.u8()?, c.u8()?, c.u8()?);
        let frame_len = num(&mut c)?;
        let order = c.u8()?;
        let (lag_min, lag_max) = (num(&mut c)?, num(&mut c)?);
        let total = c.varint()?.checked_sub(1);
        let quality = Some(c.u8()?).filter(|&q| q != 255).map(|q| q as f32 / 10.0);
        let params = Params { sample_rate, channels, bits, lossless: flags & 1 != 0, frame_len, order, lag_min, lag_max };
        params.check()?;
        Ok(Head { params, total, quality })
    }
}

/// Where things are in a finished file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Index {
    /// (unit type, byte offset) of the label units
    pub labels: Vec<(u8, u64)>,
    /// (first sample, byte offset) of the key packets
    pub keys: Vec<(u64, u64)>,
    /// samples per channel in the file
    pub total: u64,
}

impl Index {
    fn payload(&self) -> Vec<u8> {
        let mut b = Vec::new();
        put_varint(&mut b, self.labels.len() as u64);
        for &(ty, at) in &self.labels {
            b.push(ty);
            put_varint(&mut b, at);
        }
        put_varint(&mut b, self.keys.len() as u64);
        let mut last = (0u64, 0u64);
        for &(sample, at) in &self.keys {
            put_varint(&mut b, sample - last.0);
            put_varint(&mut b, at - last.1);
            last = (sample, at);
        }
        b
    }

    fn parse(b: &[u8], total: u64) -> Result<Index> {
        let mut c = Cursor::new(b);
        let mut idx = Index { total, ..Default::default() };
        for _ in 0..c.count()? {
            idx.labels.push((c.u8()?, c.varint()?));
        }
        let mut last = (0u64, 0u64);
        for _ in 0..c.count()? {
            last = (last.0.saturating_add(c.varint()?), last.1.saturating_add(c.varint()?));
            idx.keys.push(last);
        }
        Ok(idx)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Writing

/// Writes a stream unit by unit. Nothing already written is ever changed, so `w` can be a pipe or a socket.
pub struct Writer<W: Write> {
    w: W,
    off: u64,
    seq: u8,
    frame_len: u64,
    index: Index,
}

impl<W: Write> Writer<W> {
    pub fn new(mut w: W, head: &Head) -> Result<Self> {
        head.params.check()?;
        w.write_all(&MAGIC)?;
        let mut me = Writer { w, off: MAGIC.len() as u64, seq: 0, frame_len: head.params.frame_len as u64, index: Index::default() };
        me.unit(UNIT_HEAD, &head.payload())?;
        Ok(me)
    }

    fn unit(&mut self, ty: u8, payload: &[u8]) -> Result<u64> {
        if payload.len() > MAX_UNIT {
            return Err(Error::Unsupported(format!("a unit of {} bytes", payload.len())));
        }
        let b = unit_bytes(ty, payload);
        let at = self.off;
        self.w.write_all(&b)?;
        self.off += b.len() as u64;
        Ok(at)
    }

    /// Writes the labels as a META and an EVENTS unit (whichever have content). May be called more than once: a
    /// reader adds the units up.
    pub fn labels(&mut self, l: &Labels) -> Result<()> {
        for (ty, payload) in [(UNIT_META, l.meta_payload()), (UNIT_EVENTS, l.events_payload())] {
            if let Some(p) = payload {
                let at = self.unit(ty, &p)?;
                self.index.labels.push((ty, at));
            }
        }
        Ok(())
    }

    /// Writes a packet of `nframes` frames that follows the last one. `valid_last`: the samples of its last frame
    /// that count, for the end of a stream that is not a whole number of frames.
    pub fn audio(&mut self, key: bool, nframes: usize, valid_last: Option<u32>, data: &[u8]) -> Result<()> {
        let mut pl = Vec::with_capacity(data.len() + 12);
        pl.push(if key { AUDIO_KEY } else { 0 } | if valid_last.is_some() { AUDIO_SHORT } else { 0 });
        pl.push(self.seq);
        put_varint(&mut pl, nframes as u64);
        if key {
            put_varint(&mut pl, self.index.total);
        }
        if let Some(v) = valid_last {
            put_varint(&mut pl, v as u64);
        }
        pl.extend_from_slice(data);
        self.audio_unit(&pl)
    }

    /// Writes an AUDIO payload as it is (taken from another file) and books it.
    pub fn audio_unit(&mut self, payload: &[u8]) -> Result<()> {
        let a = AudioHead::parse(payload, self.frame_len)?;
        let at = self.unit(UNIT_AUDIO, payload)?;
        if let Some(first) = a.first {
            self.index.total = first;
            self.index.keys.push((first, at));
        }
        self.seq = a.seq.wrapping_add(1);
        self.index.total += a.samples;
        Ok(())
    }

    /// Writes the INDEX and END units and hands the sink back.
    pub fn finish(mut self) -> Result<W> {
        let at = self.unit(UNIT_INDEX, &self.index.payload())?;
        let mut end = Vec::with_capacity(16);
        end.extend_from_slice(&at.to_le_bytes());
        end.extend_from_slice(&self.index.total.to_le_bytes());
        self.unit(UNIT_END, &end)?;
        self.w.flush()?;
        Ok(self.w)
    }
}

/// The start of an AUDIO payload, before the coded frames.
struct AudioHead {
    seq: u8,
    nframes: usize,
    /// Some(first sample) on a key packet
    first: Option<u64>,
    /// samples per channel the packet stands for
    samples: u64,
    /// bytes of this head
    len: usize,
}

impl AudioHead {
    fn parse(b: &[u8], frame_len: u64) -> Result<AudioHead> {
        let mut c = Cursor::new(b);
        let (flags, seq) = (c.u8()?, c.u8()?);
        let nframes = c.varint()?;
        if nframes == 0 || nframes > MAX_PACKET_FRAMES as u64 {
            return Err(bad(format!("a packet of {nframes} frames")));
        }
        let first = if flags & AUDIO_KEY != 0 { Some(c.varint()?) } else { None };
        let mut samples = nframes * frame_len;
        if flags & AUDIO_SHORT != 0 {
            let valid = c.varint()?;
            if valid == 0 || valid >= frame_len {
                return Err(bad("a short last frame that is not short"));
            }
            samples -= frame_len - valid;
        }
        Ok(AudioHead { seq, nframes: nframes as usize, first, samples, len: b.len() - c.rest().len() })
    }
}

/// Samples -> a whole file in memory.
pub fn encode(pcm: &Pcm, o: &EncOpts, labels: &Labels) -> Result<Vec<u8>> {
    Ok(encode_stats(pcm, o, labels)?.0)
}

/// `encode`, and where the encoder's bits went.
pub fn encode_stats(pcm: &Pcm, o: &EncOpts, labels: &Labels) -> Result<(Vec<u8>, crate::encoder::EncStats)> {
    if pcm.ch.is_empty() || pcm.ch.len() > 8 || pcm.ch.iter().any(|c| c.len() != pcm.len()) {
        return Err(Error::Unsupported(format!("{} channels (1 to 8, all of one length)", pcm.ch.len())));
    }
    let p = Params::new(pcm.rate, pcm.ch.len() as u8, pcm.bits, o.lossless);
    let head = Head { params: p.clone(), total: Some(pcm.len() as u64), quality: (!o.lossless).then_some(o.quality) };
    let mut w = Writer::new(Vec::new(), &head)?;
    w.labels(labels)?;
    let n = p.frame_len as usize;
    let frames = pcm.len().div_ceil(n);
    // whole frames per packet, whole packets between key packets
    let per = ((o.packet_ms.max(1) as u64 * p.sample_rate as u64 / 1000) as usize / n).clamp(1, MAX_PACKET_FRAMES);
    let key_every = (o.key_ms as usize * p.sample_rate as usize / 1000 / n / per).max(1);
    let mut enc = Encoder::new(p, o);
    for (pi, f0) in (0..frames).step_by(per).enumerate() {
        let nf = per.min(frames - f0);
        let key = pi % key_every == 0;
        let data = enc.encode_packet(&pcm.ch, f0, nf, key);
        let short = (f0 + nf == frames && !pcm.len().is_multiple_of(n)).then_some((pcm.len() % n) as u32);
        w.audio(key, nf, short, &data)?;
    }
    Ok((w.finish()?, enc.stats))
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading

pub struct Unit {
    pub ty: u8,
    pub payload: Vec<u8>,
    /// where its 0xBA is, counted from the start of the stream
    pub offset: u64,
}

/// Finds the units in a byte stream. What is not a unit (the magic, damage, the bytes before a live stream's first
/// whole unit) is passed over and counted.
pub struct UnitReader<R: Read> {
    r: R,
    buf: Vec<u8>,
    start: usize,
    offset: u64,
    eof: bool,
    /// bytes passed over
    pub skipped: u64,
}

impl<R: Read> UnitReader<R> {
    /// `offset`: where in the stream `r` stands.
    pub fn new(r: R, offset: u64) -> Self {
        UnitReader { r, buf: Vec::new(), start: 0, offset, eof: false, skipped: 0 }
    }

    /// Makes `need` bytes available from `start` on; false when the stream ends before that.
    fn fill(&mut self, need: usize) -> Result<bool> {
        while self.buf.len() - self.start < need {
            if self.eof {
                return Ok(false);
            }
            if self.start >= 1 << 16 {
                self.buf.drain(..self.start);
                self.start = 0;
            }
            let old = self.buf.len();
            self.buf.resize(old + (need - (old - self.start)).max(1 << 14), 0);
            let got = loop {
                match self.r.read(&mut self.buf[old..]) {
                    Ok(n) => break n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        self.buf.truncate(old);
                        return Err(e.into());
                    }
                }
            };
            self.buf.truncate(old + got);
            self.eof = got == 0;
        }
        Ok(true)
    }

    /// Passes over the magic if the stream stands at it.
    fn skip_magic(&mut self) -> Result<()> {
        if self.fill(4)? && self.buf[self.start..self.start + 4] == MAGIC {
            self.start += 4;
            self.offset += 4;
        }
        Ok(())
    }

    /// The next unit whose checksum holds, or None at the end of the stream.
    pub fn next_unit(&mut self) -> Result<Option<Unit>> {
        loop {
            if !self.fill(3)? {
                return Ok(None);
            }
            if self.buf[self.start] == SYNC && (1..=0x7F).contains(&self.buf[self.start + 1]) {
                // the length: at most 4 bytes of varint for MAX_UNIT
                let (mut len, mut i, mut whole) = (0usize, 2usize, false);
                while i < 6 && self.fill(i + 1)? {
                    let v = self.buf[self.start + i];
                    len |= ((v & 0x7F) as usize) << (7 * (i - 2));
                    i += 1;
                    if v & 0x80 == 0 {
                        whole = true;
                        break;
                    }
                }
                if whole && len <= MAX_UNIT && self.fill(i + len + 4)? {
                    let u = &self.buf[self.start..self.start + i + len + 4];
                    if crc32(&u[1..i + len]).to_le_bytes() == u[i + len..] {
                        let unit = Unit { ty: u[1], payload: u[i..i + len].to_vec(), offset: self.offset };
                        self.start += i + len + 4;
                        self.offset += (i + len + 4) as u64;
                        return Ok(Some(unit));
                    }
                }
            }
            self.start += 1;
            self.offset += 1;
            self.skipped += 1;
        }
    }
}

/// Decoded samples of consecutive frames.
pub struct Chunk {
    /// the index of the first sample
    pub start: u64,
    /// one buffer per channel
    pub pcm: Vec<Vec<i32>>,
}

/// the longest stretch of missing audio that is written out as silence, in seconds
pub const MAX_GAP_SECS: u64 = 600;

impl Chunk {
    /// Samples of silence to put before this chunk when `have` samples are there already: the audio that was lost
    /// before it. A gap of more than ten minutes is a stream joined late or a wrong time line, and an error here.
    pub fn gap(&self, have: u64, rate: u32) -> Result<usize> {
        let gap = self.start.saturating_sub(have);
        if gap > MAX_GAP_SECS * rate as u64 {
            return Err(bad(format!("{} s of audio are missing before sample {}", gap / rate as u64, self.start)));
        }
        Ok(gap as usize)
    }
}

/// Reads a stream front to back: audio comes out in chunks, labels pile up in `labels` as their units pass.
pub struct Reader<R: Read> {
    units: UnitReader<R>,
    pub head: Head,
    pub labels: Labels,
    dec: Decoder,
    /// false until a key packet has been decoded, and again after a gap
    synced: bool,
    seq: u8,
    pos: u64,
    /// the END unit's number of samples, once it has passed
    pub total: Option<u64>,
}

impl<R: Read> Reader<R> {
    /// Reads up to the HEAD unit. On a live stream that is the next time the sender repeats it.
    pub fn new(r: R) -> Result<Self> {
        let mut units = UnitReader::new(r, 0);
        units.skip_magic()?;
        loop {
            match units.next_unit()? {
                None => return Err(bad("not a BAC stream: no header")),
                Some(u) if u.ty == UNIT_HEAD => return Self::with_head(units, Head::parse(&u.payload)?),
                Some(_) => {}
            }
        }
    }

    /// A reader for a stream whose header is known already, standing at byte `offset` (after a seek).
    pub fn resume(r: R, head: Head, offset: u64) -> Result<Self> {
        Self::with_head(UnitReader::new(r, offset), head)
    }

    fn with_head(units: UnitReader<R>, head: Head) -> Result<Self> {
        let dec = Decoder::new(head.params.clone())?;
        Ok(Reader { units, head, labels: Labels::default(), dec, synced: false, seq: 0, pos: 0, total: None })
    }

    /// Bytes that were not part of any unit so far.
    pub fn skipped(&self) -> u64 {
        self.units.skipped
    }

    /// The next decoded packet, or None at the end of the stream. Packets that cannot be decoded yet (before the
    /// first key packet, after a gap) are passed over.
    pub fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        loop {
            let Some(u) = self.units.next_unit()? else { return Ok(None) };
            match u.ty {
                UNIT_META => self.labels.merge_meta(&u.payload)?,
                UNIT_EVENTS => self.labels.merge_events(&u.payload)?,
                UNIT_AUDIO => {
                    if let Some(c) = self.audio(&u.payload)? {
                        return Ok(Some(c));
                    }
                }
                UNIT_END if u.payload.len() == 16 => self.total = Some(u64::from_le_bytes(u.payload[8..].try_into().unwrap())),
                _ => {}
            }
        }
    }

    fn audio(&mut self, b: &[u8]) -> Result<Option<Chunk>> {
        let a = AudioHead::parse(b, self.head.params.frame_len as u64)?;
        match a.first {
            Some(first) => {
                self.dec.reset();
                self.pos = first;
                self.synced = true;
            }
            None if !self.synced || a.seq != self.seq => {
                self.synced = false;
                return Ok(None);
            }
            None => {}
        }
        self.seq = a.seq.wrapping_add(1);
        let mut pcm = vec![Vec::with_capacity(a.samples as usize); self.head.params.channels as usize];
        self.dec.decode_packet(&b[a.len..], a.nframes, &mut pcm)?;
        pcm.iter_mut().for_each(|c| c.truncate(a.samples as usize));
        let start = self.pos;
        self.pos += a.samples;
        Ok(Some(Chunk { start, pcm }))
    }
}

/// A whole stream in memory -> its header, labels and samples.
pub fn decode(bytes: &[u8]) -> Result<(Head, Labels, Pcm)> {
    let mut r = Reader::new(bytes)?;
    let p = r.head.params.clone();
    let mut ch = vec![Vec::new(); p.channels as usize];
    while let Some(c) = r.next_chunk()? {
        // a stream that lost a part has gaps: they come out as silence
        let gap = c.gap(ch[0].len() as u64, p.sample_rate)?;
        for (o, part) in ch.iter_mut().zip(&c.pcm) {
            o.resize(o.len() + gap, 0);
            o.extend_from_slice(part);
        }
    }
    Ok((r.head, r.labels, Pcm { rate: p.sample_rate, bits: p.bits, ch }))
}

/// What `scan` counted.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// samples per channel
    pub samples: u64,
    pub packets: usize,
    pub keys: usize,
    /// bytes of the AUDIO units and of the label units, their framing included
    pub audio_bytes: u64,
    pub label_bytes: u64,
}

/// Header, labels and counts of a stream, without decoding its audio.
pub fn scan(input: &[u8]) -> Result<(Head, Labels, Stats)> {
    let mut units = UnitReader::new(input, 0);
    units.skip_magic()?;
    let mut head: Option<Head> = None;
    let mut labels = Labels::default();
    let mut st = Stats::default();
    while let Some(u) = units.next_unit()? {
        let size = unit_bytes(u.ty, &[]).len() as u64 + u.payload.len() as u64 + if u.payload.len() >= 128 { 1 } else { 0 } + if u.payload.len() >= 16_384 { 1 } else { 0 };
        match (u.ty, &head) {
            (UNIT_HEAD, None) => head = Some(Head::parse(&u.payload)?),
            (UNIT_META, _) => {
                labels.merge_meta(&u.payload)?;
                st.label_bytes += size;
            }
            (UNIT_EVENTS, _) => {
                labels.merge_events(&u.payload)?;
                st.label_bytes += size;
            }
            (UNIT_AUDIO, Some(h)) => {
                let a = AudioHead::parse(&u.payload, h.params.frame_len as u64)?;
                if let Some(first) = a.first {
                    st.keys += 1;
                    st.samples = first;
                }
                st.samples += a.samples;
                st.packets += 1;
                st.audio_bytes += size;
            }
            _ => {}
        }
    }
    Ok((head.ok_or_else(|| bad("not a BAC stream: no header"))?, labels, st))
}

/// The INDEX of a finished file, found through its last 23 bytes. None for a stream that was never finished.
pub fn read_index<F: Read + Seek>(f: &mut F) -> Result<Option<Index>> {
    let len = f.seek(SeekFrom::End(0))?;
    if len < END_LEN {
        return Ok(None);
    }
    f.seek(SeekFrom::Start(len - END_LEN))?;
    let Some(end) = UnitReader::new(&mut *f, len - END_LEN).next_unit()? else { return Ok(None) };
    if end.ty != UNIT_END || end.payload.len() != 16 || end.offset != len - END_LEN {
        return Ok(None);
    }
    let at = u64::from_le_bytes(end.payload[..8].try_into().unwrap());
    let total = u64::from_le_bytes(end.payload[8..].try_into().unwrap());
    f.seek(SeekFrom::Start(at))?;
    match UnitReader::new(&mut *f, at).next_unit()? {
        Some(u) if u.ty == UNIT_INDEX && u.offset == at => Ok(Some(Index::parse(&u.payload, total)?)),
        _ => Ok(None),
    }
}

/// The samples [from, to) of a file, decoding only from the key packet before `from`. Also gives the header and
/// the labels (all of them, if the file has an index; those before `to` otherwise).
pub fn decode_range<F: Read + Seek>(f: &mut F, from: u64, to: u64) -> Result<(Head, Labels, Pcm)> {
    f.seek(SeekFrom::Start(0))?;
    let head = Reader::new(&mut *f)?.head;
    let index = read_index(f)?;
    let mut labels = Labels::default();
    let mut r = match &index {
        Some(idx) => {
            for &(ty, at) in &idx.labels {
                f.seek(SeekFrom::Start(at))?;
                match UnitReader::new(&mut *f, at).next_unit()? {
                    Some(u) if u.ty == UNIT_META && ty == UNIT_META => labels.merge_meta(&u.payload)?,
                    Some(u) if u.ty == UNIT_EVENTS && ty == UNIT_EVENTS => labels.merge_events(&u.payload)?,
                    _ => return Err(bad("the index points at something that is not a label unit")),
                }
            }
            let at = idx.keys.iter().rev().find(|k| k.0 <= from).or(idx.keys.first()).map_or(0, |k| k.1);
            f.seek(SeekFrom::Start(at))?;
            Reader::resume(&mut *f, head.clone(), at)?
        }
        None => {
            // no index: read from the start
            f.seek(SeekFrom::Start(0))?;
            Reader::new(&mut *f)?
        }
    };
    let p = head.params.clone();
    let mut ch = vec![Vec::new(); p.channels as usize];
    while let Some(c) = r.next_chunk()? {
        let end = c.start + c.pcm[0].len() as u64;
        if c.start >= to {
            break;
        }
        if end > from {
            let (a, b) = (from.saturating_sub(c.start) as usize, (to.min(end) - c.start) as usize);
            for (o, part) in ch.iter_mut().zip(&c.pcm) {
                o.extend_from_slice(&part[a..b]);
            }
        }
    }
    if index.is_none() {
        labels = std::mem::take(&mut r.labels);
    }
    Ok((head, labels, Pcm { rate: p.sample_rate, bits: p.bits, ch }))
}

/// A file with its labels replaced: the audio packets are copied as they are, nothing is coded again.
pub fn rewrite_labels(input: &[u8], labels: &Labels) -> Result<Vec<u8>> {
    let mut units = UnitReader::new(input, 0);
    units.skip_magic()?;
    let mut w: Option<Writer<Vec<u8>>> = None;
    while let Some(u) = units.next_unit()? {
        if u.ty == UNIT_HEAD {
            if w.is_none() {
                let mut nw = Writer::new(Vec::with_capacity(input.len()), &Head::parse(&u.payload)?)?;
                nw.labels(labels)?;
                w = Some(nw);
            }
        } else if let Some(w) = w.as_mut() {
            match u.ty {
                UNIT_AUDIO => w.audio_unit(&u.payload)?,
                UNIT_META | UNIT_EVENTS | UNIT_INDEX | UNIT_END => {}
                // units of a later version are kept where they stand
                ty => {
                    w.unit(ty, &u.payload)?;
                }
            }
        }
    }
    w.ok_or_else(|| bad("not a BAC stream: no header"))?.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labels::{Event, TIER_TAG, TIER_TEXT};
    use std::io::Cursor as IoCursor;

    fn tone(rate: u32, secs: f64) -> Pcm {
        let n = (rate as f64 * secs) as usize;
        let ch = (0..n).map(|i| {
            let t = i as f64 / rate as f64;
            (3000.0 * (t * 220.0 * std::f64::consts::TAU).sin() * (t * 3.0).sin().abs() + 40.0 * ((i * 7919) % 13) as f64) as i32
        });
        Pcm { rate, bits: 16, ch: vec![ch.collect()] }
    }

    fn labels() -> Labels {
        let mut l = Labels::default();
        l.set_attr("language", "en");
        let id = l.speaker_id("jax");
        l.add(Event { tier: TIER_TEXT.into(), start: 0, end: 30_000, speaker: Some(id), weight: None, text: "One two three.".into(), attrs: vec![] });
        l.add(Event { tier: TIER_TAG.into(), start: 8_000, end: 20_000, speaker: Some(id), weight: None, text: "singing".into(), attrs: vec![] });
        l
    }

    fn lossless() -> EncOpts {
        EncOpts { lossless: true, ..Default::default() }
    }

    #[test]
    fn a_file_comes_back_with_its_labels() {
        // 3.21 s: not a whole number of frames
        let pcm = tone(16_000, 3.21);
        let file = encode(&pcm, &lossless(), &labels()).unwrap();
        assert_eq!(&file[..4], b"BAC1");
        let (head, l, back) = decode(&file).unwrap();
        assert_eq!(back, pcm);
        assert_eq!(l, labels());
        assert_eq!(head.total, Some(pcm.len() as u64));
        assert!(head.params.lossless);
    }

    /// hands out a few bytes per read, like a slow network
    struct Trickle<'a>(&'a [u8], usize);
    impl Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.1).min(self.0.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    #[test]
    fn a_stream_plays_as_it_arrives() {
        let pcm = tone(16_000, 2.0);
        let file = encode(&pcm, &lossless(), &labels()).unwrap();
        let mut r = Reader::new(Trickle(&file, 7)).unwrap();
        let mut got = Vec::new();
        let mut chunks = 0;
        while let Some(c) = r.next_chunk().unwrap() {
            assert_eq!(c.start, got.len() as u64);
            // the labels were there before the first sound
            assert_eq!(r.labels, labels());
            got.extend_from_slice(&c.pcm[0]);
            chunks += 1;
        }
        assert_eq!(got, pcm.ch[0]);
        assert_eq!(chunks, 4);
        assert_eq!(r.total, Some(pcm.len() as u64));
        assert_eq!(r.skipped(), 0);
    }

    #[test]
    fn joining_late_starts_at_a_key_packet() {
        let pcm = tone(16_000, 4.0);
        let file = encode(&pcm, &lossless(), &Labels::default()).unwrap();
        let head = Reader::new(&file[..]).unwrap().head;
        // cut in somewhere in the second second: the third second's key packet is the first that decodes
        let mut r = Reader::resume(&file[file.len() * 3 / 8..], head, 0).unwrap();
        let c = r.next_chunk().unwrap().unwrap();
        assert_eq!(c.start, 32_000);
        assert_eq!(c.pcm[0], pcm.ch[0][32_000..40_000]);
        assert!(r.skipped() > 0);
    }

    #[test]
    fn damage_costs_only_up_to_the_next_key_packet() {
        let pcm = tone(16_000, 4.0);
        let mut file = encode(&pcm, &lossless(), &Labels::default()).unwrap();
        let at = file.len() * 3 / 8;
        file[at] ^= 0x40;
        let (_, _, back) = decode(&file).unwrap();
        assert_eq!(back.len(), pcm.len());
        let same = (0..pcm.len()).filter(|&i| back.ch[0][i] == pcm.ch[0][i]).count();
        // at most the rest of one second is lost
        assert!(same >= pcm.len() - 16_000 && same < pcm.len(), "{same} of {}", pcm.len());
        assert_eq!(back.ch[0][32_000..], pcm.ch[0][32_000..]);
    }

    #[test]
    fn a_range_is_read_through_the_index() {
        let pcm = tone(16_000, 5.0);
        let file = encode(&pcm, &lossless(), &labels()).unwrap();
        let mut f = IoCursor::new(&file);
        let idx = read_index(&mut f).unwrap().unwrap();
        assert_eq!(idx.total, pcm.len() as u64);
        assert_eq!(idx.keys.len(), 5);
        assert_eq!(idx.labels.len(), 2);
        let (_, l, part) = decode_range(&mut f, 40_123, 51_000).unwrap();
        assert_eq!(part.ch[0], pcm.ch[0][40_123..51_000]);
        assert_eq!(l, labels());
        // a stream without its end still gives the range, read from the start
        let cut = &file[..file.len() - 30];
        let (_, l, part) = decode_range(&mut IoCursor::new(cut), 40_123, 51_000).unwrap();
        assert_eq!(part.ch[0], pcm.ch[0][40_123..51_000]);
        assert_eq!(l, labels());
    }

    #[test]
    fn labels_are_replaced_without_coding_again() {
        let pcm = tone(16_000, 2.5);
        let file = encode(&pcm, &EncOpts::default(), &Labels::default()).unwrap();
        let (_, _, before) = decode(&file).unwrap();
        let tagged = rewrite_labels(&file, &labels()).unwrap();
        let (_, l, after) = decode(&tagged).unwrap();
        assert_eq!(after, before);
        assert_eq!(l, labels());
        assert_eq!(read_index(&mut IoCursor::new(&tagged)).unwrap().unwrap().keys.len(), 3);
        // and taken away again: the first file, byte for byte
        assert_eq!(rewrite_labels(&tagged, &Labels::default()).unwrap(), file);
    }

    #[test]
    fn damaged_files_give_errors_or_sound_but_no_panic() {
        let pcm = tone(16_000, 1.5);
        let file = encode(&pcm, &EncOpts::default(), &labels()).unwrap();
        let mut rng = 99u32;
        let mut next = move || {
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (rng >> 8) as usize
        };
        for round in 0..400 {
            let mut bad = file.clone();
            for _ in 0..1 + next() % 4 {
                let at = next() % bad.len();
                bad[at] = next() as u8;
            }
            if round % 3 == 0 {
                bad.truncate(next() % bad.len());
            }
            let _ = decode(&bad);
            let _ = scan(&bad);
            let _ = rewrite_labels(&bad, &Labels::default());
            let _ = decode_range(&mut IoCursor::new(&bad), 3_000, 9_000);
        }
        // a unit that is whole but says nonsense: checksums do not stop a careless writer
        for payload in [vec![], vec![1, 0, 1], vec![1, 0, 200, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F], vec![3, 0, 0]] {
            let mut bad = file[..40].to_vec();
            let head_end = 4 + unit_bytes(UNIT_HEAD, &Reader::new(&file[..]).unwrap().head.payload()).len();
            bad.truncate(head_end);
            bad.extend_from_slice(&unit_bytes(UNIT_AUDIO, &payload));
            assert!(decode(&bad).is_err(), "{payload:?}");
        }
    }

    #[test]
    fn garbage_is_not_a_stream() {
        assert!(Reader::new(&b"RIFF and so on"[..]).is_err());
        assert!(decode(&[]).is_err());
        let mut rng = 12345u32;
        let junk: Vec<u8> = (0..5000)
            .map(|_| {
                rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                if rng >> 28 == 0 {
                    SYNC
                } else {
                    (rng >> 16) as u8
                }
            })
            .collect();
        assert!(decode(&junk).is_err());
    }
}
