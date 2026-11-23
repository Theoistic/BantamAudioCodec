# Research: which codec to build on

State: 2026-11-21. The question was which existing codec is the best template for a voice format that is small,
streams, and carries its transcript, speakers and range tags. Short answer:

- **Coding: Opus**, more exactly its speech layer (SILK): linear prediction, pitch prediction, noise shaped under
  the voice, a range coder. Nothing free does voice better per bit without a neural network.
- **Framing and the lossless end: FLAC**: frames that stand alone, checksums, a seek table, integer arithmetic.
- **Entropy coder and unit structure: AV1.** AV1 is a video codec and codes no audio. Useful from it are the
  adaptive multi-symbol arithmetic coder and the stream of typed, length-prefixed units.
- **Labels: no audio format has them.** Speech corpora keep transcripts in files beside the audio. Tiers of timed
  events, as in Praat's TextGrid, are the model; BAC puts them inside the file.

## The field

| Family | Examples | 16 kHz voice | Good at | Why not the template |
|---|---|---:|---|---|
| Plain samples | WAV | 256 kbit/s | exact, trivial | size |
| Lossless | FLAC, ALAC, WavPack, Monkey's Audio, OptimFROG, MPEG-4 ALS | 135-150 | exact | The floor is the recording's own noise. The strongest (OptimFROG, Monkey's Audio) end up a few percent under FLAC at many times the work [1] |
| Transform, lossy | MP3, Vorbis, AAC | 32-64 | music | Built for music; voice needs 32 kbit/s and more. Vorbis lives in Ogg |
| Speech, lossy | **Opus** (RFC 6716), EVS, xHE-AAC | 12-32 | voice at low delay | Opus is the template. EVS and xHE-AAC are as good or better at the lowest rates and need patent licences |
| Parametric | Codec2, MELP, LPCNet | 0.7-3 | intelligible at almost nothing | does not sound like the speaker |
| Neural | SoundStream, EnCodec, DAC, Mimi, SNAC, WavTokenizer, BigCodec, FocalCodec | 0.5-6 | natural sound at 1 kbit/s [2][3] | see below |
| Classical + learned decoder | Opus 1.5 and 1.6 [4][5] | 6-16 | old bitstream, better sound | a later step, not a start |

**Neural codecs** are the new compression technology, and where the lowest rates are: BigCodec codes speech at
1.04 kbit/s with one codebook [2]; Mimi runs at 1.1 kbit/s with 8 codebooks and 550 bit/s with 4 [3]. They are not
the base of this format, for four reasons. The decoder is a network of tens of millions of weights, not a few
hundred lines of integer code. What it gives back is plausible, not the recording: content errors grow as the rate
falls [6], and a 2026 benchmark on African speech found codecs of similar rate far apart [7]. Two decoders do not
give the same samples. And everything trained on such audio is capped by that codec. Our own codec at 550 bit/s
showed the same. Their place here is a track of tokens beside the audio (Format.md, section 6), read by a language
model without decoding any sound.

**Opus** since 1.5 goes the other way: the bitstream stays, and the decoder gets learned parts: loss concealment,
redundancy against lost packets, enhancement of low-rate speech (LACE, NoLACE [5]), and in 1.6 bandwidth extension
from 16 kHz to 48 kHz [4]. For BAC that is the path after this one: the same files, a better decoder.

**AV1** has no audio. Video in AV1 usually travels with Opus; AOMedia's own audio specification, IAMF, is a container for
immersive sound that holds Opus, AAC, FLAC or plain samples [8]. What AV1 gives is its entropy coder [9]: symbols of
up to 16 values coded directly under tables that adapt after every symbol, with no tables in the stream.

**Containers.** Ogg is made for streaming (pages, a time stamp per page, a checksum); timed text in it needs extra
streams that few tools read. Matroska has subtitle tracks, cues and tags, and is large. FLAC has metadata blocks, none
timed. WAV has cue and label chunks few programs keep. So the labels became first-class units of our own stream.

## What BAC takes from where

| From | Taken | Changed |
|---|---|---|
| Opus / SILK | linear prediction + pitch prediction + noise feedback, a range coder, steps per 5 ms | plain scalar steps, down to step 1 = lossless; reflection coefficients on a grid, not vector-quantised line spectra |
| FLAC | integer predictor with a shift, exact reconstruction, checksums, seek table | one predictor per 20 ms carried over between frames; no Rice codes |
| AV1 | adaptive multi-symbol tables, typed units, key frames | 32-bit range coder with carry; the tables reset at key packets |
| WavPack hybrid [10] | one structure from lossy to lossless, noise shaping in the time domain | one file, not a lossy file plus a correction file |
| TextGrid, WebVTT | tiers of timed events | inside the audio file, with speakers |

Why not simply Opus packets in our container: one bitstream covers 7 kbit/s to lossless, the decoder is exact
integer arithmetic (the same samples on every machine, which a training set wants), and it adds no dependency. The
container does not care; a later HEAD profile can carry another codec.

## Training on lossy audio

- Speech recognition: GigaSpeech stores its 10,000 hours as Opus at 32 kbit/s and measured 0.1 to 0.2 points of
  word error rate against uncompressed audio [11]. A far-field study lost 12.6 % relative at 16 kbit/s per channel,
  6.6 % at 32 and 2.4 % at 128 [12].
- Speech synthesis: no study found. A model that learns to produce sound also learns the codec's noise, so audio
  that a synthesiser is trained to produce should stay lossless or at a high quality until we have measured it.

## Measured

`tools/compare_codecs.py`: every codec on the same mono clips, ffmpeg 8.1 for the others (libopus with
`-application voip`, FLAC at compression level 12). Quality numbers are from `bac compare` (metrics.rs): SNR,
noise-to-mask ratio (NMR: the error against a rough masking threshold; lower is better, below 0 mostly hidden) and
band distance (how far the energy per critical band is off; lower is better).

LibriSpeech test-clean, 40 clips spread over the speakers, 344 s, 16 kHz:

| Codec | kbit/s | MB/hour | SNR dB | NMR dB | Band dB |
|---|---:|---:|---:|---:|---:|
| WAV | 256.0 | 115.2 | exact | | |
| FLAC, level 12 | 147.2 | 66.3 | exact | | |
| **BAC lossless** | **133.5** | **60.1** | exact | | |
| Opus 16k | 16.0 | 7.2 | 7.9 | 5.6 | 2.97 |
| **BAC -q 3** | 15.7 | 7.1 | 12.5 | 7.1 | 3.06 |
| Opus 24k | 23.8 | 10.7 | 8.6 | 4.1 | 2.80 |
| **BAC -q 4** | 21.9 | 9.9 | 17.1 | 5.3 | 2.44 |
| Opus 32k | 31.6 | 14.2 | 8.9 | 3.0 | 2.62 |
| MP3 32k | 32.5 | 14.6 | 17.8 | 2.6 | 3.27 |
| Vorbis 32k | 31.0 | 14.0 | 16.5 | 2.9 | 1.65 |
| **BAC -q 5** | 28.4 | 12.8 | 21.5 | 3.0 | 1.86 |
| **BAC -q 6** | 35.1 | 15.8 | 25.4 | 0.7 | 1.45 |
| MP3 48k | 48.7 | 21.9 | 21.9 | -0.7 | 2.79 |
| **BAC -q 8** | 48.6 | 21.9 | 31.9 | -4.4 | 0.78 |

The jax voice set (Bantam-Voice, 60 clips, 323 s), brought to 16 kHz mono as the training set stores it:

| Codec | kbit/s | MB/hour | SNR dB | NMR dB | Band dB |
|---|---:|---:|---:|---:|---:|
| FLAC, level 12 | 155.8 | 70.1 | exact | | |
| **BAC lossless** | **135.8** | **61.1** | exact | | |
| Opus 16k | 16.0 | 7.2 | 9.1 | 4.7 | 2.10 |
| **BAC -q 3** | 16.4 | 7.4 | 13.9 | 7.6 | 3.45 |
| Opus 24k | 23.3 | 10.5 | 9.5 | 2.9 | 1.83 |
| **BAC -q 4** | 22.8 | 10.3 | 18.6 | 5.2 | 2.61 |
| Opus 32k | 30.7 | 13.8 | 9.6 | 1.8 | 1.63 |
| **BAC -q 5** | 29.4 | 13.2 | 22.9 | 2.7 | 1.90 |
| **BAC -q 6** | 36.3 | 16.3 | 27.0 | 0.1 | 1.39 |

What this says:

- Lossless: 9 % under the best FLAC on LibriSpeech, 13 % on the jax voice.
- Lossy, default quality 4: about 10 MB per hour, a seventh of FLAC.
- Against Opus on these numbers: level at about 30 kbit/s on LibriSpeech, a little behind there on the jax voice,
  and clearly behind below: on the jax voice Opus at 16 kbit/s measures like BAC at about 24. BAC keeps the
  waveform much closer at every rate (SNR), which is what its structure is good at.
- What it does not say: how it sounds. Nobody has listened yet. The numbers compare waveforms, and Opus is tuned
  by ear to sound better than its waveform error; the true distance at low rates may be larger.

Where the bits go at quality 4 (LibriSpeech, 21.9 kbit/s; `bac bench --bits`): samples 18.5, predictor 1.5, pitch
0.9, steps and flags 0.7, container 0.3. Of the frames 56 % are voiced, 18 % are coded as background, 44 % keep the
predictor of the frame before. Speed on one core at 16 kHz: encoding 200 times real time, decoding 1,700 times.

## Tuning record

Settings of the reference encoder that were decided by sweeps over 30 LibriSpeech clips, read off at equal bitrate:

- Noise shaping `gamma` 0.85 at coarse steps down to 0.6 at fine ones. Shaping the noise fully like the voice
  (1.0) cost 4 to 9 dB of SNR and gained nothing in NMR.
- Rounding dead zone 0.35 (coarse) to 0.15 (fine): at 12 kbit/s a dead zone of 0.33 measured 1.5 dB better in NMR
  than one of 0.12.
- The pitch is searched below 1 kHz; searched in the whole band only 18 % of frames came out voiced, now 54 %.
- Long-term prediction pays for its side information and little more. It is the weakest tool so far.
- Noise in place of zeros was first tried on all unvoiced frames and was worse on every measure at equal rate. It
  is now used only on the background between words, with its level matched to the step: 2 to 6 kbit/s saved.
  The measures count replaced noise as error by construction, so this one is decided by principle, not by number.
- A key packet every second costs 0.2 kbit/s against one every two seconds; packets of 0.2 s in place of 1 s cost
  0.3 kbit/s.

## Next, in the order of what would tell us most

1. Listen: `samples/` has one conversation at five qualities beside the original.
2. Word error rate of the hearing model on decoded audio, and a synthesis check, per quality.
3. Encoder, same format: noise shaped between the pitch harmonics, a better use of long-term prediction, a
   rate-distortion choice per sample in place of the dead zone, starting tables learned from speech.
4. Format, before version 1: a token track, two channels coded jointly, a learned enhancer in the decoder.

## Sources

1. [FLAC comparison of lossless codecs](https://xiph.org/flac/comparison.pdf)
2. [BigCodec: Pushing the Limits of Low-Bitrate Neural Speech Codec](https://arxiv.org/abs/2409.05377)
3. [LILAC: An Idempotent Neural Speech Codec](https://arxiv.org/abs/2608.05727) (Mimi and BigCodec side by side at 0.5 kbit/s)
4. [Opus 1.6](https://opus-codec.org/demo/opus-1.6), [release notes](https://opus-codec.org/release/stable/2025/12/15/libopus-1_6.html)
5. [NoLACE: Improving Low-Complexity Speech Codec Enhancement](https://arxiv.org/abs/2309.14521)
6. [Codec-SUPERB: An In-Depth Analysis of Sound Codec Models](https://arxiv.org/abs/2402.13071)
7. [How Robust Are Neural Audio Codecs for African Speech?](https://arxiv.org/abs/2610.00154)
8. [AOMedia: What is IAMF?](https://aomedia.org/iamf/)
9. [A Technical Overview of AV1](https://arxiv.org/abs/2008.06091)
10. [WavPack: hybrid lossless audio compression](https://wavpack.com/)
11. [GigaSpeech: An Evolving, Multi-domain ASR Corpus](https://arxiv.org/abs/2106.06909)
12. [Multi-channel Acoustic Modeling using Mixed Bitrate OPUS Compression](https://arxiv.org/abs/2002.00122)

Also: RFC 6716 (Opus), RFC 9639 (FLAC), and [Improving Opus Low Bit Rate Quality with Neural Speech
Synthesis](https://arxiv.org/abs/1905.04628) for SILK's noise feedback structure.
