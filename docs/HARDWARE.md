# What tatum needs to run on

How much processor and memory a song asks for, measured, and what that means for
a Raspberry Pi or a microcontroller. None of this was run on the boards
themselves: the engine was compiled for each of them, the work each song asks
for was counted exactly, and the boards' capacity is an estimate stated with its
assumptions below. `cargo run --release -p tatum-core --example bench` measures
the real thing on the machine it runs on; a Pi in hand settles any row here.

The numbers assume #29 (delay lines sized to the song), which most of the
memory column depends on.

## What a song asks for

Two things decide whether a song plays on a machine: the memory its engine holds
once built, and the instructions per second its heaviest moment asks for. The
heaviest moment matters, not the average: a song is as playable as its worst
four seconds. Ten songs from `examples/`, lightest to heaviest:

| song | RAM | 64-bit ARM | 32-bit ARM (ARMv6) | Thumb-2, double FPU | Thumb-2, single FPU |
|---|---:|---:|---:|---:|---:|
| `arp_keys` | 343 KB | 207 M/s | 262 M/s | 277 M/s | 304 M/s |
| `live_set` | 429 KB | 294 | 342 | 359 | 427 |
| `techno_robot` | 525 KB | 354 | 411 | 429 | 526 |
| `acid_arp` | 465 KB | 406 | 465 | 483 | 569 |
| `detroit` | 754 KB | 588 | 665 | 693 | 779 |
| `dark_techno` | 1.6 MB | 696 | 827 | 861 | 983 |
| `hitech_psy` | 967 KB | 851 | 924 | 953 | 1155 |
| `shiva_circuit` | 2.5 MB | 886 | 963 | 991 | 1172 |
| `liquid_dnb` | 1.4 MB | 973 | 1044 | 1071 | 1224 |
| `warp_drive` | 1.5 MB | 979 | 1102 | 1139 | 1340 |

M/s is millions of instructions per second of audio, at 44.1 kHz. Built at
32 kHz (see "Building for another sample rate" in [CLI.md](CLI.md)) a song asks
for about 28% less; at 22.05 kHz, about half.

**How it was counted.** RAM is what the engine holds once built, under a
counting allocator. Instructions are exact counts of the engine playing, one
128-sample block at a time as an audio callback asks: valgrind on 64-bit ARM
Linux over each song's heaviest four seconds, and QEMU with its instruction
plugin over the first ten seconds for the 32-bit builds, scaled to the heaviest
four by each song's own ratio between the two. Counted directly at the heaviest
moment as a check, Thumb-2 came out within 2% of the scaled figure.

**What the columns stand for.** ARMv6 is the original Raspberry Pi Zero.
Thumb-2 with a double-precision FPU is a Cortex-M7 like the Teensy 4.1's or the
Daisy Seed's. Thumb-2 with only a single-precision FPU stands in for a
Cortex-M4F or M33, and for the ESP32s: there the engine's meters, which sum in
double precision, run in software, which is the 10–25% between the last two
columns.

## What a board can give

A board's capacity, in the same units, is its clock times the instructions it
completes per cycle on code like this, which is mostly single-precision floating
point in short dependent chains. That second number is an assumption, not a
measurement, and it is the soft part of this page:

| board | core | clock | assumed per cycle | M/s for audio | RAM |
|---|---|---:|---:|---:|---|
| Raspberry Pi 5 | Cortex-A76 | 2.4 GHz | 2–3 | 3000+ | 2–8 GB |
| Raspberry Pi 4 | Cortex-A72 | 1.5–1.8 GHz | 1.5–2 | 2000+ | 1–8 GB |
| Raspberry Pi Zero 2 W | Cortex-A53 | 1 GHz | 0.7–1.0 | 700–1000 | 512 MB |
| Raspberry Pi Zero / Zero W | ARM1176 | 1 GHz | 0.5–0.7 | 500–700 | 512 MB |
| Teensy 4.1 | Cortex-M7 | 600 MHz (816 overclocked) | 1.0–1.2 | 600–720 (800–980) | 1 MB, two 512 KB halves, + PSRAM |
| Daisy Seed | Cortex-M7 | 480 MHz | 1.0–1.2 | 480–580 | 64 MB SDRAM |
| ESP32-P4 | RISC-V, FPU | 400 MHz | ~1 | ~400 | 768 KB + PSRAM |
| ESP32-S3 | Xtensa LX7, FPU | 240 MHz | ~1 | ~240 | 512 KB + PSRAM |
| RP2350 (Pico 2) | Cortex-M33 | 150 MHz | ~1 | ~150 | 520 KB |

The engine plays on one core. A board with four keeps the other three for the
live screen, MIDI and the file watcher, which on a microcontroller do not exist.

Leave a third of a core free: an audio callback that takes all of its budget
sometimes takes more, and late is a click. So a song fits where its column is
under about 70% of the board's figure.

## What fits where

| board | at 44.1 kHz | at 32 kHz |
|---|---|---|
| Pi 4, Pi 5 | everything, with blends and the live screen | — |
| Pi Zero 2 W | the light songs; `detroit` and `dark_techno` at the edge; not the heavy ones | the medium songs; the heavy ones at the edge |
| Pi Zero (original) | `arp_keys`, `live_set`, little else | the light songs |
| Teensy 4.1 | `arp_keys`, `live_set`; `techno_robot` and `acid_arp` at the edge | the light songs; `detroit` at the edge, and `dark_techno` overclocked |
| Daisy Seed | `arp_keys`, `live_set` | the light songs |
| ESP32-P4 | none comfortably | `arp_keys` |
| ESP32-S3, RP2350 | none | none; a much smaller song, written for it |

Light here means up to about `acid_arp`; medium, `detroit` and `dark_techno`;
heavy, the rest of the table.

Two things double the work for a while and need the margin. A blend between two
steps of a set (`set play --blend`) plays both for its length. A step change
builds the next engine while the current one plays, and holds both in memory
until it lands.

## Memory

The four lightest songs fit in 512 KB: one half of a Teensy's RAM, or an
ESP32-S3 without PSRAM, though the S3 lacks the processor for them. The rest
need more, and on a microcontroller that means PSRAM, which is slower than
internal RAM. What takes it is mostly effects with a memory of their own: each
reverb, each delay line (sized to the longest echo the song can ask for), and a
`capture`, which records whole bars -- `dub_techno`'s four stereo bars are
2.8 MB of its 3.5.

Playing never allocates (#27): a song's memory is all taken when it is built, so
a heap does not fragment over a set.

## Flash

The engine with its parser and compiler, built for a Cortex-M7 with a song in
it: 412 KB optimised for speed, 285 KB for size. A `.synth` file is a few KB of
text, so songs can live on an SD card and be compiled on the board.

## What a board would still need

The engine compiles, unchanged, for every target in the tables
(`thumbv7em-none-eabihf`, `thumbv8m.main-none-eabihf`,
`riscv32imafc-unknown-none-elf`, `arm-unknown-linux-gnueabihf`,
`aarch64-unknown-linux-gnu`): it is `no_std` with `alloc` and has no
dependencies. A Raspberry Pi runs Linux and the CLI as it is; it needs a sound
card (an I2S DAC or a USB one) and nothing else. A microcontroller needs the rest
written: the audio output, USB MIDI, reading songs from storage, and a screen if
it has one.

## Measuring on a board

On a Pi, or any machine with Rust:

```
cargo run --release -p tatum-core --example bench -- examples/*.synth --seconds 300
```

For each song it prints the memory it holds, whether it allocated while playing
(it must not), how many times faster than real time it plays on average and over
its heaviest four seconds, and its median, 99th-percentile and worst block
against the 2.9 ms a block has. The heaviest four seconds is the number to read:
under about 1.5x the song is not safe to play live on that machine.
