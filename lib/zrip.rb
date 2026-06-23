# frozen_string_literal: true

require_relative "zrip/zrip"        # Rust extension
require_relative "zrip/version"

module Zrip
  DEFAULT_LEVEL = 3
end

require_relative "zrip/dictionary"
require_relative "zrip/block_codec"
require_relative "zrip/frame_codec"
require_relative "zrip/dict_trainer"
