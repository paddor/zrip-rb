# zrip — Ractor-safe Zstandard for Ruby

[![CI](https://github.com/paddor/zrip-rb/actions/workflows/ci.yml/badge.svg)](https://github.com/paddor/zrip-rb/actions/workflows/ci.yml)
[![Gem Version](https://img.shields.io/gem/v/zrip?color=e9573f)](https://rubygems.org/gems/zrip)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Ruby](https://img.shields.io/badge/Ruby-%3E%3D%204.0-CC342D?logo=ruby&logoColor=white)](https://www.ruby-lang.org)

Ruby bindings for [zrip](https://crates.io/crates/zrip), a pure-Rust Zstandard
implementation. Built with [magnus](https://github.com/matsadler/magnus) and
declared Ractor-safe so you can compress from any Ractor without a global lock.

## Features

- **Frame codec** for standard Zstd frames (Ractor-shareable)
- **Block codec** with per-Ractor context (no lock overhead)
- **Dictionary support** for both frame and block codecs
- **FastCOVER-based dictionary trainer** (`DictTrainer`)
- **Configurable compression levels** (default: 3)
- **Bounded decompression** with `max_output_size:` and frame content size checks
- **Ractor-safe**: `FrameCodec` is shareable across Ractors, `BlockCodec` is
  per-Ractor (mutable context state)

## Install

Requires Ruby >= 4.0 and a Rust toolchain (for building the native extension):

```sh
gem install zrip
```

Or in your Gemfile:

```ruby
gem "zrip"
```

## Usage

### Frame codec (standard Zstd frames)

```ruby
require "zrip"

codec = Zrip::FrameCodec.new
compressed = codec.compress("hello world " * 1000)
original   = codec.decompress(compressed)
```

### Block codec

```ruby
codec = Zrip::BlockCodec.new
compressed = codec.compress("hello world " * 1000)
original   = codec.decompress(compressed)
```

### Compression levels

```ruby
fast   = Zrip::FrameCodec.new(level: -3)   # negative = faster
strong = Zrip::FrameCodec.new(level: 19)    # higher = smaller output
```

### Bounded decompression

```ruby
codec = Zrip::FrameCodec.new

# Limit output size to 1 MiB
codec.decompress(compressed, max_output_size: 1_048_576)

# Read frame content size from header (without decompressing)
Zrip::FrameCodec.get_frame_content_size(compressed)  #=> 12000
```

### Dictionary compression

```ruby
dict = Zrip::Dictionary.new(bytes: trained_dict_bytes)
codec = Zrip::FrameCodec.new(dict: dict)

compressed = codec.compress("common log prefix: event=login user=alice")
original   = codec.decompress(compressed)
```

### Dictionary training

```ruby
trainer = Zrip::DictTrainer.new(8192)
messages.each { |msg| trainer.add_sample(msg) }
dict_bytes = trainer.train

dict  = Zrip::Dictionary.new(bytes: dict_bytes)
codec = Zrip::FrameCodec.new(dict: dict)
```

### Ractor safety

```ruby
codec = Zrip::FrameCodec.new

ractors = 4.times.map do |i|
  Ractor.new(codec) do |c|
    data = "ractor #{Ractor.current} payload " * 100
    ct   = c.compress(data)
    raise "mismatch" unless c.decompress(ct) == data
    :ok
  end
end

ractors.each { |r| p r.value }  # => :ok, :ok, :ok, :ok
```

## API

| Class / Module | Method | Description |
|---|---|---|
| `Zrip::FrameCodec` | `.new(dict: nil, level: 3)` | Create a frame codec, optionally with a `Dictionary`, raw `String` dict, or compression level |
| | `.get_frame_content_size(string)` | Read Frame_Content_Size from a Zstd frame header |
| | `#compress(string)` | Compress to Zstd frame |
| | `#decompress(string, max_output_size: nil)` | Decompress a Zstd frame, optionally bounded |
| | `#has_dict?` | Whether a dictionary is loaded |
| | `#id` | Dictionary ID (nil without dict) |
| | `#size` | Dictionary size in bytes (0 without dict) |
| | `#level` | Compression level |
| `Zrip::BlockCodec` | `.new(dict: nil, level: 3)` | Create a block codec, optionally with a dict |
| | `#compress(string)` | Compress to Zstd block |
| | `#decompress(string, max_output_size: nil)` | Decompress a Zstd block, optionally bounded |
| | `#has_dict?` | Whether a dictionary is loaded |
| | `#size` | Dictionary size in bytes (0 without dict) |
| | `#level` | Compression level |
| `Zrip::Dictionary` | `.new(bytes:, id: nil)` | Immutable dictionary value object (`Data.define`) |
| | `#bytes` | Frozen binary dict bytes |
| | `#id` | 32-bit dictionary ID (auto-detected from ZDICT header or SHA-256) |
| | `#size` | Dictionary size in bytes |
| `Zrip::DictTrainer` | `.new(max_dict_size)` | Create a trainer |
| | `#add_sample(string)` | Feed a training sample (skips < 4 bytes) |
| | `#train` | Consume the trainer, return dict bytes |
| | `#sample_count` | Number of accepted samples |
| | `#total_bytes` | Total bytes of accepted samples |
| | `#trained?` | Whether `#train` has been called |
| | `#max_dict_size` | Configured max dict size |
| `Zrip::DecompressError` | | Raised on decompression failure |
| `Zrip::CompressError` | | Raised on compression failure |
| `Zrip::MissingContentSizeError` | | Raised when Frame_Content_Size is absent (subclass of `DecompressError`) |
| `Zrip::OutputSizeLimitError` | | Raised when declared content size exceeds limit (subclass of `DecompressError`) |

## License

[MIT](LICENSE)
