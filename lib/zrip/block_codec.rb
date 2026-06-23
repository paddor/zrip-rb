# frozen_string_literal: true

require_relative "dictionary"

module Zrip
  class BlockCodec
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
