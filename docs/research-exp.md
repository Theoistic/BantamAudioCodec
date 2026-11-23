# Research: smaller files, a better container, and a path to a standard

State: 2026-11-22. An analysis, not a change: no code in this repository was touched for it. It reads the `bac`
crate as it is (format version 0), measures it with the existing `bac bench` and `tools/compare_codecs.py` plus
throwaway scripts, and sets that against codec research from 2020 to 2026. `Research.md` says why Opus, FLAC and AV1
were the templates; this document says what to do next.

Every number carries its origin:

| Mark | Meaning |
|---|---|
| [m] | measured here, on 86 clips (546 s) of the jax set, taken from the lossless 24 kHz FLACs |
| [s] | read in the source given |
| [s2] | seen only in a search summary or in another paper's table: check before relying on it |
| [e] | my estimate or reasoning: not measured, not published |

Two limits of everything below. Nobody has listened to the lossy mode, and no word error rate has been taken through
it: all quality numbers are waveform measures, which favour BAC over Opus. And the clips are synthetic voices
(Qwen3-TTS renders); the lossless LibriSpeech and LJSpeech sources are no longer on this machine.

## 1. Summary

1. **BAC spends the same bits on every frequency band.** Quality 4 costs 1.2 bits per sample at 8, 12.8, 16 and
   24 kHz [m]. The band from 4 to 8 kHz is 46 % of a 16 kHz file, and it is mostly noise that needs no waveform.
   Coding the low band as a waveform and the high band as an envelope with noise is the largest single saving:
   about 22.7 -> 14 kbit/s at 16 kHz and 32.8 -> 15 kbit/s at 24 kHz [e].
2. **Against Opus, BAC needs about 1.6 times the bits today** at its default quality [m], on measures that favour
   BAC. Split-band coding closes that; a better encoder (noise shaping, a search in place of rounding, fractional
   pitch) is what could pass it. Above about 24 kbit/s the best classical codecs are level with each other [s]:
   there is no quality win there, only a feature win.
3. **Against FLAC, BAC wins by 5 %, not by the 9 to 13 % written in `README.md` and `Research.md`.** Those figures
   counted 8 kB of padding that ffmpeg puts into every FLAC file; on short clips that is 7 % [m]. BAC is still the
   smallest of the five lossless coders tried here. The coders that beat FLAC by 15 to 25 % on speech do it with a
   cascade of adaptive filters [s], on 44.1 kHz material; a quick estimate of such a cascade on these 16 and 24 kHz
   clips found under 1 % [m]. Lossless is close to its floor here, and the win over FLAC is in what the file can
   do, not in many more percent.
4. **What no other codec offers is the combination**: a lossy decoder that gives the same samples on every machine,
   one syntax from 7 kbit/s to lossless, labels inside the file, and a decoder of a few hundred lines. Opus requires
   none of the first three [s]. That, and not quality per bit alone, is the claim to build a standard on.
5. **Because lossy decoding is exact, a file can carry its own correction to lossless** as an optional unit, and a
   hash of its decoded samples. Neither is possible for Opus, AAC or MP3.
6. **Words should be an alignment of the transcript, not a second transcript.** The `text` and `word` tiers of the
   sample file already disagree in case and punctuation [m].
7. **The container is sound and misses six things** a standard has: a must-understand flag, two version numbers, a
   sample hash, time ranges for labels in the index, tracks, and padding.
8. **A standard is three documents** (bitstream, file framing, mappings into Matroska, MP4 and RTP), test vectors, a
   second implementation and a patent review.
9. **First, before any of it: listen, and take word error rates.** Half of the decisions below turn on what the ear
   and the hearing model say, and one earlier decision (no noise in place of unvoiced speech) was taken on measures
   that cannot judge it.

## 2. Where BAC stands today

### 2.1 Lossy, against Opus

16 kHz mono, 86 clips, `tools/compare_codecs.py`, libopus through ffmpeg 8.1 with `-application voip` [m]. NMR is
the noise-to-mask ratio and band is the band distance of `metrics.rs`; lower is better for both.

| Codec | kbit/s | MB/hour | SNR dB | NMR dB | Band dB |
|---|---:|---:|---:|---:|---:|
| Opus 6k | 6.6 | 3.0 | 6.2 | 9.0 | 8.00 |
| Opus 8k | 8.3 | 3.8 | 8.2 | 7.1 | 7.62 |
| Opus 10k | 10.5 | 4.7 | 8.0 | 7.4 | 2.79 |
| **BAC -q 2** | 10.9 | 4.9 | 9.6 | 9.8 | 4.56 |
| Opus 12k | 12.3 | 5.5 | 8.7 | 6.1 | 2.46 |
| Opus 16k | 15.9 | 7.2 | 9.3 | 4.5 | 2.18 |
| **BAC -q 3** | 16.4 | 7.4 | 13.7 | 7.8 | 3.55 |
| **BAC -q 4** | 22.7 | 10.2 | 18.3 | 5.4 | 2.69 |
| Opus 24k | 23.4 | 10.5 | 9.7 | 2.5 | 1.92 |
| **BAC -q 5** | 29.4 | 13.2 | 22.6 | 2.7 | 1.96 |
| Opus 32k | 30.8 | 13.9 | 9.9 | 1.2 | 1.71 |
| **BAC -q 6** | 36.2 | 16.3 | 26.7 | 0.2 | 1.43 |

- BAC at quality 4 (22.7 kbit/s) measures like Opus at about 14 kbit/s by NMR and like Opus at about 11 by band
  distance. BAC needs 1.6 to 2 times the bits.
- BAC's SNR is 9 dB higher than Opus's at equal rate. That is the problem stated as a number: BAC pays to keep a
  waveform, Opus pays to keep a sound.
- Opus at 6 and 8 kbit/s is narrowband: its band distance jumps from 2.8 to 7.6 below 10 kbit/s. That gap is where
  a wideband codec at 6 to 10 kbit/s can win clearly (section 5).
- Above 30 kbit/s the two are level on these measures, as `Research.md` found on LibriSpeech.

### 2.2 The cost of bandwidth

The same clips resampled; `bac bench --bits`, quality 4 [m]:

| Sample rate | Band | kbit/s | Sample bits, kbit/s | Bits per sample | Side information, kbit/s |
|---|---|---:|---:|---:|---:|
| 8 kHz | 0-4 kHz | 12.2 | 9.3 | 1.16 | 2.9 |
| 12.8 kHz | 0-6.4 kHz | 18.4 | 15.1 | 1.18 | 3.3 |
| 16 kHz | 0-8 kHz | 22.8 | 19.1 | 1.20 | 3.6 |
| 24 kHz | 0-12 kHz | 32.8 | 28.9 | 1.20 | 4.0 |

The step of a subframe is a fixed multiple of the size of the prediction residual (`encoder.rs`, "the step"), so
every sample gets about the same number of bits whatever band its energy is in. Going from 8 to 16 kHz adds
10.6 kbit/s; going from 16 to 24 kHz adds 10 more. At every quality the picture is the same (quality 2: 6.3, 8.9,
10.9, 15.4 kbit/s).

Side information at 16 kHz: predictor 1.48, pitch 1.11, steps 0.55, frame flags 0.18, container 0.31 kbit/s. It does
not shrink with quality, so it is a third of a quality 2 file. Of the frames 67 % are voiced, 13 % are coded as
background, 44 % keep the predictor of the frame before.

### 2.3 Lossless, against FLAC and others

The 86 clips, each as one file, every other coder through ffmpeg 8.1 [m]:

| Coder | 16 kHz, kbit/s | against FLAC -12 | 24 kHz, kbit/s | against FLAC -12 |
|---|---:|---:|---:|---:|
| WAV | 256.1 | | 384.1 | |
| FLAC -12 as ffmpeg writes it (8 kB padding per file) | 155.2 | +7.1 % | 214.2 | +5.1 % |
| ALAC | 151.0 | +4.2 % | 209.3 | +2.7 % |
| WavPack, default | 149.2 | +3.0 % | 208.5 | +2.3 % |
| FLAC -5, padding taken off | 146.9 | +1.4 % | 206.9 | +1.5 % |
| FLAC -8, padding taken off | 146.0 | +0.8 % | 205.4 | +0.8 % |
| **FLAC -12, padding taken off** | **144.9** | | **203.9** | |
| TTA | 143.5 | -0.9 % | 200.3 | -1.7 % |
| WavPack, level 8 | 141.4 | -2.4 % | 198.5 | -2.6 % |
| **BAC lossless** | **137.1** | **-5.4 %** | **193.3** | **-5.2 %** |

- **A correction to the earlier documents.** ffmpeg's FLAC writer reserves 8,192 bytes of padding in every file. On
  clips of six seconds that is 10 kbit/s. `tools/compare_codecs.py` counts the whole file, so the "9 % under FLAC
  on LibriSpeech, 13 % on the jax voice" of `README.md` and `Research.md` is mostly padding: taken off, those two
  measurements come to about 4.4 % and 5.4 % [e, recomputed from the clip counts]. In a file of an hour the padding
  does not matter, and 5 % is what BAC saves.
- BAC's figure includes its own container (0.3 kbit/s).
- ffmpeg's FLAC and WavPack encoders are not the reference programs; `flac -8` and `wavpack -hhx` may differ by a
  percent either way. Monkey's Audio, OptimFROG and MPEG-4 ALS were not on this machine.

### 2.4 Labels

The sample conversation (29.7 s, two speakers, 63 words) holds 1,462 bytes of labels: 394 bit/s [m]. About 15 bytes
go to each word event, of which the word's own letters are 4. Against 28.6 kbit/s of audio that is 1.4 %; against a
14 kbit/s file it is 2.8 %, against a 7 kbit/s file 5.6 %. Labels are stored as plain bytes, not compressed.

## 3. Smaller lossy files

### 3.1 Two bands, two representations

The question that started this: could low notes have longer frames and high notes shorter ones? In BAC as it is,
nothing has a frame in that sense. It codes every sample, so its time resolution is one sample at every frequency;
the 20 ms frame and the 5 ms subframe only say how often the predictor, the pitch and the step may change.

What is right in the idea is that the two halves of the spectrum want different things:

| | Low band (below 4 to 6.4 kHz) | High band (above) |
|---|---|---|
| Holds | pitch harmonics, the formants | fricatives, breath, the edge of plosives |
| Changes | slowly: a vowel is steady for 50 to 200 ms | fast: a burst is 5 to 10 ms |
| The ear wants | the waveform, or close to it | the right loudness at the right moment, the right colour |
| So code | samples, with long prediction memory | a spectral envelope per 20 ms, a gain per 1.25 to 5 ms, noise |
| Costs | 1.2 bits per sample [m] | 1 to 2 kbit/s in all [s] |

So the high band does get the finer time grid, as the idea says, but the saving comes from not sending its waveform
at all. This is what speech codecs have converged on:

- EVS codes 6.4 to 14.4 kHz with its time-domain bandwidth extension for about 1.55 kbit/s at 13.2 kbit/s total [s2].
- G.729.1 codes 4 to 7 kHz as 16 time-envelope values of 1.25 ms and 12 band energies per 20 ms, for about
  2 kbit/s [s].
- Opus's hybrid mode codes speech up to 8 kHz in its speech layer and only the band above in its transform layer [s].
- Meta's MLow (2024) is a split-band CELP codec with a very cheap high band: POLQA 3.9 against Opus's 1.89 at
  6 kbit/s, wideband against narrowband [s].
- Microsoft's Satin (2021) codes only the low band and lets a network infer the high band from it and a little side
  information: super-wideband speech from 6 kbit/s [s].
- Opus 1.6 (December 2025) extends 8 kHz of bandwidth to 20 kHz with no bits at all; 9 kbit/s with its enhancer and
  this extension matches Opus 1.4 fullband at 18 [s].

What the research does not support is a fixed rule "long frames low, short frames high" as a source of gain by
itself. Transform codecs switch time against frequency resolution by signal, not by rule: CELT per band and per
frame [s], MP3's mixed blocks, ATRAC's block length per band. Among neural codecs none found gives each band its own
frame rate; band-split codecs keep one rate and vary the bits, and one of them found that splitting the low band of
speech finely hurt (UTMOS 2.52 with five bands against 3.36 with one band up to 2 kHz) [s]. The measured gains from
variable time resolution are by content: steady vowels and silence get fewer updates (section 3.4).

**For BAC: a second profile** (the HEAD unit has the field).

- A core at 8 kHz or 12.8 kHz, coded exactly as today.
- Above it, per 20 ms: an envelope of 6 to 8 reflection coefficients on the existing grid, four to sixteen gains,
  and a mix flag. The decoder makes the high band's excitation from its own noise generator and from the core's
  excitation moved up, shapes it, and adds the two bands with a fixed integer filter pair. All of it is integer
  arithmetic, so the decoder stays exact.
- Lossless and the high qualities keep profile 0 (the whole band as samples): a parametric band cannot be refined
  to exact.

| | Today | 12.8 kHz core + band | 8 kHz core + band |
|---|---:|---:|---:|
| 16 kHz, quality 4 | 22.8 [m] | 18.4 + 0.8 = 19 [e] | 12.2 + 1.5 = 14 [e] |
| 24 kHz, quality 4 | 32.8 [m] | 18.4 + 1.5 = 20 [e] | 12.2 + 2.0 = 14 [e] |
| 16 kHz, quality 2 | 10.9 [m] | 8.9 + 0.8 = 10 [e] | 6.3 + 1.5 = 8 [e] |

The core figures are measured; the band figures are what published codecs spend. Which split sounds right is a
listening question: 6.4 kHz is the safe choice of AMR-WB and EVS, 4 kHz saves twice as much and puts the upper
formants of some voices into the parametric band.

Patents: EVS (2014) is licensed, and its bandwidth extension should not be the model. AMR-WB (2001) makes its top
band from noise shaped by an extended predictor and a gain, and G.729.1 dates from 2006; a design from art that old
is the safer ground. This needs a real review, not this paragraph.

### 3.2 The encoder, in the format as it is

None of these changes a file's syntax; old decoders read the result.

1. **Shape the noise as Opus's speech layer does.** BAC feeds the rounding error back through its coding predictor
   with the poles pulled in by one factor (`encoder.rs`, `shape`). SILK runs a separate shaping analysis with two
   bandwidth factors, adds a spectral tilt, and on voiced frames shapes the noise between the pitch harmonics with
   taps (0.25, 0.5, 0.25) at the pitch lag [s]. Noise between harmonics is what makes a coarsely coded voice hoarse.
2. **Search, do not round.** Each sample is rounded on its own with a dead zone. SILK searches with delayed
   decisions over 32 samples [s]; CELP codecs choose a whole subframe's pulses by analysis-by-synthesis. At
   quality 2 most coded values are 0 and the rest are +-1: that is a pulse codebook already, and the decoder side
   of a pulse codebook is trivial. An encoder that picks the pulses of a subframe together, against a weighted
   error, is the classical way to make a coder of this kind work under 1 bit per sample [e].
3. **Try noise for unvoiced speech again, by ear.** It "was worse on every measure" (`Research.md`), and every one of
   those measures counts replaced noise as error by construction. EVS's unvoiced mode drops the pitch codebook and
   uses a Gaussian excitation [s]. This is the same question as 3.1, inside the low band.
4. **Spend bits by what is heard.** A constant step-to-residual ratio gives a quiet fricative as many bits per sample
   as a loud vowel. A loudness model per subframe, and a look ahead over the whole file (this is a file codec: two
   passes are free), would move bits to where they are heard [e]. No published figure was found for what a storage
   codec gains from long look-ahead over a conversational one; treat it as unproven.
5. **Track the pitch.** The lag is searched per frame in the open loop, and inside a subframe shorter than the lag
   the unquantised residual stands in for the excitation the decoder will have. A pitch track over several frames
   and a closed-loop search against the real excitation remove octave jumps and the mismatch at coarse steps.

### 3.3 Format tools, to decide before version 1

1. **Fractional pitch lags.** Lags are whole samples and the three taps are symmetric (`codec.rs`, `ltp_taps`). A
   symmetric filter cannot shift by half a sample; it only blurs. At 16 kHz a lag error of half a sample turns the
   prediction of a 2 kHz harmonic by 22 degrees and of a 4 kHz harmonic by 45 [e]. High voices have lags of 30 to 80
   samples, where this matters most. AMR-WB and EVS use quarter-sample lags; Kroon and Atal's 1991 paper reports
   the gain "especially for female speakers" [s2]. `Research.md` calls long-term prediction "the weakest tool so
   far"; this is the likely reason. Cost: 2 bits per subframe, or asymmetric tap shapes.
2. **Starting tables learned from speech.** Every key packet starts all tables from formulas and relearns them. One
   key packet per second in place of one per two seconds costs 0.2 kbit/s [m, `Research.md`]. Tables trained on a
   corpus and frozen in the specification give most of that back, make short files smaller, and make short
   independent packets affordable for live use. Opus uses fixed tables for exactly that robustness [s].
3. **Frame types that mean something.** EVS has six coding modes (inactive, unvoiced, voiced, transition, generic,
   audio) and moves bits between its tools by mode [s]. BAC has voiced and unvoiced and a background flag. A
   transition type (an onset: no useful past for pitch, fine steps for 5 ms) and an inactive type (a noise
   description every 100 ms and nothing else) are the two that pay in speech. A talker in a conversation is active
   about 40 % of the time [s2]; EVS's variable-rate mode averages 5.9 kbit/s on that basis [s].
4. **Slower side information in steady sound.** 44 % of frames already keep their predictor. Sending a predictor
   every 40 ms and interpolating, and a step and pitch contour per frame in place of four deltas, is the classical
   form of what the variable-rate tokenizers measure: CodecSlime merges steady frames and cut word errors by 32 %
   at equal rate, VARSTok beat a fixed 40 Hz tokenizer at 31 Hz on average [s].
5. **The predictor's bits.** 53 bits per new predictor at order 16 [m]. SILK spends a 5-bit first stage and then
   small corrections, about 30 to 37 bits [s, my arithmetic], and interpolates. Half a kbit/s is there at low rates.
6. **A long-term postfilter** (LC3, CELT's comb filter [s]) as a decoder tool for the low qualities. It changes the
   output, so it must be integer and switched off for lossless.

### 3.4 A learned enhancer after the decoder

Opus kept its bitstream and put networks behind the decoder: LACE (about 100 MFLOPS) and NoLACE (about 400 MFLOPS)
steer classical pitch and spectral postfilters, and 9 kbit/s with NoLACE beats plain 12 kbit/s [s]. For BAC this is
an optional stage after the exact decoder: the file and the conformance point stay the same, a player may make it
sound better. It belongs after sections 3.1 to 3.3, and it would run on the native runtime.

### 3.5 What it adds up to

| Step | 16 kHz, quality 4 | Basis |
|---|---:|---|
| Today | 22.7 kbit/s | [m] |
| Split band at 4 kHz | about 14 | [e] from 2.2 and published band costs |
| + fractional pitch, tracking, trained tables, slower side information | about 12 to 13 | [e] |
| + shaping and search: the same sound at a coarser step | about 9 to 11 | [e], the least certain line |
| Opus at the same NMR today | about 14 | [m] |

The last line of estimates is where BAC would pass Opus on its own ground. None of it is known until it is built
and heard.

## 4. Smaller lossless files

### 4.1 What the strongest coders do

The best table found is in the Sac repository: 16 clips at 44.1 kHz stereo, FLAC 1.5 `-8pe` as the base [s].

| Coder | Method | Against FLAC, mean | On the two speech clips | Decoding |
|---|---|---:|---:|---|
| Monkey's Audio, extra high | sign-LMS filters of 16, 256 and 1,280 taps in a row, range coder | -5.5 % | -2.2 %, -6.5 % | as slow as encoding |
| MPEG-4 ALS, RLS-LMS mode | backward-adaptive cascade, block Gilbert-Moore coder | -6.6 % | -11.2 %, -14.3 % | slow |
| OptimFROG, max | closed; a cascade with mixing of its stages | -9.0 % | -15.2 %, -24.2 % | slow |
| Sac | least squares per frame, then an NLMS cascade, bias correction, a bit-plane coder with mixing | -9.9 to -11.0 % | -17.9 %, -24.7 % | 20 to 60 s for 289 s; floating point, output depends on the machine |
| Transformer over bytes (Trilobyte, 2026) | 90 million weights and an arithmetic coder | LibriSpeech: 2.11x against FLAC's 1.74x, -17.5 % | | "orders of magnitude" slower; loses to FLAC at 24 bits |

Single mechanisms, where a figure exists [s]: a better entropy coder with a predictor of order 30 in place of Rice
codes with order 15, about 1 %; bias cancellation, up to 1 %; prediction between channels, 3 % on average.

### 4.2 The same idea tried on these clips

A script, not the codec: BAC's predictor rebuilt (one per 20 ms, unquantised), variants after it, and every residual
priced by the entropy of BAC's own token under BAC's own contexts. Ten clips, 56 s; side information not counted [m].

| Variant | 16 kHz, kbit/s | | 24 kHz, kbit/s | |
|---|---:|---:|---:|---:|
| A: predictor of order 16 (20 at 24 kHz) | 133.7 | | 189.1 | |
| C: A + three-tap pitch prediction per 5 ms, as BAC has | 130.7 | -2.2 % | 186.2 | -1.5 % |
| C + NLMS filter of 256 taps on what is left | 130.5 | -2.4 % | 185.9 | -1.7 % |
| C + NLMS filter of 32 taps on what is left | 130.3 | -2.6 % | 185.6 | -1.9 % |
| Predictor of order 32 + pitch (its extra side bits not counted) | 129.6 | -3.1 % | 184.5 | -2.5 % |
| An NLMS filter of 32 taps alone, no predictor | 158.0 | +18 % | 225.9 | +19 % |

- An adaptive filter after BAC's predictor and pitch filter gains 0.1 to 0.4 %. A predictor of twice the order gains
  0.9 % and would spend about that much on its coefficients.
- An adaptive filter alone is 18 % worse than a predictor sent every 20 ms: speech changes faster than a plain
  filter of that kind follows.
- This is weak evidence against the cascade, not proof: one filter with one step size, 56 s of synthetic voices, a
  cost model. But it fits the published table, where the large gains are on 44.1 kHz recordings. Speech sampled at
  44.1 kHz is nearly empty above 8 to 10 kHz, and a long filter predicts an empty band very well. At 16 kHz the
  speech fills the band and there is little left to find.

### 4.3 What is left

1. **The entropy coder**: two models mixed (one fast, one slow table per context), and a correction stage on the
   coded probability. About 1 % each in the literature [s]; 1 to 3 % together is a fair hope [e].
2. **Structure that is not sound**, each near zero on a clean recording and large where it applies:
   - Band-limited material: telephone speech stored at 16 kHz, or anything upsampled. A long-predictor mode for such
     files; one report has Monkey's Audio 18 % under FLAC on upsampled audio [s2].
   - Unused low bits (FLAC, WavPack and OptimFROG detect them [s]); exact repeats; digital silence (BAC has it).
   - Audio that was lossy before. LibriSpeech was made from MP3 files [e, from memory]; some of what the
     transformer finds there may be that.
3. **The correction unit** (section 8): no gain in size, but the lossy file and the exact recording become one file
   whose larger part can be left at home. WavPack does this with two files [s]. Lossy plus correction is usually a
   few percent larger than plain lossless [e]; to be measured.
4. **Starting tables learned from speech**: helps short files most, as in 3.3.

By classical means BAC's lossless mode can gain a few percent more. The transformer's 17.5 % shows that more is
there to be found, at a cost no file format can ask of a player.

## 5. Beating Opus and FLAC

**FLAC** is beaten today by 5 % on size (section 2.3), and section 4 says not to expect much more. It is beaten
more clearly on what the file holds: one file from lossy to exact, labels, a check of the decoded samples. What FLAC
keeps: it is everywhere, it has a specification (RFC 9639), and its decoder is faster than anything with adaptive
tables. BAC decodes 16 kHz speech at 1,700 times real time on one quiet core (`Research.md`); that is enough for a
data loader, and it should be measured against FLAC's decoder on the same machine before anyone claims it.

**Opus** cannot be beaten everywhere, and it does not have to be:

| Range | State of the art | What BAC can do |
|---|---|---|
| 6 to 10 kbit/s | Opus is narrowband [m]; MLow and EVS are wideband or more and far ahead of it [s] | A wideband or 24 kHz file at 8 to 10 kbit/s with the split band and a searching encoder. The clearest place to win |
| 10 to 16 kbit/s | EVS leads Opus: POLQA 4.34 against 4.03 at 16 kbit/s fullband [s]; MOS 6.38 (EVS wideband) and 7.10 (EVS super-wideband) against 5.49 (Opus) at 13.2, on a 9-point scale, 2016 [s]. The gap is bandwidth extension and mode-switched coding | The same tools: sections 3.1 to 3.3 |
| 24 kbit/s and up | EVS and Opus "perform almost identically" at 24.4 [s] | Level at best. Win on what follows |

What Opus does not have, and a format for stored speech wants:

- **Exact decoding.** Opus's conformance is a quality threshold on the output and an equal final coder state; equal
  samples are not required, and float builds differ by compiler and processor [s]. A training set coded in BAC is
  the same numbers everywhere.
- **Lossless in the same syntax**, and a correction unit that turns a lossy file into the exact recording
  (section 4).
- **Labels** on the audio's own time line, in the file.
- **Tables that follow the speaker across packets.** Opus resets with every packet because packets get lost. A file
  does not lose packets. No published figure for the cost was found.
- **No per-packet overhead**: Opus in Ogg at 16 kbit/s carries a table-of-contents byte per 20 ms and page headers.
- **A measured operating point for training.** Opus at 16 kbit/s cost 10 % relative word error with matched
  training, 3.3 % at 32 kbit/s, 1.6 % at 128 [s]; GigaSpeech at 32 kbit/s lost 0.1 to 0.2 points [s]. No such
  number exists for BAC. A codec that publishes "quality N costs X word errors and Y in synthesis" per setting
  would be the first.

## 6. What the newer codecs teach

Read for ideas. A neural codec as the base stays ruled out for the reasons in `Research.md`, and the research since
adds two: the low-frame-rate tokenizers need decoders of 60 to 950 million weights, and their scores mislead
(FocalCodec at 12.5 Hz scores UTMOS 4.22, above the original's 4.09, while word errors through it rise from 2.18 to
7.94 %) [s].

| Idea | Where | Use in BAC |
|---|---|---|
| Parametric or inferred high band | EVS, G.729.1, MLow 2024, Satin 2021, Opus 1.6 2025 | 3.1 |
| Learned postfilter behind a classical decoder | Opus 1.5 LACE and NoLACE 2024 | 3.4 |
| Time resolution by content | CodecSlime, VARSTok, TFC, all 2025 | 3.3, items 3 and 4 |
| Coarse information at a low rate, detail at a high one | SNAC 2024 (0.98 kbit/s scored MUSHRA 88.4 against DAC's 85.0 at 2.5) | envelope per 20 to 40 ms, gains per 1.25 to 5 ms |
| Noise generated in the decoder | SNAC's noise blocks; EVS unvoiced mode | 3.1, 3.2 item 3 |
| Predicting parameters from the frames before | TF-Codec 2022 | contexts for predictor and step from the last frame |
| One code per recording for what does not change | BiCodec, LSCodec, TiCodec 2023 to 2025 | per-file starting tables or a mean predictor in the header; small |
| Redundancy against loss | Opus DRED 2024 | only for live use; a unit type of its own |
| A fixed budget for complexity | LRAC 2025 challenge: 300 to 700 MFLOP/s | BAC's decoder is orders below; say so with a number |
| Judge by more than one score | Codec-SUPERB 2024, WARP-Q 2021, SCOREQ 2024 | 10.1 |
| A language model as the lossless coder | Trilobyte 2026: 2.11x on LibriSpeech with 90M weights | the bound in section 4, not a design |

## 7. The transcript: words or utterances

Both, with the utterance as the unit and words as its alignment.

**Why the utterance is canonical.** It is what people read and what recognition and synthesis train on: text with
its case, punctuation and numbers as written. WebVTT (cues, with optional time stamps inside a cue for words),
Lhotse (a supervision with an `alignment` of words or phones), and the Podcasting 2.0 transcript all have this
shape [s]. Words alone lose it, and "word" is not a unit in languages written without spaces.

**Why words belong in the file.** Seeking and highlighting; cutting training clips at word boundaries; alignment
targets for a duplex model without running an aligner again; editing sound by editing text.

**What is wrong today.** A `word` event carries its own copy of the word. In the sample file the `text` tier says
"okay so like, how old are you even?" and the `word` tier says "Okay," "so" "like" [m]: two transcripts, already
different. Aligners also leave some tokens without a time ("2014.", "£13.60") [s], and the format has no way to say
that.

**Proposal.**

| | |
|---|---|
| Segment | an event of the `text` tier, as now, plus a language tag (BCP 47) |
| Word | start, duration, and a span (byte offset and length) of its segment's text; optional confidence. No text of its own |
| Unaligned word | a span with no time |
| Other alignments | `phone`, `char`, `syllable`, `token` by the same rule: spans of a parent, or free text when there is no parent |
| Origin | per tier: who or what made it (a person, a model and its version) |
| Size | about 4 to 5 bytes per word against 15 today [e]: 80 bit/s against 265 |

Word level should be optional in the format and expected by the tools: `bac info` should say when a file has
utterances without words.

## 8. The container

What is there is right in kind: a sync byte, a type, a length, a CRC per unit, readers skip unknown types, key
packets, an index at the end that a stream does not need. Ogg, Matroska and FLAC are built from the same parts [s].
What a standard would add:

| Gap | Today | Proposal | Model |
|---|---|---|---|
| Unknown units | all skipped | one bit of the type says "skip me" or "stop if you do not know me". A reader that skips a unit that changes decoding gives noise without an error | PNG chunks, Matroska |
| Versions | one byte | "written by" and "least version that can read" | Matroska DocTypeReadVersion |
| Whole-file check | none | a hash of the decoded samples in the END unit. The decoder is exact, so this works for lossy files too: every file tests its decoder | FLAC's MD5, lossless only |
| Correction | none | an optional unit holding original minus decoded: lossy now, exact when wanted | WavPack hybrid, as one file |
| Labels and time | the index lists label units without times; all labels come before the audio | EVENTS units per stretch of time near their audio, with first and last sample in the index | Matroska cues |
| Tracks | channels only | a track table: audio tracks of their own rate, a token track, a second language of labels | Matroska, MP4 |
| Editing | every label edit rewrites the file | a padding unit after the labels | FLAC PADDING, Matroska Void |
| Resync | a false 0xBA can ask for 4 MB before its CRC fails | a one-byte check on the unit header | FLAC, Ogg |
| Label size | plain bytes | the label units through the range coder with a small text model | |
| Vocabulary | free keys | a registry: tier names, attribute keys, tag words; keys from outside as reversed domain names | Matroska tags, IANA |
| Origin and consent | none | a reserved unit for a signed manifest (C2PA), and plain attributes `synthetic`, `voice-consent`. This format stores cloned voices: it should be able to say so | C2PA |
| Live use | packets of 0.5 s; a lost packet costs up to a second | with trained tables (3.3): packets of 20 ms that stand alone, as an RTP payload | RFC 7587 |
| Streamable subset | not defined | a named subset a hardware or web player can rely on | FLAC |

Unit types 6 to 126 and the HEAD profile are reserved already; the format has room for all of this.

## 9. The path to a standard

How the ones that made it did it: Opus through an IETF working group, with the reference decoder inside RFC 6716,
test vectors, and separate documents for RTP, Ogg and MP4; FLAC and Matroska through the IETF's CELLAR group after
years of use (RFC 9639 and RFC 9559, both 2024); LC3 through the Bluetooth SIG [s]. No neural codec is a standard:
the IETF's machine-learning codec group only admits extensions that stay compatible with Opus, and is still
discussing where model weights would even be published [s].

1. **Split the specification in three.** The packet bitstream with its exact decoder. The native file framing
   (units, labels, index). Mappings: a Matroska codec ID with the HEAD unit as private data, an MP4 sample entry, an
   RTP payload. Opus defines no container at all; FLAC has its own framing and the mappings. A codec that only
   lives in its own container is not carried by browsers, editors and phones.
2. **Specify the labels as a format of their own** that can also travel as a track or beside other audio, with
   exact export to WebVTT and to the Podcasting 2.0 JSON.
3. **Freeze version 1** only after sections 3.1, 3.3 and 8 are decided: every one of them changes the syntax.
4. **Conformance by equality.** Test vectors with the hash of their output. Opus needed a tolerance; BAC does not.
5. **A second decoder by someone else**, from the document alone. `Format.md` claims to be enough; this is the test.
6. **Fuzzing and limits.** Header values that are trusted are FLAC's cited source of overflows [s].
7. **A patent review and a royalty-free statement.** The core (linear prediction, pitch prediction, noise feedback,
   range coding) is art of the 1980s to 2000s. The tools of section 3 need checking one by one.
8. **Registrations**: a media type, a file extension checked for collisions (`.bac` is used by backup tools [e]), a
   Matroska codec ID, an MP4 code.
9. **Users before committees.** FLAC and Matroska were standards in use long before they were RFCs. The first
   users here are speech datasets: a corpus published as BAC with its transcripts inside, and loaders for the
   common training tools, is worth more than a draft.

## 10. What would make it a breakthrough

Not quality per bit alone. The published evidence says the best classical codecs are level at 24 kbit/s, and that a
noise-feedback coder of BAC's family is behind below 16 without bandwidth extension and a searching encoder. With
them BAC reaches the field; it does not leave it.

What nothing else is, all at once:

1. **Exact at every quality.** The same samples from every decoder, lossy included, with a hash in the file.
2. **One file from 8 kbit/s to the exact recording**, with the correction as a part that can be left out.
3. **The smallest fast lossless coder for speech**: 5 % under FLAC, 3 % under the best other coder tried here, in
   integers. A lead, not a leap (section 4).
4. **The transcript, the words, the speakers and the tags inside**, on the sample grid.
5. **A measured cost for training**: word errors and synthesis quality per setting, published with the codec.
6. **Small enough to implement in a day**: a decoder of a few hundred lines, no tables to license.

That is a format for stored speech: datasets, archives, audiobooks, recorded calls, voice notes with their text. It
is the place FLAC with a JSON file beside it holds today, and where Opus is used for lack of anything exact. For
live calls Opus stays the answer.

## 11. Experiments, in the order of what they decide

1. **Listen** to `samples/` at quality 2 to 6. Decides whether the lossy mode is usable at all, and at which step
   the artefacts start.
2. **Word errors through the codec**, per quality, with the hearing model, on audio that has a lossless source
   (the jax FLACs are on this machine; LibriSpeech test-clean would have to be fetched again). Decides the default
   quality for training data.
3. **The split, by hand**: low band through BAC, high band replaced by shaped noise with the right envelope, mixed
   in a script. No codec work. Decides 4 kHz against 6.4 kHz, and whether 3.1 is worth building.
4. **Unvoiced frames as noise**, heard and not measured.
5. **Fractional lags**: prediction gain of the pitch filter with quarter-sample lags against whole ones, in a
   script, on high and low voices. Decides 3.3 item 1.
6. **Lossless on real recordings**: section 4.2 again on speech that was recorded, not synthesised, and at
   44.1 kHz, with a tuned cascade. Decides whether the cascade is dropped for good. Then the two-model coder.
7. **Trained starting tables**: bits saved on files of 2, 10 and 60 seconds.
8. **Decoder speed against FLAC and Opus**, one core, same machine.

Steps 1 and 2 need no code. Steps 3 to 7 are scripts and encoder experiments. Only the building of 3.1 and 3.3
touches the format.

One thing to put right whatever is chosen: `tools/compare_codecs.py` should take FLAC's padding off, and the two
figures in `README.md` and `Research.md` should follow (section 2.3).

## Sources

Measurements: `bac bench --bits`, `tools/compare_codecs.py`, and scripts kept outside the repository.

Codecs, classical and hybrid
1. [RFC 6716: Opus](https://www.rfc-editor.org/rfc/rfc6716.txt) (SILK shaping, delayed decision, long-term prediction, conformance)
2. [Opus 1.3](https://jmvalin.ca/opus/opus-1.3/), [Opus 1.5](https://opus-codec.org/demo/opus-1.5/), [Opus 1.6](https://opus-codec.org/demo/opus-1.6/)
3. [NoLACE](https://arxiv.org/abs/2309.14521), [Opus DRED draft](https://datatracker.ietf.org/doc/draft-ietf-mlcodec-opus-dred/)
4. [CELT: time-frequency resolution per band](https://arxiv.org/pdf/1602.04845)
5. [Overview of the EVS codec architecture](https://www.ericsson.com/en/reports-and-papers/research-papers/overview-of-the-evs-codec-architecture), [EVS coding modes](https://itecspec.com/spec/3gpp-26-445-5-1-13-coding-mode-determination), [EVS bandwidth extension, abstract](https://search.isc.ac/Inventory/49/3435350.htm)
6. [G.729.1 time-domain bandwidth extension](https://www.isca-archive.org/interspeech_2007/geiser07_interspeech.pdf)
7. [MLow, Meta 2024](https://engineering.fb.com/2024/06/13/web/mlow-metas-low-bitrate-audio-codec/)
8. [Satin, Microsoft 2021](https://techcommunity.microsoft.com/t5/microsoft-teams-blog/satin-microsoft-s-latest-ai-powered-audio-codec-for-real-time/ba-p/2141382)
9. [LC3 tools](https://ar5iv.labs.arxiv.org/html/2201.13093)
10. [Rämö et al., EVS, Opus and AMR-WB listening test, Interspeech 2016](https://www.isca-archive.org/interspeech_2016/ramo16_interspeech.pdf)
11. [OpenACE: codec evaluation, 2024](https://arxiv.org/pdf/2409.08374)
12. [LPCNet at 1.6 kbit/s](https://ar5iv.labs.arxiv.org/html/1903.12087), [FARGAN](https://arxiv.org/abs/2405.21069)

Neural codecs and their evaluation
13. [SoundStream](https://arxiv.org/abs/2107.03312), [EnCodec](https://arxiv.org/abs/2210.13438), [DAC](https://arxiv.org/abs/2306.06546), [Mimi / Moshi](https://arxiv.org/html/2410.00037)
14. [SNAC](https://arxiv.org/html/2410.14411), [BSCodec](https://arxiv.org/html/2511.06150v1), [TF-Codec](https://arxiv.org/abs/2207.08363)
15. [CodecSlime](https://arxiv.org/html/2506.21074v2), [VARSTok](https://arxiv.org/html/2509.04685v3), [TFC](https://arxiv.org/html/2505.16845v1)
16. [BigCodec](https://arxiv.org/html/2409.05377), [TAAE](https://arxiv.org/html/2411.19842), [FocalCodec](https://arxiv.org/html/2502.04465), [BiCodec](https://arxiv.org/html/2503.01710), [LSCodec](https://arxiv.org/html/2410.15764), [TiCodec](https://arxiv.org/abs/2310.00014)
17. [LRAC 2025 challenge](https://arxiv.org/html/2510.23312v2), [Codec-SUPERB](https://arxiv.org/abs/2402.13071), [WARP-Q](https://arxiv.org/abs/2102.10449), [SCOREQ](https://arxiv.org/abs/2410.06675)

Lossless
18. [RFC 9639: FLAC](https://www.rfc-editor.org/rfc/rfc9639.txt), [FLAC comparison](https://xiph.org/flac/comparison.pdf)
19. [Sac: lossless audio with OLS, LMS cascade and bias stages](https://github.com/slmdev/sac) (its table also gives Monkey's Audio, OptimFROG and MPEG-4 ALS on the same clips)
20. [Monkey's Audio decoder in FFmpeg](https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/apedec.c)
21. [MPEG-4 ALS, Liebchen 2009](https://koreascience.kr/article/JAKO200909659871856.pdf)
22. [Ulacha and Stasiński: cascaded OLS and NLMS, 2015](https://www.czasopisma.pan.pl//Content/87842/PDF/29.pdf)
23. [Lossless audio with a language model (Trilobyte), 2026](https://arxiv.org/abs/2603.08683)

Training on coded audio
24. [GigaSpeech](https://arxiv.org/abs/2106.06909), [Mixed-bitrate Opus for acoustic modelling, Khare et al. 2020](https://arxiv.org/pdf/2002.00122)

Containers, transcripts, standards
25. [RFC 9559: Matroska](https://www.rfc-editor.org/rfc/rfc9559.txt), [RFC 3533: Ogg](https://www.rfc-editor.org/rfc/rfc3533.txt), [Opus in ISOBMFF](https://opus-codec.org/docs/opus_in_isobmff.html), [IAMF](https://aomedia.org/iamf/)
26. [WebVTT](https://www.w3.org/TR/webvtt1/), [Lhotse corpus model](https://lhotse.readthedocs.io/en/latest/corpus.html), [WhisperX](https://github.com/m-bain/whisperX)
27. [IETF mlcodec charter](https://datatracker.ietf.org/doc/charter-ietf-mlcodec/), [minutes, November 2025](https://datatracker.ietf.org/doc/minutes-124-mlcodec-202511062200/)
28. [AudioSeal](https://arxiv.org/html/2401.17264v2), [xHE-AAC at Audible, Fraunhofer 2025](https://www.audioblog.iis.fraunhofer.com/?p=8958)

Not reached while writing this: the Hydrogenaudio wiki, van Beurden's lossless comparison (revision 5), 3GPP TR
26.952, the ETSI text of LC3plus. Numbers that rest on them are marked [s2] or left out.
