# frozen_string_literal: true

module Zrip
  # FastCOVER-based Zstandard dictionary trainer.
  #
  # @!method self.new(max_dict_size)
  #   Create a dictionary trainer.
  #   @param max_dict_size [Integer] maximum dictionary size in bytes
  #   @return [DictTrainer]
  #
  # @!method self._native_new(max_dict_size)
  #   Native constructor used by `.new`.
  #   @param max_dict_size [Integer] maximum dictionary size in bytes
  #   @return [DictTrainer]
  #
  # @!method add_sample(bytes)
  #   Add a training sample. Samples shorter than 4 bytes are ignored.
  #   @param bytes [String] sample bytes
  #   @return [nil]
  #
  # @!method train
  #   Train and consume the trainer.
  #   @return [String] dictionary bytes
  #
  # @!method sample_count
  #   @return [Integer] accepted sample count
  #
  # @!method total_bytes
  #   @return [Integer] total bytes from accepted samples
  #
  # @!method trained?
  #   @return [Boolean]
  #
  # @!method max_dict_size
  #   @return [Integer] configured maximum dictionary size
  class DictTrainer
    def self.new(max_dict_size)
      _native_new(max_dict_size)
    end
  end
end
