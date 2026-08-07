# frozen_string_literal: true

require_relative "dictionary"

module Zrip
  # Zstandard frame-format codec.
  #
  # `FrameCodec` is Ractor-shareable. It uses an internal lock around native
  # compression and decompression contexts.
  #
  # @!method self.new(dict: nil, level: DEFAULT_LEVEL)
  #   Create a frame codec.
  #   @param dict [Dictionary, String, nil] optional Zstandard dictionary
  #   @param level [Integer] compression level
  #   @return [FrameCodec]
  #
  # @!method self._native_new(dict, id, level)
  #   Native constructor used by `.new`.
  #   @param dict [String, nil] dictionary bytes
  #   @param id [Integer] dictionary ID, or `0` without a dictionary
  #   @param level [Integer] compression level
  #   @return [FrameCodec]
  #   @raise [CompressError]
  #
  # @!method self.get_frame_content_size(bytes)
  #   Read `Frame_Content_Size` from a Zstandard frame header.
  #   @param bytes [String] compressed Zstandard frame
  #   @return [Integer, nil]
  #   @raise [DecompressError]
  #
  # @!method compress(bytes)
  #   Compress bytes to a Zstandard frame.
  #   @param bytes [String] uncompressed bytes
  #   @return [String]
  #   @raise [CompressError]
  #
  # @!method decompress(bytes, max_output_size: nil)
  #   Decompress a Zstandard frame.
  #   @param bytes [String] compressed Zstandard frame
  #   @param max_output_size [Integer, nil] optional output byte limit
  #   @return [String]
  #   @raise [DecompressError]
  #   @raise [OutputSizeLimitError]
  #
  # @!method _native_decompress(bytes, max_output_size)
  #   Native decompression entry used by #decompress.
  #   @param bytes [String] compressed Zstandard frame
  #   @param max_output_size [Integer] output byte limit, or `0` for unbounded
  #   @return [String]
  #   @raise [DecompressError]
  #   @raise [OutputSizeLimitError]
  #
  # @!method has_dict?
  #   @return [Boolean]
  #
  # @!method id
  #   @return [Integer, nil] dictionary ID
  #
  # @!method size
  #   @return [Integer] dictionary size in bytes
  #
  # @!method level
  #   @return [Integer] compression level
  class FrameCodec
    def self.new(dict: nil, level: DEFAULT_LEVEL)
      case dict
      when nil
        _native_new(nil, 0, Integer(level))
      when Dictionary
        _native_new(dict.bytes, dict.id, Integer(level))
      when String
        d = Dictionary.new(bytes: dict)
        _native_new(d.bytes, d.id, Integer(level))
      else
        raise TypeError, "expected Zrip::Dictionary, String, or nil; got #{dict.class}"
      end
    end


    def decompress(bytes, max_output_size: nil)
      _native_decompress(bytes, Integer(max_output_size || 0))
    end
  end
end
