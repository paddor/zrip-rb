# frozen_string_literal: true

require_relative "test_helper"

class TestNativeSoak < Minitest::Test
  ONE_KIB = 1024
  ONE_MIB = 1024 * 1024
  RSS_LIMIT_KB = Integer(ENV.fetch("SOAK_RSS_LIMIT_MB", "256")) * 1024
  THREADS = Integer(ENV.fetch("SOAK_THREADS", "4"))

  def test_codecs_survive_long_running_native_use
    deadline = now + Float(ENV.fetch("SOAK_SECONDS", "60"))
    payloads = [payload(ONE_KIB), payload(ONE_MIB)]
    frame = Zrip::FrameCodec.new(level: 1)
    block = Zrip::BlockCodec.new(level: 1)
    dict = build_dictionary
    frame_with_dict = Zrip::FrameCodec.new(dict: dict, level: 1)
    block_with_dict = Zrip::BlockCodec.new(dict: dict, level: 1)
    rss_floor = nil
    last_gc = now
    iterations = 0

    GC.start

    while now < deadline
      bytes = payloads[iterations % payloads.length]

      round_trip_frame(frame, bytes)
      round_trip_frame(frame_with_dict, bytes)
      round_trip_block(block, bytes)
      round_trip_block(block_with_dict, bytes)
      exercise_errors(frame, block, bytes)
      churn_native_objects(dict, iterations)
      thread_round_trips(frame, block, bytes) if (iterations % 25).zero?

      if now - last_gc >= 1.0
        GC.start
        rss_floor ||= rss_kb
        assert_operator rss_kb - rss_floor, :<, RSS_LIMIT_KB
        last_gc = now
      end

      iterations += 1
    end

    assert_operator iterations, :>, 0
    puts "soak_iterations=#{iterations} rss_kb=#{rss_kb}"
  end

  private

  def now
    Process.clock_gettime(Process::CLOCK_MONOTONIC)
  end

  def payload(size)
    base = 256.times.map do |i|
      "user=#{i}|region=eu-west-#{i % 4}|status=active|trace=#{format("%08x", i)}\n"
    end.join.b

    (base * ((size / base.bytesize) + 1)).byteslice(0, size).b
  end

  def build_dictionary
    trainer = Zrip::DictTrainer.new(8192)
    200.times do |i|
      trainer.add_sample("user_#{i}@example.com|status=active|tier=gold|region=eu-west-#{i % 4}")
    end
    Zrip::Dictionary.new(bytes: trainer.train)
  end

  def round_trip_frame(codec, bytes)
    compressed = codec.compress(bytes.dup)
    assert_equal bytes.bytesize, Zrip::FrameCodec.get_frame_content_size(compressed)
    assert_equal bytes, codec.decompress(compressed)
    assert_equal bytes, codec.decompress(compressed, max_output_size: bytes.bytesize)
  end

  def round_trip_block(codec, bytes)
    compressed = codec.compress(bytes.dup)
    assert_equal bytes, codec.decompress(compressed)
    assert_equal bytes, codec.decompress(compressed, max_output_size: bytes.bytesize)
  end

  def exercise_errors(frame, block, bytes)
    compressed = frame.compress(bytes)
    assert_raises(Zrip::OutputSizeLimitError) do
      frame.decompress(compressed, max_output_size: bytes.bytesize - 1)
    end

    assert_raises(Zrip::OutputSizeLimitError) do
      block.decompress(block.compress(bytes), max_output_size: bytes.bytesize - 1)
    end

    assert_raises(Zrip::DecompressError) { frame.decompress("not a zstd frame") }
    assert_raises(Zrip::DecompressError) { block.decompress("not a zstd block") }
    assert_raises(TypeError) { frame.compress(Object.new) }
  end

  def churn_native_objects(dict, iteration)
    10.times do |i|
      frame = Zrip::FrameCodec.new(dict: (i.even? ? dict : nil), level: 1)
      block = Zrip::BlockCodec.new(dict: (i.odd? ? dict : nil), level: 1)
      msg = "churn=#{iteration}-#{i}|status=active"
      assert_equal msg, frame.decompress(frame.compress(msg))
      assert_equal msg, block.decompress(block.compress(msg))
    end
  end

  def thread_round_trips(frame, block, bytes)
    threads = THREADS.times.map do |i|
      Thread.new do
        25.times do |j|
          msg = "#{i}-#{j}-".b + bytes
          assert_equal msg, frame.decompress(frame.compress(msg))
          assert_equal msg, block.decompress(block.compress(msg))
        end
      end
    end
    threads.each(&:value)
  end

  def rss_kb
    status = File.read("/proc/self/status")
    status[/^VmRSS:\s+(\d+)\s+kB$/, 1].to_i
  end
end
