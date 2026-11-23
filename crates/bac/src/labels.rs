//! What a file says about its audio: attributes of the recording, the speakers (voice profiles), and events on the
//! time line. An event is a range of samples on a named tier with a text: the tier `text` holds the transcript in
//! utterances, `word` and `phone` hold alignments, `tag` holds free labels such as "whisper", "laughing" or
//! "singing". Any other tier name is as good (`emotion`, `style`, `lyric`, `note`): readers skip what they do not
//! know.
//!
//! Stored as two unit payloads, META and EVENTS (docs/Format.md), and exchanged with people as JSON.

use crate::error::{bad, Result};
use crate::json::Json;
use crate::wire::{put_bytes, put_str, put_varint, Cursor};

pub const TIER_TEXT: &str = "text";
pub const TIER_WORD: &str = "word";
pub const TIER_PHONE: &str = "phone";
pub const TIER_TAG: &str = "tag";

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    /// anything else, for instance a learned voice vector
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub key: String,
    pub value: Value,
}

/// A voice profile: who speaks, and what is known about the voice.
#[derive(Clone, Debug, PartialEq)]
pub struct Speaker {
    /// what events refer to; 1 and up
    pub id: u32,
    pub name: String,
    pub attrs: Vec<Attr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub tier: String,
    /// the samples [start, end)
    pub start: u64,
    pub end: u64,
    pub speaker: Option<u32>,
    /// how strong or how sure, 0 to 255
    pub weight: Option<u8>,
    pub text: String,
    pub attrs: Vec<Attr>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Labels {
    pub attrs: Vec<Attr>,
    pub speakers: Vec<Speaker>,
    /// ordered by start
    pub events: Vec<Event>,
}

const EV_SPEAKER: u8 = 1;
const EV_WEIGHT: u8 = 2;
const EV_ATTRS: u8 = 4;

fn put_attrs(out: &mut Vec<u8>, attrs: &[Attr]) {
    put_varint(out, attrs.len() as u64);
    for a in attrs {
        put_str(out, &a.key);
        match &a.value {
            Value::Text(t) => {
                out.push(0);
                put_str(out, t);
            }
            Value::Bytes(b) => {
                out.push(1);
                put_bytes(out, b);
            }
        }
    }
}

fn get_attrs(c: &mut Cursor) -> Result<Vec<Attr>> {
    let n = c.count()?;
    let mut attrs = Vec::with_capacity(n);
    for _ in 0..n {
        let key = c.str()?;
        let value = match c.u8()? {
            0 => Value::Text(c.str()?),
            // kinds this version does not know are kept as bytes
            _ => Value::Bytes(c.bytes()?.to_vec()),
        };
        attrs.push(Attr { key, value });
    }
    Ok(attrs)
}

fn set(attrs: &mut Vec<Attr>, a: Attr) {
    match attrs.iter_mut().find(|x| x.key == a.key) {
        Some(x) => x.value = a.value,
        None => attrs.push(a),
    }
}

impl Labels {
    pub fn is_empty(&self) -> bool {
        self.attrs.is_empty() && self.speakers.is_empty() && self.events.is_empty()
    }

    /// The META unit: attributes and speakers. None when there are none.
    pub fn meta_payload(&self) -> Option<Vec<u8>> {
        if self.attrs.is_empty() && self.speakers.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        put_attrs(&mut out, &self.attrs);
        put_varint(&mut out, self.speakers.len() as u64);
        for s in &self.speakers {
            put_varint(&mut out, s.id as u64);
            put_str(&mut out, &s.name);
            put_attrs(&mut out, &s.attrs);
        }
        Some(out)
    }

    /// The EVENTS unit: the tier names once, then the events by start time. None when there are none.
    pub fn events_payload(&self) -> Option<Vec<u8>> {
        if self.events.is_empty() {
            return None;
        }
        let mut ev: Vec<&Event> = self.events.iter().collect();
        ev.sort_by_key(|e| e.start);
        let mut tiers: Vec<&str> = Vec::new();
        for e in &ev {
            if !tiers.contains(&e.tier.as_str()) {
                tiers.push(&e.tier);
            }
        }
        let mut out = Vec::new();
        put_varint(&mut out, tiers.len() as u64);
        for t in &tiers {
            put_str(&mut out, t);
        }
        put_varint(&mut out, ev.len() as u64);
        let mut last = 0u64;
        for e in ev {
            put_varint(&mut out, tiers.iter().position(|t| *t == e.tier).unwrap() as u64);
            put_varint(&mut out, e.start - last);
            put_varint(&mut out, e.end.saturating_sub(e.start));
            last = e.start;
            out.push(e.speaker.map_or(0, |_| EV_SPEAKER) | e.weight.map_or(0, |_| EV_WEIGHT) | if e.attrs.is_empty() { 0 } else { EV_ATTRS });
            if let Some(s) = e.speaker {
                put_varint(&mut out, s as u64);
            }
            if let Some(w) = e.weight {
                out.push(w);
            }
            put_str(&mut out, &e.text);
            if !e.attrs.is_empty() {
                put_attrs(&mut out, &e.attrs);
            }
        }
        Some(out)
    }

    /// Takes a META unit in: a later attribute or speaker replaces an earlier one of the same key or id.
    pub fn merge_meta(&mut self, b: &[u8]) -> Result<()> {
        let mut c = Cursor::new(b);
        for a in get_attrs(&mut c)? {
            set(&mut self.attrs, a);
        }
        for _ in 0..c.count()? {
            let id = u32::try_from(c.varint()?).map_err(|_| bad("a speaker id above 32 bits"))?;
            let s = Speaker { id, name: c.str()?, attrs: get_attrs(&mut c)? };
            match self.speakers.iter_mut().find(|x| x.id == id) {
                Some(x) => *x = s,
                None => self.speakers.push(s),
            }
        }
        Ok(())
    }

    /// Takes an EVENTS unit in: its events join the ones already there.
    pub fn merge_events(&mut self, b: &[u8]) -> Result<()> {
        let mut c = Cursor::new(b);
        let tiers = (0..c.count()?).map(|_| c.str()).collect::<Result<Vec<_>>>()?;
        let mut last = 0u64;
        for _ in 0..c.count()? {
            let tier = tiers.get(c.varint()? as usize).ok_or_else(|| bad("an event on a tier that is not listed"))?.clone();
            let start = last.checked_add(c.varint()?).ok_or_else(|| bad("an event past the end of time"))?;
            let end = start.saturating_add(c.varint()?);
            last = start;
            let flags = c.u8()?;
            let speaker = if flags & EV_SPEAKER != 0 { Some(u32::try_from(c.varint()?).map_err(|_| bad("a speaker id above 32 bits"))?) } else { None };
            let weight = if flags & EV_WEIGHT != 0 { Some(c.u8()?) } else { None };
            let text = c.str()?;
            let attrs = if flags & EV_ATTRS != 0 { get_attrs(&mut c)? } else { Vec::new() };
            self.events.push(Event { tier, start, end, speaker, weight, text, attrs });
        }
        self.events.sort_by_key(|e| e.start);
        Ok(())
    }

    pub fn set_attr(&mut self, key: &str, text: &str) {
        set(&mut self.attrs, Attr { key: key.into(), value: Value::Text(text.into()) });
    }

    /// The id of the speaker of that name; a new speaker if there is none.
    pub fn speaker_id(&mut self, name: &str) -> u32 {
        if let Some(s) = self.speakers.iter().find(|s| s.name == name) {
            return s.id;
        }
        let id = self.speakers.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        self.speakers.push(Speaker { id, name: name.into(), attrs: Vec::new() });
        id
    }

    pub fn speaker(&self, id: u32) -> Option<&Speaker> {
        self.speakers.iter().find(|s| s.id == id)
    }

    /// Adds an event, keeping the order by start.
    pub fn add(&mut self, e: Event) {
        let at = self.events.partition_point(|x| x.start <= e.start);
        self.events.insert(at, e);
    }

    pub fn tier<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Event> + 'a {
        self.events.iter().filter(move |e| e.tier == name)
    }

    /// The events of a tier that overlap the samples [from, to).
    pub fn at<'a>(&'a self, name: &'a str, from: u64, to: u64) -> impl Iterator<Item = &'a Event> + 'a {
        self.tier(name).filter(move |e| e.start < to && e.end > from)
    }

    // -----------------------------------------------------------------------------------------------------------------
    // JSON: times in seconds, speakers by name

    pub fn to_json(&self, rate: u32) -> String {
        let secs = |s: u64| Json::Num((s as f64 / rate as f64 * 1e6).round() / 1e6);
        let attrs = |a: &[Attr]| {
            Json::Obj(
                a.iter()
                    .map(|a| {
                        (a.key.clone(), match &a.value {
                            Value::Text(t) => Json::Str(t.clone()),
                            Value::Bytes(b) => Json::Obj(vec![("hex".into(), Json::Str(b.iter().map(|v| format!("{v:02x}")).collect()))]),
                        })
                    })
                    .collect(),
            )
        };
        let mut top = Vec::new();
        if !self.attrs.is_empty() {
            top.push(("attrs".to_string(), attrs(&self.attrs)));
        }
        if !self.speakers.is_empty() {
            let list = self.speakers.iter().map(|s| {
                let mut o = vec![("id".to_string(), Json::Num(s.id as f64)), ("name".to_string(), Json::Str(s.name.clone()))];
                if !s.attrs.is_empty() {
                    o.push(("attrs".into(), attrs(&s.attrs)));
                }
                Json::Obj(o)
            });
            top.push(("speakers".to_string(), Json::Arr(list.collect())));
        }
        let events = self.events.iter().map(|e| {
            let mut o = vec![("tier".to_string(), Json::Str(e.tier.clone())), ("start".to_string(), secs(e.start)), ("end".to_string(), secs(e.end))];
            if let Some(id) = e.speaker {
                o.push(("speaker".into(), match self.speaker(id) {
                    Some(s) if !s.name.is_empty() => Json::Str(s.name.clone()),
                    _ => Json::Num(id as f64),
                }));
            }
            if let Some(w) = e.weight {
                o.push(("weight".into(), Json::Num((w as f64 / 255.0 * 1000.0).round() / 1000.0)));
            }
            o.push(("text".into(), Json::Str(e.text.clone())));
            if !e.attrs.is_empty() {
                o.push(("attrs".into(), attrs(&e.attrs)));
            }
            Json::Obj(o)
        });
        top.push(("events".to_string(), Json::Arr(events.collect())));
        Json::Obj(top).pretty()
    }

    pub fn from_json(src: &str, rate: u32) -> Result<Labels> {
        let j = Json::parse(src)?;
        let attrs = |v: Option<&Json>| -> Result<Vec<Attr>> {
            let Some(v) = v else { return Ok(Vec::new()) };
            let o = v.as_obj().ok_or_else(|| bad("labels: \"attrs\" must be an object"))?;
            o.iter()
                .map(|(k, v)| {
                    let value = match v {
                        Json::Str(s) => Value::Text(s.clone()),
                        Json::Num(_) | Json::Bool(_) => Value::Text(v.pretty().trim().to_string()),
                        other => {
                            let hex = other.get("hex").and_then(Json::as_str).ok_or_else(|| bad(format!("labels: attribute \"{k}\" must be text or {{\"hex\": ...}}")))?;
                            let bytes = (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)).collect::<std::result::Result<Vec<u8>, _>>();
                            Value::Bytes(bytes.ok().filter(|_| hex.len() % 2 == 0 && hex.is_ascii()).ok_or_else(|| bad(format!("labels: attribute \"{k}\" is not hex")))?)
                        }
                    };
                    Ok(Attr { key: k.clone(), value })
                })
                .collect()
        };
        let mut l = Labels { attrs: attrs(j.get("attrs"))?, ..Default::default() };
        for s in j.get("speakers").and_then(Json::as_arr).unwrap_or(&[]) {
            let name = s.get("name").and_then(Json::as_str).unwrap_or("").to_string();
            let id = match s.get("id").and_then(Json::as_f64) {
                Some(v) if v >= 1.0 && v <= u32::MAX as f64 => v as u32,
                Some(_) => return Err(bad("labels: a speaker id must be 1 or more")),
                None => l.speakers.iter().map(|s| s.id).max().unwrap_or(0) + 1,
            };
            if l.speaker(id).is_some() {
                return Err(bad(format!("labels: two speakers with id {id}")));
            }
            l.speakers.push(Speaker { id, name, attrs: attrs(s.get("attrs"))? });
        }
        let samples = |e: &Json, key: &str| -> Result<u64> {
            let v = e.get(key).and_then(Json::as_f64).filter(|v| *v >= 0.0 && v.is_finite()).ok_or_else(|| bad(format!("labels: an event needs \"{key}\" in seconds")))?;
            Ok((v * rate as f64).round() as u64)
        };
        for e in j.get("events").and_then(Json::as_arr).unwrap_or(&[]) {
            let (start, end) = (samples(e, "start")?, samples(e, "end")?);
            if end < start {
                return Err(bad("labels: an event that ends before it starts"));
            }
            let speaker = match e.get("speaker") {
                None | Some(Json::Null) => None,
                Some(Json::Str(name)) => Some(l.speaker_id(name)),
                Some(Json::Num(v)) if *v >= 1.0 => Some(*v as u32),
                Some(_) => return Err(bad("labels: \"speaker\" must be a name or an id")),
            };
            let weight = e.get("weight").and_then(Json::as_f64).map(|w| (w.clamp(0.0, 1.0) * 255.0).round() as u8);
            l.events.push(Event {
                tier: e.get("tier").and_then(Json::as_str).unwrap_or(TIER_TEXT).to_string(),
                start,
                end,
                speaker,
                weight,
                text: e.get("text").and_then(Json::as_str).unwrap_or("").to_string(),
                attrs: attrs(e.get("attrs"))?,
            });
        }
        l.events.sort_by_key(|e| e.start);
        Ok(l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Labels {
        let mut l = Labels::default();
        l.set_attr("title", "Kitchen talk");
        l.set_attr("language", "en");
        let jax = l.speaker_id("jax");
        l.speakers[0].attrs.push(Attr { key: "style".into(), value: Value::Text("dry, calm".into()) });
        l.speakers[0].attrs.push(Attr { key: "voice-row".into(), value: Value::Bytes(vec![0, 1, 254, 255]) });
        let ann = l.speaker_id("ann");
        l.add(Event { tier: TIER_TEXT.into(), start: 0, end: 36_960, speaker: Some(jax), weight: None, text: "Well, that went well.".into(), attrs: vec![] });
        l.add(Event { tier: TIER_TAG.into(), start: 20_000, end: 36_960, speaker: Some(jax), weight: Some(200), text: "laughing".into(), attrs: vec![] });
        l.add(Event { tier: TIER_TEXT.into(), start: 40_000, end: 60_001, speaker: Some(ann), weight: None, text: "Søk — \"quiet\"".into(), attrs: vec![Attr { key: "lang".into(), value: Value::Text("nb".into()) }] });
        l.add(Event { tier: TIER_WORD.into(), start: 0, end: 5_000, speaker: None, weight: None, text: "Well".into(), attrs: vec![] });
        l
    }

    #[test]
    fn units_carry_everything() {
        let l = sample();
        let mut back = Labels::default();
        back.merge_meta(&l.meta_payload().unwrap()).unwrap();
        back.merge_events(&l.events_payload().unwrap()).unwrap();
        assert_eq!(back, l);
        assert!(Labels::default().meta_payload().is_none() && Labels::default().events_payload().is_none());
    }

    #[test]
    fn json_carries_everything() {
        let l = sample();
        let text = l.to_json(16_000);
        assert_eq!(Labels::from_json(&text, 16_000).unwrap(), l);
        assert!(text.contains("\"speaker\": \"jax\""));
    }

    #[test]
    fn later_units_add_and_replace() {
        let mut l = sample();
        let mut more = Labels::default();
        more.set_attr("language", "nb");
        more.speakers.push(Speaker { id: 1, name: "jax2".into(), attrs: vec![] });
        more.add(Event { tier: TIER_TAG.into(), start: 100, end: 200, speaker: None, weight: None, text: "whisper".into(), attrs: vec![] });
        l.merge_meta(&more.meta_payload().unwrap()).unwrap();
        l.merge_events(&more.events_payload().unwrap()).unwrap();
        assert_eq!(l.attrs.len(), 2);
        assert_eq!(l.speaker(1).unwrap().name, "jax2");
        assert_eq!(l.at(TIER_TAG, 0, 150).count(), 1);
        assert!(l.events.windows(2).all(|w| w[0].start <= w[1].start));
    }

    #[test]
    fn damaged_units_are_errors() {
        let l = sample();
        for payload in [l.meta_payload().unwrap(), l.events_payload().unwrap()] {
            for cut in 0..payload.len() {
                let mut x = Labels::default();
                let _ = x.merge_meta(&payload[..cut]);
                let _ = x.merge_events(&payload[..cut]);
            }
        }
    }
}
