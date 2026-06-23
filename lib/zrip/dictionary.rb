# frozen_string_literal: true

require "digest"

module Zrip
  Dictionary = Data.define(:bytes, :id)

  class Dictionary
    ZDICT_MAGIC       = "\x37\xA4\x30\xEC".b.freeze
    USER_DICT_ID_MIN  = 32_768
    USER_DICT_ID_MAX  = (2**31) - 1
    USER_DICT_ID_SIZE = USER_DICT_ID_MAX - USER_DICT_ID_MIN + 1


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


    def size
      bytes.bytesize
    end
  end
end
