# frozen_string_literal: true

require "digest"

module Zrip
  # Immutable Zstandard dictionary value.
  #
  # @!attribute [r] bytes
  #   @return [String] frozen binary dictionary bytes
  #
  # @!attribute [r] id
  #   @return [Integer] dictionary ID
  Dictionary = Data.define(:bytes, :id)

  class Dictionary
    ZDICT_MAGIC       = "\x37\xA4\x30\xEC".b.freeze
    USER_DICT_ID_MIN  = 32_768
    USER_DICT_ID_MAX  = (2**31) - 1
    USER_DICT_ID_SIZE = USER_DICT_ID_MAX - USER_DICT_ID_MIN + 1


    # @param bytes [String] dictionary bytes
    # @param id [Integer, nil] optional dictionary ID
    def initialize(bytes:, id: nil)
      b = bytes.b
      id ||= if b.byteslice(0, 4) == ZDICT_MAGIC
               b.byteslice(4, 4).unpack1("V")
             else
               raw = Digest::SHA256.digest(b).byteslice(0, 4).unpack1("V")
               USER_DICT_ID_MIN + (raw % USER_DICT_ID_SIZE)
             end
      super(bytes: b.freeze, id: id)
    end


    # @return [Integer] dictionary size in bytes
    def size
      bytes.bytesize
    end
  end
end
