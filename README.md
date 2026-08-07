# zrip: Ractor-safe Zstandard for Ruby

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
- **Configurable compression levels** (default: 1)
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

# Limit total output size to 1 MiB
codec.decompress(compressed, max_output_size: 1024 * 1024)

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

## Documentation

Reference: <https://rubydoc.info/gems/zrip>

## License

[MIT](LICENSE)
