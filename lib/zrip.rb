# frozen_string_literal: true

require_relative "zrip/zrip"        # Rust extension
require_relative "zrip/version"

module Zrip
  # Default compression level used by `FrameCodec` and `BlockCodec`.
  DEFAULT_LEVEL = 1

  # @!parse
  #   # Raised when Zstandard decompression fails.
  #   class DecompressError < StandardError; end
  #
  #   # Raised when Zstandard compression fails.
  #   class CompressError < StandardError; end
  #
  #   # Reserved subclass for streams without a declared frame content size.
  #   class MissingContentSizeError < DecompressError; end
  #
  #   # Raised when decompressed output exceeds the requested limit.
  #   class OutputSizeLimitError < DecompressError; end
end

require_relative "zrip/dictionary"
require_relative "zrip/block_codec"
require_relative "zrip/frame_codec"
require_relative "zrip/dict_trainer"
