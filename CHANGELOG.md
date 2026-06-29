# Changelog

## [Unreleased]

## [0.1.1] - 2026-06-29

- Update `zrip` crate dependency from 0.3 to 0.8.

## [0.1.0] - 2026-06-20

- Initial release.
- `Zrip::FrameCodec`: frame-format Zstandard codec (Ractor-shareable).
- `Zrip::BlockCodec`: frame-format Zstandard codec, per-Ractor (no lock overhead).
- `Zrip::Dictionary`: immutable value type for Zstandard dictionaries.
- `Zrip::DictTrainer`: FastCOVER-based dictionary trainer.
- `Zrip::FrameCodec.get_frame_content_size`: reads FCS from frame header.
