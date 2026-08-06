# Changelog

## [Unreleased]

- Release the Ruby GVL around large frame/block compression and frame
  decompression calls.
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
