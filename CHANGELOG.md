# Changelog

## [Unreleased]

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
