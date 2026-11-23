//! `bac`: the command line of the Bantam Audio Codec.

use bac::container::{Reader, Stats, MAGIC};
use bac::labels::{Attr, Event, Value, TIER_TAG, TIER_TEXT};
use bac::metrics;
use bac::pcm::{read_wav, sample_bytes, wav_header, write_wav};
use bac::{EncOpts, Labels, Pcm};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const USAGE: &str = "\
bac - Bantam Audio Codec: voice audio with its transcript, speakers and range tags in one file

  bac [FILE]                                                    the previewer: plays the file, shows its transcript
                                                                word by word, its speakers and tags
  bac encode IN [-o OUT.bac] [-q 0..10 | --lossless] [labels]   WAV or FLAC in (also BAC: codes it again)
  bac join SPEC.json -o OUT.bac [-q 0..10 | --lossless]         many recordings one after the other in one file,
                                                                each a row of its transcript
  bac decode IN [-o OUT.wav] [--from SEC] [--to SEC]            to WAV; '-' is the standard input or output
  bac info   IN                                                 what the file is and what it says about its audio
  bac label  IN [-o OUT.bac] [--export FILE] [labels]           change labels; the audio is copied, not coded again
  bac compare REFERENCE TEST                                    how far TEST is from REFERENCE (WAV, FLAC or BAC)
  bac bench  PATH... [-q LIST] [--limit N]                      sizes, quality numbers and speed over files or folders

encode
  -q N            quality 0 (smallest) to 10 (finest), default 4
  --lossless      the output decodes to the input, bit for bit
  --keep-noise    lossy: code the background between words as it is, not as shaped noise (several kbit/s more)
  --mono          average the channels into one
  --packet-ms N   audio per packet, default 500 (a live sender wants 20 to 100)
  --key-ms N      a packet that decodes on its own at least this often (seek, join), default 1000

labels (encode and label)
  --import FILE           take the labels from a JSON file (see bac label --export)
  --clear                 drop the labels the file has        --clear-tier NAME   drop one tier
  --set KEY=VALUE         an attribute of the recording, e.g. --set language=en
  --speaker NAME[:K=V,..] a voice profile, e.g. --speaker \"jax:gender=male,style=dry and calm\"
  --by NAME               the speaker of the events added by this command
  --text RANGE=TEXT       transcript of a range, e.g. --text \"all=Well, that went well.\"
  --tag RANGE=LABEL       a range tag, e.g. --tag 1.2-2.75=laughing  --tag 3-end=whisper
  --event TIER:RANGE=TEXT an event on any tier, e.g. --event word:0.31-0.58=well
  RANGE is START-END in seconds (END may be 'end'), or 'all'.

join
  SPEC.json is {\"attrs\": {..}, \"speakers\": [{\"name\": \"ann\", \"attrs\": {..}}], \"pieces\": [{\"file\": \"a.flac\",
  \"text\": \"what is said\", \"speaker\": \"ann\", \"id\": \"a-001\", \"gap\": 0.5, \"tier\": \"text\", \"attrs\": {..}}, ..]}.
  Only \"pieces\" and each piece's \"file\" are needed. The files (WAV, FLAC, BAC; of one sample rate; relative to the
  folder of SPEC.json) follow one another, each after \"gap\" seconds of silence, and each becomes an event over its
  samples with its text, speaker and attributes (\"id\" is kept as the attribute id). Takes the options of encode.

Play a file as it streams:   bac decode talk.bac -o - | ffplay -autoexit -nodisp -

previewer (bac FILE, or bac play FILE)
  Space play or pause    Left, Right 5 seconds    Up, Down the row before, after    Home, End
  Ctrl+O open    Ctrl+S save as WAV    Ctrl+F search    Ctrl+I the file's details    Esc clears search and filter
  A click on a word, a row or the waveform plays from there. Rest the mouse on a voice's name for what the file
  says about it. Files can be dropped on the window.
  bac play FILE --snapshot OUT.png [--at SEC] [--size WxH] [--scale F] [--spectrogram] [--details]
                         [--search TEXT] [--mouse X,Y]    draws the window into a picture without opening it
";

type R<T> = Result<T, Box<dyn std::error::Error>>;

fn main() {
    if let Err(e) = run() {
        // whoever read our output has heard enough (a player that was closed): not an error
        let closed = |e: &std::io::Error| e.kind() == std::io::ErrorKind::BrokenPipe || e.raw_os_error() == Some(232);
        let pipe = e.downcast_ref::<std::io::Error>().is_some_and(closed) || matches!(e.downcast_ref::<bac::Error>(), Some(bac::Error::Io(e)) if closed(e));
        if !pipe {
            eprintln!("bac: {e}");
            std::process::exit(1);
        }
    }
}

fn run() -> R<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // no command: the previewer, on the file if one is named (a double click, a file dropped on the program)
    let known = ["encode", "join", "decode", "info", "label", "compare", "bench", "play", "help", "-h", "--help"];
    let cmd = match args.first() {
        Some(c) if known.contains(&c.as_str()) => c.clone(),
        Some(c) if !Path::new(c).is_file() => return Err(format!("'{c}' is not a command or a file (bac help lists the commands)").into()),
        _ => return play(&mut Args(args.iter().cloned().map(Some).collect())),
    };
    let mut a = Args(args[1..].iter().cloned().map(Some).collect());
    match cmd.as_str() {
        "play" => play(&mut a),
        "join" => join(&mut a),
        "encode" => encode(&mut a),
        "decode" => decode(&mut a),
        "info" => info(&mut a),
        "label" => label(&mut a),
        "compare" => compare(&mut a),
        "bench" => bench(&mut a),
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command '{other}' (bac help lists them)").into()),
    }
}

/// The previewer window, or a picture of it.
fn play(a: &mut Args) -> R<()> {
    let snapshot = a.opt("--snapshot")?;
    let at = a.num::<f64>("--at")?.unwrap_or(0.0);
    let scale = a.num::<f32>("--scale")?.unwrap_or(1.0);
    let size = a.opt("--size")?;
    let search = a.opt("--search")?.unwrap_or_default();
    let (spectrogram, details) = (a.flag("--spectrogram"), a.flag("--details"));
    let mouse = match a.opt("--mouse")? {
        Some(m) => {
            let (x, y) = m.split_once(',').ok_or("--mouse takes X,Y")?;
            Some((x.parse().map_err(|_| "--mouse takes X,Y")?, y.parse().map_err(|_| "--mouse takes X,Y")?))
        }
        None => None,
    };
    let o = bac_player::Options { file: None, autoplay: a.flag("--autoplay"), mute: a.flag("--mute"), quit_after: a.num("--quit-after")? };
    let file = match a.rest()?.as_slice() {
        [] => None,
        [one] => Some(one.clone()),
        _ => return Err("the previewer opens one file".into()),
    };
    if let Some(out) = snapshot {
        let mut shot = bac_player::Shot { file, at, scale, spectrogram, search, details, mouse, ..Default::default() };
        if let Some(s) = size {
            let (w, h) = s.split_once('x').ok_or("--size takes WIDTHxHEIGHT")?;
            (shot.width, shot.height) = (w.parse().map_err(|_| "--size takes WIDTHxHEIGHT")?, h.parse().map_err(|_| "--size takes WIDTHxHEIGHT")?);
        }
        return Ok(bac_player::snapshot(&shot, &out)?);
    }
    // started by a double click, the program has a console window of its own that nobody wants to see
    bac_player::leave_own_console();
    Ok(bac_player::run(bac_player::Options { file, ..o })?)
}

/// The arguments not taken yet.
struct Args(Vec<Option<String>>);

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        match self.0.iter().position(|a| a.as_deref() == Some(name)) {
            Some(i) => {
                self.0[i] = None;
                true
            }
            None => false,
        }
    }

    fn opt(&mut self, name: &str) -> R<Option<String>> {
        let Some(i) = self.0.iter().position(|a| a.as_deref() == Some(name)) else { return Ok(None) };
        let v = self.0.get_mut(i + 1).and_then(Option::take).ok_or_else(|| format!("{name} needs a value"))?;
        self.0[i] = None;
        Ok(Some(v))
    }

    fn opts(&mut self, name: &str) -> R<Vec<String>> {
        let mut all = Vec::new();
        while let Some(v) = self.opt(name)? {
            all.push(v);
        }
        Ok(all)
    }

    fn num<T: std::str::FromStr>(&mut self, name: &str) -> R<Option<T>> {
        match self.opt(name)? {
            Some(v) => Ok(Some(v.parse().map_err(|_| format!("{name}: '{v}' is not a number"))?)),
            None => Ok(None),
        }
    }

    /// What is left: the file arguments. An option nobody asked for is an error.
    fn rest(&mut self) -> R<Vec<String>> {
        let rest: Vec<String> = self.0.drain(..).flatten().collect();
        match rest.iter().find(|a| a.starts_with('-') && a.len() > 1) {
            Some(a) => Err(format!("unknown option '{a}'").into()),
            None => Ok(rest),
        }
    }

    fn one(&mut self, what: &str) -> R<String> {
        let rest = self.rest()?;
        match rest.as_slice() {
            [one] => Ok(one.clone()),
            _ => Err(format!("expected {what}").into()),
        }
    }
}

fn read_all(path: &str) -> R<Vec<u8>> {
    if path == "-" {
        let mut b = Vec::new();
        std::io::stdin().lock().read_to_end(&mut b)?;
        Ok(b)
    } else {
        std::fs::read(path).map_err(|e| format!("{path}: {e}").into())
    }
}

/// Audio of any kind this tool reads, with the labels it carries.
fn load(path: &str) -> R<(Pcm, Labels, usize)> {
    let b = read_all(path)?;
    let (pcm, labels) = if b.starts_with(b"RIFF") {
        (read_wav(&b)?, Labels::default())
    } else if b.starts_with(b"fLaC") || b.starts_with(b"ID3") {
        (bac::flac::read_flac(&b)?, Labels::default())
    } else if b.starts_with(&MAGIC) {
        let (_, labels, pcm) = bac::decode(&b)?;
        (pcm, labels)
    } else {
        return Err(format!("{path}: not a WAV, FLAC or BAC file").into());
    };
    Ok((pcm, labels, b.len()))
}

fn enc_opts(a: &mut Args) -> R<EncOpts> {
    let mut o = EncOpts { lossless: a.flag("--lossless"), model_noise: !a.flag("--keep-noise"), ..Default::default() };
    if let Some(q) = a.num::<f32>("-q")? {
        if !(0.0..=10.0).contains(&q) {
            return Err("-q takes 0 to 10".into());
        }
        o.quality = q;
    }
    if let Some(v) = a.num("--packet-ms")? {
        o.packet_ms = v;
    }
    if let Some(v) = a.num("--key-ms")? {
        o.key_ms = v;
    }
    Ok(o)
}

/// "1.2-3.4", "3-end" or "all" -> samples.
fn range(s: &str, rate: u32, total: u64) -> R<(u64, u64)> {
    if s == "all" {
        return Ok((0, total));
    }
    let secs = |t: &str| -> R<u64> {
        if t == "end" {
            return Ok(total);
        }
        let v: f64 = t.parse().map_err(|_| format!("'{t}' is not a time in seconds"))?;
        if v.is_nan() || v < 0.0 {
            return Err(format!("'{t}' is not a time in seconds").into());
        }
        Ok((v * rate as f64).round() as u64)
    };
    let (from, to) = s.split_once('-').ok_or_else(|| format!("'{s}' is not a range: START-END in seconds, or all"))?;
    let (from, to) = (secs(from)?, secs(to)?);
    if to < from {
        return Err(format!("the range '{s}' ends before it starts").into());
    }
    Ok((from, to))
}

/// Applies the label options to `l`. True if any was given.
fn label_opts(a: &mut Args, l: &mut Labels, rate: u32, total: u64) -> R<bool> {
    let mut changed = false;
    if a.flag("--clear") {
        *l = Labels::default();
        changed = true;
    }
    for tier in a.opts("--clear-tier")? {
        l.events.retain(|e| e.tier != tier);
        changed = true;
    }
    if let Some(path) = a.opt("--import")? {
        let text = String::from_utf8(read_all(&path)?).map_err(|_| format!("{path}: not UTF-8"))?;
        let new = Labels::from_json(&text, rate).map_err(|e| format!("{path}: {e}"))?;
        // what the file brings replaces attribute by attribute and speaker by speaker, and adds its events
        if let Some(m) = new.meta_payload() {
            l.merge_meta(&m)?;
        }
        if let Some(e) = new.events_payload() {
            l.merge_events(&e)?;
        }
        changed = true;
    }
    for kv in a.opts("--set")? {
        let (k, v) = kv.split_once('=').ok_or_else(|| format!("--set {kv}: expected KEY=VALUE"))?;
        l.set_attr(k, v);
        changed = true;
    }
    for spec in a.opts("--speaker")? {
        let (name, attrs) = spec.split_once(':').unwrap_or((&spec, ""));
        let id = l.speaker_id(name);
        let s = l.speakers.iter_mut().find(|s| s.id == id).unwrap();
        for kv in attrs.split(',').filter(|kv| !kv.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or_else(|| format!("--speaker {spec}: expected NAME:KEY=VALUE,..."))?;
            let attr = Attr { key: k.trim().into(), value: Value::Text(v.trim().into()) };
            match s.attrs.iter_mut().find(|x| x.key == attr.key) {
                Some(x) => *x = attr,
                None => s.attrs.push(attr),
            }
        }
        changed = true;
    }
    let by = a.opt("--by")?.map(|name| l.speaker_id(&name));
    let mut events: Vec<(String, String)> = Vec::new();
    events.extend(a.opts("--text")?.into_iter().map(|v| (TIER_TEXT.to_string(), v)));
    events.extend(a.opts("--tag")?.into_iter().map(|v| (TIER_TAG.to_string(), v)));
    for v in a.opts("--event")? {
        let (tier, rest) = v.split_once(':').ok_or_else(|| format!("--event {v}: expected TIER:RANGE=TEXT"))?;
        events.push((tier.to_string(), rest.to_string()));
    }
    for (tier, spec) in events {
        let (r, text) = spec.split_once('=').ok_or_else(|| format!("'{spec}': expected RANGE=TEXT"))?;
        let (start, end) = range(r, rate, total)?;
        l.add(Event { tier, start, end, speaker: by, weight: None, text: text.to_string(), attrs: Vec::new() });
        changed = true;
    }
    Ok(changed)
}

fn kbits(bytes: usize, secs: f64) -> f64 {
    bytes as f64 * 8.0 / secs.max(1e-9) / 1000.0
}

/// Many recordings one after the other in one file, each an event over its samples: `bac join`.
fn join(a: &mut Args) -> R<()> {
    use bac::json::Json;
    let o = enc_opts(a)?;
    let mono = a.flag("--mono");
    let out = a.opt("-o")?.ok_or("join: -o OUT.bac is needed")?;
    let spec_path = a.one("one specification file (JSON)")?;
    let spec = Json::parse(&String::from_utf8(read_all(&spec_path)?).map_err(|_| format!("{spec_path}: not UTF-8"))?).map_err(|e| format!("{spec_path}: {e}"))?;
    let base = Path::new(&spec_path).parent().map(Path::to_path_buf).unwrap_or_default();
    // a value of the specification as text: numbers and true/false are written as they stand
    let text_of = |v: &Json| match v {
        Json::Str(s) => s.clone(),
        other => other.pretty().trim().to_string(),
    };
    let attrs_of = |v: Option<&Json>| -> Vec<Attr> { v.and_then(Json::as_obj).unwrap_or(&[]).iter().map(|(k, v)| Attr { key: k.clone(), value: Value::Text(text_of(v)) }).collect() };
    let mut labels = Labels { attrs: attrs_of(spec.get("attrs")), ..Default::default() };
    for s in spec.get("speakers").and_then(Json::as_arr).unwrap_or(&[]) {
        let name = s.get("name").and_then(Json::as_str).ok_or("join: a speaker needs a \"name\"")?;
        let id = labels.speaker_id(name);
        labels.speakers.iter_mut().find(|x| x.id == id).unwrap().attrs = attrs_of(s.get("attrs"));
    }
    let pieces = spec.get("pieces").and_then(Json::as_arr).filter(|p| !p.is_empty()).ok_or("join: the specification needs a list \"pieces\"")?;
    let mut all: Option<Pcm> = None;
    let mut src_bytes = 0;
    for (i, p) in pieces.iter().enumerate() {
        let file = p.get("file").and_then(Json::as_str).ok_or_else(|| format!("join: piece {} has no \"file\"", i + 1))?;
        let path = if Path::new(file).is_absolute() { PathBuf::from(file) } else { base.join(file) };
        let (mut pcm, _, bytes) = load(&path.to_string_lossy())?;
        src_bytes += bytes;
        if mono {
            pcm.ch = vec![pcm.mono()];
        }
        let all = all.get_or_insert_with(|| Pcm { rate: pcm.rate, bits: pcm.bits, ch: vec![Vec::new(); pcm.ch.len()] });
        if (pcm.rate, pcm.bits, pcm.ch.len()) != (all.rate, all.bits, all.ch.len()) {
            return Err(format!("{file}: {} Hz, {} bits, {} channels, but the pieces before it have {} Hz, {} bits, {} channels", pcm.rate, pcm.bits, pcm.ch.len(), all.rate, all.bits, all.ch.len()).into());
        }
        let gap = (p.get("gap").and_then(Json::as_f64).unwrap_or(0.0).max(0.0) * all.rate as f64).round() as usize;
        let start = all.len() + gap;
        for (to, from) in all.ch.iter_mut().zip(&pcm.ch) {
            to.resize(start, 0);
            to.extend_from_slice(from);
        }
        let mut attrs = attrs_of(p.get("attrs"));
        if let Some(id) = p.get("id") {
            attrs.insert(0, Attr { key: "id".into(), value: Value::Text(text_of(id)) });
        }
        let speaker = p.get("speaker").and_then(Json::as_str).map(|n| labels.speaker_id(n));
        labels.add(Event {
            tier: p.get("tier").and_then(Json::as_str).unwrap_or(TIER_TEXT).to_string(),
            start: start as u64,
            end: all.len() as u64,
            speaker,
            weight: None,
            text: p.get("text").map(text_of).unwrap_or_default(),
            attrs,
        });
    }
    let pcm = all.unwrap();
    let t = Instant::now();
    let file = bac::encode(&pcm, &o, &labels)?;
    write_out(&out, &file)?;
    eprintln!(
        "{out}: {} pieces, {:.2} s, {} Hz, {} -> {} bytes, {:.1} kbit/s, {}, {:.0} x real time",
        pieces.len(),
        pcm.seconds(),
        pcm.rate,
        src_bytes,
        file.len(),
        kbits(file.len(), pcm.seconds()),
        if o.lossless { "lossless".to_string() } else { format!("quality {}", o.quality) },
        pcm.seconds() / t.elapsed().as_secs_f64().max(1e-9)
    );
    Ok(())
}

fn encode(a: &mut Args) -> R<()> {
    let o = enc_opts(a)?;
    let mono = a.flag("--mono");
    let out = a.opt("-o")?;
    // the label options need the audio's length, so the input is found by hand: the first argument that is no option
    let input = a.0.iter().flatten().next().filter(|s| !s.starts_with('-') || *s == "-").cloned().ok_or("encode: expected an input file")?;
    let (mut pcm, mut labels, src_bytes) = load(&input)?;
    if mono {
        pcm.ch = vec![pcm.mono()];
    }
    let at = a.0.iter().position(|s| s.as_deref() == Some(input.as_str())).unwrap();
    a.0[at] = None;
    label_opts(a, &mut labels, pcm.rate, pcm.len() as u64)?;
    if let Some(extra) = a.rest()?.first() {
        return Err(format!("encode takes one input file; '{extra}' is one too many").into());
    }
    let out = out.unwrap_or_else(|| if input == "-" { "-".into() } else { Path::new(&input).with_extension("bac").to_string_lossy().into_owned() });
    let t = Instant::now();
    let file = bac::encode(&pcm, &o, &labels)?;
    let took = t.elapsed().as_secs_f64();
    if out == "-" {
        std::io::stdout().lock().write_all(&file)?;
    } else {
        std::fs::write(&out, &file).map_err(|e| format!("{out}: {e}"))?;
    }
    eprintln!(
        "{out}: {:.2} s, {} Hz, {} -> {} bytes ({:.1} % of the source file), {:.1} kbit/s, {}, {:.0} x real time",
        pcm.seconds(),
        pcm.rate,
        src_bytes,
        file.len(),
        100.0 * file.len() as f64 / src_bytes.max(1) as f64,
        kbits(file.len(), pcm.seconds()),
        if o.lossless { "lossless".to_string() } else { format!("quality {}", o.quality) },
        pcm.seconds() / took.max(1e-9)
    );
    Ok(())
}

fn decode(a: &mut Args) -> R<()> {
    let out = a.opt("-o")?;
    let (from, to) = (a.num::<f64>("--from")?, a.num::<f64>("--to")?);
    let input = a.one("one input file")?;
    let out = out.unwrap_or_else(|| if input == "-" { "-".into() } else { Path::new(&input).with_extension("wav").to_string_lossy().into_owned() });
    // a WAV or FLAC file is simply turned into WAV, so that scripts need one tool for every kind of input
    if input != "-" {
        let mut magic = [0u8; 4];
        let n = std::fs::File::open(&input).and_then(|mut f| f.read(&mut magic)).map_err(|e| format!("{input}: {e}"))?;
        if magic[..n] != MAGIC {
            let (mut pcm, _, _) = load(&input)?;
            let at = |t: Option<f64>, d: usize| t.map_or(d, |t| ((t.max(0.0) * pcm.rate as f64).round() as usize).min(pcm.len()));
            let (a, b) = (at(from, 0), at(to, pcm.len()));
            pcm.ch.iter_mut().for_each(|c| *c = c[a.min(b)..b].to_vec());
            return write_out(&out, &write_wav(&pcm));
        }
    }
    if from.is_some() || to.is_some() {
        if input == "-" {
            return Err("--from and --to need a file, not the standard input".into());
        }
        let mut f = std::fs::File::open(&input).map_err(|e| format!("{input}: {e}"))?;
        let rate = Reader::new(&mut f)?.head.params.sample_rate as f64;
        let (a, b) = ((from.unwrap_or(0.0).max(0.0) * rate).round() as u64, to.map_or(u64::MAX, |t| (t.max(0.0) * rate).round() as u64));
        let (_, _, pcm) = bac::decode_range(&mut f, a, b)?;
        return write_out(&out, &write_wav(&pcm));
    }
    // front to back, writing as the packets come: this is the path a player takes
    let src: Box<dyn Read> = if input == "-" { Box::new(std::io::stdin().lock()) } else { Box::new(std::io::BufReader::new(std::fs::File::open(&input).map_err(|e| format!("{input}: {e}"))?)) };
    let mut r = Reader::new(src)?;
    let p = r.head.params.clone();
    if out == "-" {
        let mut w = std::io::stdout().lock();
        w.write_all(&wav_header(p.sample_rate, p.channels as usize, p.bits, r.head.total))?;
        let mut pos = 0u64;
        let mut first = true;
        while let Some(c) = r.next_chunk()? {
            // a listener who joins a live stream starts where the stream is; after that, a part that was lost
            // becomes silence, which keeps the time line
            if first {
                pos = c.start;
                first = false;
            }
            let gap = vec![vec![0i32; c.gap(pos, p.sample_rate)?]; p.channels as usize];
            w.write_all(&sample_bytes(&gap, p.bits))?;
            w.write_all(&sample_bytes(&c.pcm, p.bits))?;
            w.flush()?;
            pos = c.start + c.pcm[0].len() as u64;
        }
        return Ok(());
    }
    let mut pcm = Pcm { rate: p.sample_rate, bits: p.bits, ch: vec![Vec::new(); p.channels as usize] };
    while let Some(c) = r.next_chunk()? {
        let gap = c.gap(pcm.len() as u64, p.sample_rate)?;
        for (o, part) in pcm.ch.iter_mut().zip(&c.pcm) {
            o.resize(o.len() + gap, 0);
            o.extend_from_slice(part);
        }
    }
    if r.skipped() > 0 {
        eprintln!("bac: {input}: {} bytes were not part of any unit (damage?)", r.skipped());
    }
    write_out(&out, &write_wav(&pcm))
}

fn write_out(path: &str, bytes: &[u8]) -> R<()> {
    if path == "-" {
        std::io::stdout().lock().write_all(bytes)?;
        Ok(())
    } else {
        std::fs::write(path, bytes).map_err(|e| format!("{path}: {e}").into())
    }
}

fn clock(samples: u64, rate: u32) -> String {
    format!("{:.2}", samples as f64 / rate as f64)
}

fn print_labels(l: &Labels, rate: u32) {
    if !l.attrs.is_empty() {
        println!("attributes");
        for a in &l.attrs {
            println!("  {} = {}", a.key, show(&a.value));
        }
    }
    if !l.speakers.is_empty() {
        println!("speakers");
        for s in &l.speakers {
            let attrs: Vec<String> = s.attrs.iter().map(|a| format!("{}={}", a.key, show(&a.value))).collect();
            println!("  {} {}{}", s.id, s.name, if attrs.is_empty() { String::new() } else { format!("  ({})", attrs.join(", ")) });
        }
    }
    let mut tiers: Vec<&str> = Vec::new();
    for e in &l.events {
        if !tiers.contains(&e.tier.as_str()) {
            tiers.push(&e.tier);
        }
    }
    for tier in tiers {
        println!("{tier}");
        for e in l.tier(tier) {
            let who = e.speaker.map(|id| l.speaker(id).map_or(format!("#{id}"), |s| s.name.clone())).map_or(String::new(), |n| format!(" [{n}]"));
            let weight = e.weight.map_or(String::new(), |w| format!(" ({:.2})", w as f64 / 255.0));
            println!("  {:>8} - {:>8}{who}{weight}  {}", clock(e.start, rate), clock(e.end, rate), e.text);
        }
    }
}

fn show(v: &Value) -> String {
    match v {
        Value::Text(t) => t.clone(),
        Value::Bytes(b) => format!("<{} bytes>", b.len()),
    }
}

fn info(a: &mut Args) -> R<()> {
    let input = a.one("one input file")?;
    let b = read_all(&input)?;
    let (head, labels, st): (_, _, Stats) = bac::scan(&b)?;
    let p = &head.params;
    let secs = st.samples as f64 / p.sample_rate as f64;
    println!(
        "{input}: BAC, {} Hz, {} channel{}, {} bits, {}",
        p.sample_rate,
        p.channels,
        if p.channels == 1 { "" } else { "s" },
        p.bits,
        if p.lossless { "lossless".to_string() } else { head.quality.map_or("lossy".to_string(), |q| format!("lossy, quality {q}")) }
    );
    println!(
        "{secs:.2} s ({} samples), {} bytes, {:.1} kbit/s; audio {} bytes in {} packets ({} key), labels {} bytes",
        st.samples,
        b.len(),
        kbits(b.len(), secs),
        st.audio_bytes,
        st.packets,
        st.keys,
        st.label_bytes
    );
    print_labels(&labels, p.sample_rate);
    Ok(())
}

fn label(a: &mut Args) -> R<()> {
    let out = a.opt("-o")?;
    let export = a.opt("--export")?;
    let input = a.0.iter().flatten().next().filter(|s| !s.starts_with('-')).cloned().ok_or("label: expected a BAC file")?;
    let at = a.0.iter().position(|s| s.as_deref() == Some(input.as_str())).unwrap();
    a.0[at] = None;
    let b = read_all(&input)?;
    let (head, mut labels, st) = bac::scan(&b)?;
    let rate = head.params.sample_rate;
    let changed = label_opts(a, &mut labels, rate, st.samples)?;
    if let Some(extra) = a.rest()?.first() {
        return Err(format!("label takes one file; '{extra}' is one too many").into());
    }
    if changed || out.is_some() {
        let new = bac::rewrite_labels(&b, &labels)?;
        let out = out.unwrap_or_else(|| input.clone());
        // written beside the target and moved over it, so that a crash leaves the old file
        let tmp = PathBuf::from(format!("{out}.tmp"));
        std::fs::write(&tmp, &new).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &out).map_err(|e| format!("{out}: {e}"))?;
        eprintln!("{out}: {} events, {} speakers, {} attributes; audio unchanged", labels.events.len(), labels.speakers.len(), labels.attrs.len());
    }
    match export.as_deref() {
        Some("-") => print!("{}", labels.to_json(rate)),
        Some(path) => std::fs::write(path, labels.to_json(rate)).map_err(|e| format!("{path}: {e}"))?,
        None if !changed => print_labels(&labels, rate),
        None => {}
    }
    Ok(())
}

fn compare(a: &mut Args) -> R<()> {
    let search = a.num::<usize>("--search")?.unwrap_or(2000);
    let rest = a.rest()?;
    let [reference, test] = rest.as_slice() else { return Err("compare: expected REFERENCE and TEST".into()) };
    let ((x, _, _), (y, _, _)) = (load(reference)?, load(test)?);
    if x.rate != y.rate {
        return Err(format!("{} Hz against {} Hz: resample one of them first", x.rate, y.rate).into());
    }
    // a file of more bits is brought down to the other's scale
    let bits = x.bits.min(y.bits);
    let down = |p: &Pcm| -> Vec<i32> { p.mono().iter().map(|&v| v >> (p.bits - bits)).collect() };
    let r = metrics::compare(&down(&x), &down(&y), x.rate, bits, search);
    println!(
        "SNR {:.2} dB, segmental SNR {:.2} dB, log-spectral distance {:.2} dB, noise-to-mask {:.2} dB, band distance {:.2} dB  ({} frames, test moved by {} samples)",
        r.snr, r.seg_snr, r.lsd, r.nmr, r.band, r.frames, r.offset
    );
    Ok(())
}

/// The audio files under the paths, in a fixed order.
fn collect(paths: &[String]) -> R<Vec<PathBuf>> {
    fn walk(p: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        if p.is_dir() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(p)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
            entries.sort();
            for e in entries {
                walk(&e, out)?;
            }
        } else if matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("wav" | "flac" | "bac")) {
            out.push(p.to_path_buf());
        }
        Ok(())
    }
    let mut out = Vec::new();
    for p in paths {
        let p = Path::new(p);
        if !p.exists() {
            return Err(format!("{}: not found", p.display()).into());
        }
        walk(p, &mut out)?;
    }
    Ok(out)
}

#[derive(Default)]
struct Row {
    st: bac::encoder::EncStats,
    bytes: usize,
    enc: f64,
    dec: f64,
    snr: f64,
    seg: f64,
    lsd: f64,
    nmr: f64,
    band: f64,
}

fn bench(a: &mut Args) -> R<()> {
    let qs: Vec<f32> = match a.opt("-q")? {
        Some(list) => list.split(',').map(|v| v.trim().parse().map_err(|_| format!("-q: '{v}' is not a number"))).collect::<Result<_, _>>()?,
        None => vec![2.0, 3.0, 4.0, 5.0, 6.0, 8.0],
    };
    let limit = a.num::<usize>("--limit")?;
    let (keep_noise, show_bits) = (a.flag("--keep-noise"), a.flag("--bits"));
    let key_ms = a.num::<u32>("--key-ms")?;
    let packet_ms = a.num::<u32>("--packet-ms")?;
    let mut files = collect(&a.rest()?)?;
    if let Some(n) = limit.filter(|&n| n > 0 && n < files.len()) {
        // evenly through the list, so that a corpus sorted by speaker gives many speakers
        files = (0..n).map(|i| files[i * files.len() / n].clone()).collect();
    }
    if files.is_empty() {
        return Err("bench: no WAV, FLAC or BAC files".into());
    }
    let mut rows: Vec<Row> = (0..=qs.len()).map(|_| Row::default()).collect();
    let (mut secs, mut src_bytes, mut pcm_bytes) = (0.0f64, 0usize, 0usize);
    for f in &files {
        let (pcm, _, bytes) = load(&f.to_string_lossy())?;
        if pcm.is_empty() {
            continue;
        }
        let dur = pcm.seconds();
        secs += dur;
        src_bytes += bytes;
        pcm_bytes += pcm.len() * pcm.ch.len() * if pcm.bits <= 16 { 2 } else { 3 };
        let reference = pcm.mono();
        for (i, row) in rows.iter_mut().enumerate() {
            let mut o = if i == 0 { EncOpts { lossless: true, ..Default::default() } } else { EncOpts { quality: qs[i - 1], ..Default::default() } };
            o.key_ms = key_ms.unwrap_or(o.key_ms);
            o.packet_ms = packet_ms.unwrap_or(o.packet_ms);
            o.model_noise = !keep_noise;
            let t = Instant::now();
            let (file, st) = bac::container::encode_stats(&pcm, &o, &Labels::default())?;
            row.enc += t.elapsed().as_secs_f64();
            row.st.frames.iter_mut().zip(st.frames).for_each(|(a, b)| *a += b);
            row.st.background += st.background;
            row.st.repeats += st.repeats;
            row.st.bits_flags += st.bits_flags;
            row.st.bits_lpc += st.bits_lpc;
            row.st.bits_step += st.bits_step;
            row.st.bits_pitch += st.bits_pitch;
            row.st.bytes += st.bytes;
            let t = Instant::now();
            let (_, _, back) = bac::decode(&file)?;
            row.dec += t.elapsed().as_secs_f64();
            row.bytes += file.len();
            if i == 0 {
                if back != pcm {
                    return Err(format!("{}: lossless coding did not give the input back", f.display()).into());
                }
            } else {
                let r = metrics::compare(&reference, &back.mono(), pcm.rate, pcm.bits, 0);
                row.snr += r.snr * dur;
                row.seg += r.seg_snr * dur;
                row.lsd += r.lsd * dur;
                row.nmr += r.nmr * dur;
                row.band += r.band * dur;
            }
        }
    }
    let mb = |b: usize| b as f64 / 1e6;
    println!("{} files, {:.1} s of audio; source files {:.2} MB ({:.1} kbit/s), as plain samples {:.2} MB ({:.1} kbit/s)", files.len(), secs, mb(src_bytes), kbits(src_bytes, secs), mb(pcm_bytes), kbits(pcm_bytes, secs));
    println!("{:<9} {:>9} {:>8} {:>8} {:>7} {:>7} {:>6} {:>7} {:>6} {:>6} {:>6}", "setting", "MB", "kbit/s", "of src", "SNR", "segSNR", "LSD", "NMR", "band", "enc x", "dec x");
    for (i, r) in rows.iter().enumerate() {
        let name = if i == 0 { "lossless".to_string() } else { format!("q {}", qs[i - 1]) };
        let quality = if i == 0 { format!("{:>7} {:>7} {:>6} {:>7} {:>6}", "exact", "", "", "", "") } else { format!("{:>7.1} {:>7.1} {:>6.2} {:>7.1} {:>6.2}", r.snr / secs, r.seg / secs, r.lsd / secs, r.nmr / secs, r.band / secs) };
        println!("{:<9} {:>9.3} {:>8.1} {:>7.1}% {} {:>6.0} {:>6.0}", name, mb(r.bytes), kbits(r.bytes, secs), 100.0 * r.bytes as f64 / src_bytes as f64, quality, secs / r.enc, secs / r.dec);
    }
    if show_bits {
        println!();
        println!("where the bits go, kbit/s (frames: silent / unvoiced / voiced / copy of the other channel; of all frames background, predictor kept)");
        println!("{:<9} {:>8} {:>7} {:>7} {:>7} {:>8} {:>10}   frames", "setting", "samples", "lpc", "steps", "pitch", "flags", "container");
        for (i, r) in rows.iter().enumerate() {
            let name = if i == 0 { "lossless".to_string() } else { format!("q {}", qs[i - 1]) };
            let k = |bits: f64| bits / secs / 1000.0;
            let side = r.st.bits_lpc + r.st.bits_step + r.st.bits_pitch + r.st.bits_flags;
            let all: u64 = r.st.frames.iter().sum();
            let pct = |v: u64| 100.0 * v as f64 / all.max(1) as f64;
            println!(
                "{:<9} {:>8.2} {:>7.2} {:>7.2} {:>7.2} {:>8.2} {:>10.2}   {:.0} / {:.0} / {:.0} / {:.0} %; {:.0} %, {:.0} %",
                name,
                k(r.st.bytes as f64 * 8.0 - side),
                k(r.st.bits_lpc),
                k(r.st.bits_step),
                k(r.st.bits_pitch),
                k(r.st.bits_flags),
                k((r.bytes as f64 - r.st.bytes as f64) * 8.0),
                pct(r.st.frames[0]),
                pct(r.st.frames[1]),
                pct(r.st.frames[2]),
                pct(r.st.frames[3]),
                pct(r.st.background),
                pct(r.st.repeats)
            );
        }
    }
    Ok(())
}
