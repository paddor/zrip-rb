# frozen_string_literal: true

module Zrip
  # Picks a compression level per window of work: zrip's hill climb over the
  # levels of a range. Feed it each window's stats (see `FrameCodec#take_stats`)
  # and what limited the sender, and compress at the level it returns.
  #
  # Not Ractor-shareable: use one per connection.
  #
  # @!method level
  #   @return [Integer] the level to compress at
  class AdaptiveLevel
    BOTTLENECKS = { output: 0, cpu: 1, input: 2 }.freeze


    # @param range [Range<Integer>] levels to walk, within -8..4
    # @param start [Integer] start at the first level of the range not below
    #   it; -1 by default, as omq.rs
    # @raise [ArgumentError] if the range holds no zrip level
    def self.new(range, start: -1)
      unless Range === range && Integer === range.begin && Integer === range.end
        raise ArgumentError, "range must be a Range of Integers, got #{range.inspect}"
      end

      min = range.min
      max = range.max
      raise ArgumentError, "#{range.inspect} is empty" if min.nil?

      _native_new(min, max, Integer(start))
    end


    # Records one window at the current level and returns the next level.
    #
    # @param plain [Integer] input bytes compressed in the window
    # @param compressed [Integer] compressed bytes
    # @param busy [Float] seconds spent compressing
    # @param bottleneck [Symbol, nil] `:output` (writes stalled), `:cpu`
    #   (input waited while output kept up), `:input` (nothing waited), or nil
    #   (the default) when unclear; the level then holds
    # @return [Integer]
    # @raise [ArgumentError] on an unknown bottleneck
    def update(plain:, compressed:, busy:, bottleneck: nil)
      code = if bottleneck.nil?
        -1
      else
        BOTTLENECKS.fetch(bottleneck) do
          raise ArgumentError, "bottleneck must be :output, :cpu, :input or nil, got #{bottleneck.inspect}"
        end
      end

      _update(Integer(plain), Integer(compressed), (Float(busy) * 1e9).round, code)
    end

  end
end
