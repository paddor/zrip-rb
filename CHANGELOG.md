# Changelog

## [Unreleased]

## [0.3.0] - 2026-11-26

- `FrameCodec#level=` and `BlockCodec#level=` switch later frames to a new
  level. A dictionary stays loaded; zrip prepares it again for the level.
- `FrameCodec#take_stats` and `BlockCodec#take_stats` return the bytes in
  and out and the time spent compressing since the last call. The time is
  measured natively around each compression, so it leaves out waits for the
  GVL and the codec lock.
- `Zrip::AdaptiveLevel.new(range, start: -1)` wraps zrip's adaptive level:
  `#update(plain:, compressed:, busy:, bottleneck: nil)` takes one window's
  stats and what limited the sender, and returns the next level.
- `FrameCodec#decompress` and `BlockCodec#decompress` decode into a new
  buffer per call. The codec kept a buffer the size of its largest frame
  for its lifetime, and copied each frame out of it.

## [0.2.2] - 2026-11-26

- Update the `zrip` crate dependency to 0.8.29. Levels are retuned, so
  compressed output differs from 0.2.1, and decoding rejects the frames C
  zstd 1.6.0 rejects.
- `DictTrainer#train` fills `max_dict_size`. Dictionaries came out 110-150
  bytes short. Sizes under 256 now count as 256, and `max_dict_size`
  returns the size used. They gave a dictionary of padding only.
- Let several threads compress or decompress one mutable String at once.
  Calls that release the GVL now borrow a frozen copy of the input instead
  of locking it, so they no longer raise `RuntimeError` when another call
  holds the same String, and the caller may modify it meanwhile.
- `max_output_size: 0` now limits output to 0 bytes. It used to disable the
  limit. Omit it or pass `nil` for no limit.
- Fail on a bad first frame header without releasing the GVL.
- Document that `BlockCodec` writes and reads Zstandard frames.

## [0.2.1] - 2026-10-11

- Update the `zrip` crate dependency to 0.8.11 and `rb-sys` to 0.9.130.
- Make `FrameCodec` instances Ractor-shareable as documented, with native
  dictionary copies that leave caller strings mutable.
- Unlock input strings and free native results when Ruby interrupts a call
  that released the GVL.
- Protect mutable inputs when mutex contention releases the GVL, including
  calls below the normal size threshold.
- Catch Rust panics inside native callbacks instead of aborting the process.
- Preserve Ruby exceptions, `Thread#kill`, and `catch`/`throw` through
  protected native calls.
- Avoid trainer deadlocks during sample coercion and borrow decompression
  input after size coercion.
- Enforce `max_output_size:` inside the native decoder across concatenated
  frames, before allocating or decoding past the limit.
- Validate complete frame headers when reading frame content sizes.
- Correct the README compression level example to the supported maximum.
- Update GitHub Actions checkout to v7 and check Rust tests, formatting, and
  Clippy in CI.

## [0.2.0] - 2026-08-07

- Release the Ruby GVL around large frame/block compression and frame
  decompression calls.
- Update the `zrip` crate dependency to 0.8.5.
- Replace Magnus with direct `rb-sys` bindings, add TruffleRuby build support,
  and lower the Ruby requirement to 3.4.
- Apply `max_output_size:` to the total decompressed output of concatenated
  Zstandard frames.
- Add RubyDoc metadata and move API reference details from `README.md` into
  YARD comments.

## [0.1.2] - 2026-08-05

- Update `zrip` crate dependency from 0.8.3 to 0.8.4.

## [0.1.1] - 2026-06-29

- Update `zrip` crate dependency from 0.3 to 0.8.

## [0.1.0] - 2026-06-20

- Initial release.
- `Zrip::FrameCodec`: frame-format Zstandard codec (Ractor-shareable).
- `Zrip::BlockCodec`: frame-format Zstandard codec, per-Ractor (no lock overhead).
- `Zrip::Dictionary`: immutable value type for Zstandard dictionaries.
- `Zrip::DictTrainer`: FastCOVER-based dictionary trainer.
- `Zrip::FrameCodec.get_frame_content_size`: reads FCS from frame header.
