# The BAC format, version 0

Everything a second implementation needs. The Rust crate `bac` is the reference: `container.rs`, `labels.rs`,
`codec.rs`, `range.rs`, `lpc.rs`. Version 0 is a working draft: files written today may not read after the format
changes, until it is frozen as version 1.

A file or stream is the four bytes `BAC1` and then units. A decoder needs integers only, and two decoders give the
same samples bit for bit, also in lossy mode.

## 1. Primitives

| | |
|---|---|
| varint | an unsigned number, 7 bits per byte, low bits first, top bit set on every byte but the last |
| string | varint length, then that many bytes of UTF-8 |
| attrs | varint count, then per attribute: string key, one byte kind (0 = text, 1 = bytes; any other kind is read as bytes), varint length, value |
| CRC-32 | the one of zip and PNG (reflected polynomial 0xEDB88320, start and final XOR 0xFFFFFFFF), stored low byte first |
| `>>` | on signed numbers always the arithmetic shift (rounds down) |

## 2. Units

```text
0xBA   type (1..127)   length (varint, at most 4,194,304)   payload   CRC-32 of type, length and payload
```

A reader takes units in order and skips types it does not know. Bytes that are not a unit with a correct CRC (damage,
the start of a live stream) are skipped one at a time until the next unit is found.

| Type | Unit | Payload |
|---:|---|---|
| 1 | HEAD | byte version (0) · byte profile (0) · varint sample rate · byte channels (1..8) · byte bits per sample (8..24) · byte flags (bit 0: lossless) · varint frame length in samples (16..16,384) · byte predictor order (1..32) · varint lag_min (2 or more) · varint lag_max (at most 16,384) · varint samples per channel + 1 (0 = not known) · byte quality x 10 (255 = none; for display) |
| 2 | META | attrs of the recording · varint speakers · per speaker: varint id, string name, attrs |
| 3 | EVENTS | varint tiers · per tier: string name · varint events · per event: varint tier index, varint start minus the previous event's start, varint duration, byte flags, [varint speaker id if flag 1], [byte weight if flag 2], string text, [attrs if flag 4]. Times are in samples; events are in order of start |
| 4 | AUDIO | byte flags (1 = key, 2 = short) · byte sequence number · varint frames (1..4096) · [key: varint index of the packet's first sample] · [short: varint samples of the last frame that count, 1..frame length - 1] · the range coder's bytes |
| 5 | INDEX | varint label units · per unit: byte type, varint byte offset · varint key packets · per packet: varint first sample and varint byte offset, each minus the previous entry's |
| 0x7F | END | 8 bytes offset of the INDEX unit · 8 bytes samples per channel (both low byte first). The unit is 23 bytes and ends the file |

HEAD comes first. A live sender repeats it now and then, so that a listener can join. META and EVENTS may come
anywhere, any number of times, and add up: an attribute or speaker replaces an earlier one with the same key or id,
events join the events already read. A finished file has its labels before the audio, so a player has the words
before the sound; a live stream sends them as things are said.

AUDIO units are consecutive. A key packet starts from the reset state (section 4.3) and says where on the time line
it is; any other packet continues the one before, and its sequence number is the previous one plus one (mod 256).
After a gap a decoder drops packets up to the next key packet and outputs silence for the missing time.

INDEX and END are for readers that can seek: the last 23 bytes give the INDEX, the INDEX gives the labels and the
key packet before any sample. A stream without them is complete all the same.

## 3. Labels

Three things, all optional:

- **Attributes** of the recording: free keys (`language`, `title`, `source`, `license`, ...).
- **Speakers** (voice profiles): an id (1 and up), a name, and attributes: `gender`, `age`, `accent`, `style`,
  `personality`, anything. A bytes value can hold a learned voice vector.
- **Events**: a sample range on a named tier, with a text, and optionally a speaker, a weight (0..255: how strong
  or how sure) and attributes.

Tiers every tool should know:

| Tier | Text of an event |
|---|---|
| `text` | what is said in the range, one event per utterance or turn |
| `word` | one word (alignment; also the syllables of a lyric) |
| `phone` | one phone |
| `tag` | a label for the range: `whisper`, `laughing`, `singing`, `shouting`, `crying`, `breath`, `noise`, `music`, ... |

Any other tier name is valid (`emotion`, `style`, `note` for a melody). Events of one tier may overlap. As JSON
(`bac label --export`, `--import`) times are seconds and speakers are named:

```json
{
  "attrs": {"language": "en"},
  "speakers": [{"id": 1, "name": "jax", "attrs": {"style": "dry and calm"}}],
  "events": [
    {"tier": "text", "start": 0, "end": 2.31, "speaker": "jax", "text": "Well, that went well."},
    {"tier": "tag", "start": 1.2, "end": 2.31, "speaker": "jax", "weight": 0.8, "text": "laughing"}
  ]
}
```

## 4. Audio

A packet is frames of `frame length` samples; in each frame the channels follow one another. All of a packet is one
range coder stream.

### 4.1 Range coder

State: `range` = 0xFFFFFFFF, `code` = the first four bytes (big end first). Bytes past the end of the data are zeros.

```text
symbol under a table c[0..n], c[0] = 0, c[n] = 32768:
    r = range >> 15;  v = min(code / r, 32767);  s = the symbol with c[s] <= v < c[s+1]
    code -= r * c[s];  range = (c[s+1] == 32768) ? range - r * c[s] : r * (c[s+1] - c[s])
bits(n), n <= 16:
    range >>= n;  v = min(code / range, 2^n - 1);  code -= v * range
after either, while range < 2^24:  range <<= 8;  code = (code << 8) | next byte
```

More than 16 plain bits are read as the high n - 16 bits, then 16.

### 4.2 Adaptive tables

A table over n symbols has a floor f: 8 for n up to 16, 4 up to 64, 2 above. It starts from weights w:
`c[i] = f*i + (32768 - f*n) * (w[0] + .. + w[i-1]) / (w[0] + .. + w[n-1])`. After every symbol s it moves:

```text
rate = 3 + min(floor(log2(n)), 2) + (count > 15) + (count > 31)     count: symbols seen before, at most 32
for 0 < i < n:   t = (i <= s) ? f*i : 32768 - f*(n - i)
                 c[i] += (t - c[i]) >> rate   if t >= c[i],   else   c[i] -= (c[i] - t) >> rate
```

| Table (per channel) | Symbols | Starting weights w[s] |
|---|---:|---|
| type, one per previous type (4) | 4 | 1 |
| repeat | 2 | 1 |
| predictor index i | 2 L(i) - 1 | 1 + 64 / (1 + min(s, n - s)) |
| step, one for a frame's first subframe and one for the others | 96 | 1 + 64 / (1 + min(s, n - s)) |
| fill | 4 | 1 |
| lag delta | 16 | 1 + 32 / (1 + abs(s - 8)) |
| gain | 8 | 1 |
| shape | 2 | 1 |
| token, 32 of them | 16 | max(4096 >> min(s, 12), 1) |

L(i) = 32 for the first two predictor indices, 16 for the next four, 8 for the rest. Divisions are integer.

### 4.3 State of a channel

The output samples y and the excitation e (at least lag_max + 2 of the past), the predictor indices and
coefficients, the step index, the previous frame type, the noise seed, the running magnitude `avg` with the last two
magnitudes u1 and u2, and the tables. Reset: everything zero, seed 0x2545F491, tables at their starting weights.

### 4.4 A frame

```text
type        0 silent: y and e are zeros for the frame; nothing else changes
            3 copy: y and e of the frame are those of the channel before (same frame); not allowed on the first channel
            1 unvoiced, 2 voiced: as follows
repeat      1: the predictor stays
            0: for every i < order:  index[i] = ((index[i] + L(i) - 1 + symbol) mod (2 L(i) - 1)) - (L(i) - 1)
fill        0..3
four subframes; subframe j is the samples  j * N / 4  up to  (j + 1) * N / 4  of the frame (N = frame length):
    step    step index = (step index + symbol) mod 96;   step = B[index mod 4] << (index / 4),  B = 16, 19, 23, 27   (Q4)
    -- type 2 only:
    lag     first subframe: lag_min + bits(the bit length of lag_max - lag_min);   later: lag + symbol - 8
            a lag outside lag_min..lag_max is an error
    gain    0..7:  G = 0, 26, 45, 64, 83, 102, 122, 141;  0 = no long-term prediction in this subframe
    shape   (gain > 0)  side = (G + 4) >> 3 for shape 0,  (G + 2) >> 2 for shape 1;  taps = side, G - 2 side, side
    every sample: u (4.5), then the sample (4.6)
```

The predictor's coefficients from its indices, order M:

```text
k(i, L) = sign(i) * ((abs(i) * (3 L^2 - i^2) * 2^14 + L^3 / 2) / L^3)          reflection coefficient, Q15
a = empty;  for m = 0 .. M-1, with k = k(index[m], L(m)):
    a[j] = a[j] - ((k * a[m-1-j] + 2^14) >> 15)   for all j < m at once;     a[m] = k << 9        (Q24)
coef[j] = (a[j] + 2^9) >> 10, held within +-2^30                                                  (Q14)
```

### 4.5 A coded integer u

```text
mean = avg >> 5
mean < 2:   k = 0;  a = 0, 1, 2, 3, 4, 5  for avg up to 3, 7, 15, 31, 47, above
otherwise:  lg = floor(log2(mean));  k = lg - 1;  a = 6 + ((mean >> (lg - 1)) & 1)
token = symbol under table  4 a + min((u1 + u2) >> k, 3)
token = 15:  nb = bits(5);  token = 2^nb + bits(nb) + 14
magnitude = (token << k) | bits(k);   if it is not 0, one more bit: 1 = negative
then:  m = min(magnitude, 2^26);  avg += ((m << 5) - avg) >> 4;  u2 = u1;  u1 = m
```

### 4.6 A sample

```text
short = (coef[0] * y[n-1] + .. + coef[M-1] * y[n-M] + 2^13) >> 14
long  = (taps[0] * e[n-lag-1] + taps[1] * e[n-lag] + taps[2] * e[n-lag+1] + 64) >> 7        (0 without gain)
x     = sign(u) * ((step * abs(u) + 8) >> 4)                                                 for u != 0
      = +F or -F with F = (step * (0, 2, 3, 4)[fill] + 128) >> 8                             for u = 0 and F != 0:
        seed = (seed * 1664525 + 1013904223) mod 2^32;  -F if its top bit is set
e[n]  = x + long, held within 32 bits
y[n]  = short + e[n], held within the sample range of the stream's bits per sample
```

With step index 0 everywhere and fill 0, x = u and the output is the input exactly: that is lossless mode, and the
HEAD flag is the encoder's word that it did so.

## 5. What is not in the format

How an encoder picks the predictor, the pitch, the steps, the fill, the packet length and the key interval. The
reference encoder's choices are in `encoder.rs`; a better encoder writes the same format.

## 6. Reserved for later versions

Unit types 6 to 126 (a track of model tokens beside the audio is the first candidate), HEAD profiles above 0 (other
coding tools), the unused flag bits (written as zero), attribute kinds above 1.
