# frozen_string_literal: true

require_relative "test_helper"
require "objspace"
require "digest"

describe Zrip do
  it "version is a non empty string" do
    assert_instance_of String, Zrip::VERSION
    refute_empty Zrip::VERSION
  end
end


describe Zrip::FrameCodec, "without a dictionary" do
  before do
    @codec = Zrip::FrameCodec.new
  end


  it "round trips empty string" do
    ct = @codec.compress("")
    assert_equal "", @codec.decompress(ct)
  end


  it "round trips single byte" do
    ct = @codec.compress("x")
    assert_equal "x", @codec.decompress(ct)
  end


  it "round trips ascii text" do
    pt = "the quick brown fox jumps over the lazy dog"
    assert_equal pt, @codec.decompress(@codec.compress(pt))
  end


  it "round trips repetitive input and compresses" do
    pt = "A" * 100_000
    ct = @codec.compress(pt)
    assert_operator ct.bytesize, :<, pt.bytesize / 10
    assert_equal pt, @codec.decompress(ct)
  end


  it "round trips random bytes 1mib" do
    pt = Random.bytes(1024 * 1024)
    assert_equal pt, @codec.decompress(@codec.compress(pt))
  end


  it "round trips binary data with nul bytes" do
    pt = (0..255).map(&:chr).join * 16
    pt.force_encoding(Encoding::ASCII_8BIT)
    assert_equal pt, @codec.decompress(@codec.compress(pt))
  end


  it "decompresses concatenated frames when later frame is larger" do
    ct = @codec.compress("A") + @codec.compress("World!")
    assert_equal "AWorld!", @codec.decompress(ct)
  end


  it "max output size applies to concatenated output" do
    ct = @codec.compress("a" * 100) + @codec.compress("b" * 100)
    assert_raises(Zrip::OutputSizeLimitError) do
      @codec.decompress(ct, max_output_size: 199)
    end
    assert_equal 200, @codec.decompress(ct, max_output_size: 200).bytesize
  end


  it "emits zstd frame magic" do
    ct = @codec.compress("anything")
    assert_equal [0x28, 0xB5, 0x2F, 0xFD], ct.bytes.first(4)
  end


  it "accepts explicit compression level" do
    c1 = Zrip::FrameCodec.new(level: -3)
    c3 = Zrip::FrameCodec.new(level: 4)
    pt = ("X" * 100_000).b
    refute_equal c1.compress(pt), c3.compress(pt)
    assert_equal pt, c1.decompress(c1.compress(pt))
    assert_equal pt, c3.decompress(c3.compress(pt))
    assert_equal(-3, c1.level)
    assert_equal 4, c3.level
  end


  it "compress returns binary encoding" do
    ct = @codec.compress("hello")
    assert_equal Encoding::ASCII_8BIT, ct.encoding
  end


  it "decompress returns binary encoding" do
    pt = @codec.decompress(@codec.compress("hello"))
    assert_equal Encoding::ASCII_8BIT, pt.encoding
  end


  it "raises decompress error on garbage" do
    assert_raises(Zrip::DecompressError) { @codec.decompress("not a valid zstd frame") }
  end


  it "raises decompress error on empty input" do
    assert_raises(Zrip::DecompressError) { @codec.decompress("") }
  end


  it "decompress error is standard error subclass" do
    assert_includes Zrip::DecompressError.ancestors, StandardError
  end


  it "compress error is standard error subclass" do
    assert_includes Zrip::CompressError.ancestors, StandardError
  end


  it "has dict false and id nil" do
    refute_predicate @codec, :has_dict?
    assert_nil @codec.id
    assert_equal 0, @codec.size
  end
end


describe "Zrip::FrameCodec.get_frame_content_size" do
  before do
    @codec = Zrip::FrameCodec.new
  end


  it "returns declared content size" do
    pt = "X" * 12_345
    ct = @codec.compress(pt)
    assert_equal pt.bytesize, Zrip::FrameCodec.get_frame_content_size(ct)
  end


  it "raises decompress error on garbage" do
    assert_raises(Zrip::DecompressError) do
      Zrip::FrameCodec.get_frame_content_size("not a valid zstd frame")
    end
  end


  it "raises decompress error on empty input" do
    assert_raises(Zrip::DecompressError) do
      Zrip::FrameCodec.get_frame_content_size("")
    end
  end


  it "rejects truncated header without content size" do
    header = "\x28\xB5\x2F\xFD\x00".b
    assert_raises(Zrip::DecompressError) { Zrip::FrameCodec.get_frame_content_size(header) }
    assert_nil Zrip::FrameCodec.get_frame_content_size(header + "\x00".b)
  end


  it "rejects reserved descriptor bit" do
    compressed = @codec.compress("payload")
    compressed.setbyte(4, compressed.getbyte(4) | 0x08)
    assert_raises(Zrip::DecompressError) { Zrip::FrameCodec.get_frame_content_size(compressed) }
  end


  it "ignores unused descriptor bit" do
    compressed = @codec.compress("payload")
    compressed.setbyte(4, compressed.getbyte(4) | 0x10)
    assert_equal 7, Zrip::FrameCodec.get_frame_content_size(compressed)
  end
end


describe "Zrip decompression limits" do
  it "rejects large declared size before decoding" do
    header = "\x28\xB5\x2F\xFD\xE0".b + [2**32].pack("Q<")
    [Zrip::FrameCodec.new, Zrip::BlockCodec.new].each do |codec|
      assert_raises(Zrip::OutputSizeLimitError) do
        codec.decompress(header, max_output_size: 1024)
      end
      assert_equal "usable", codec.decompress(codec.compress("usable"))
    end
  end


  it "bounds frames without content size" do
    payload = "payload".b * 100
    header = "\x28\xB5\x2F\xFD\x00\x00".b
    block_header = [(payload.bytesize << 3) | 1].pack("V").byteslice(0, 3)
    compressed = header + block_header + payload

    [Zrip::FrameCodec.new, Zrip::BlockCodec.new].each do |codec|
      assert_nil Zrip::FrameCodec.get_frame_content_size(compressed)
      assert_raises(Zrip::OutputSizeLimitError) do
        codec.decompress(compressed, max_output_size: payload.bytesize - 1)
      end
      assert_equal payload, codec.decompress(compressed, max_output_size: payload.bytesize)
    end
  end


  it "no large output string on failed decompress" do
    codec   = Zrip::FrameCodec.new
    size    = 1024 * 1024
    garbage = "\x00".b * size

    GC.start
    before = ObjectSpace.each_object(String).count { |s| s.bytesize >= size }

    10.times do
      assert_raises(Zrip::DecompressError) { codec.decompress(garbage) }
    end

    GC.start
    after = ObjectSpace.each_object(String).count { |s| s.bytesize >= size }

    assert_equal before, after,
      "failed decompress should not leak large output strings"
  end
end


describe Zrip::Dictionary do
  before do
    @bytes = "header version=1 type=message field1=" * 4
  end


  it "stores bytes binary encoded and frozen" do
    d = Zrip::Dictionary.new(bytes: @bytes)
    assert_equal @bytes.b, d.bytes
    assert_predicate d.bytes, :frozen?
    assert_equal Encoding::ASCII_8BIT, d.bytes.encoding
  end


  it "defaults id to sha256 mapped into public range" do
    raw = Digest::SHA256.digest(@bytes)[0, 4].unpack1("V")
    expected = Zrip::Dictionary::USER_DICT_ID_MIN +
      (raw % Zrip::Dictionary::USER_DICT_ID_SIZE)
    assert_equal expected, Zrip::Dictionary.new(bytes: @bytes).id
  end


  it "auto generated ids stay within public range" do
    range = Zrip::Dictionary::USER_DICT_ID_MIN..Zrip::Dictionary::USER_DICT_ID_MAX
    200.times do
      dd = Zrip::Dictionary.new(bytes: Random.bytes(128))
      assert_includes range, dd.id
    end
  end


  it "accepts caller supplied id" do
    d = Zrip::Dictionary.new(bytes: @bytes, id: 0xDEAD_BEEF)
    assert_equal 0xDEAD_BEEF, d.id
  end


  it "size reports dict size in bytes" do
    assert_equal @bytes.bytesize, Zrip::Dictionary.new(bytes: @bytes).size
  end


  it "immutability and value equality" do
    assert_predicate Zrip::Dictionary.new(bytes: @bytes), :frozen?
    assert_equal Zrip::Dictionary.new(bytes: @bytes), Zrip::Dictionary.new(bytes: @bytes.dup)
  end


  it "shareable across ractors" do
    require_ractor

    r = Ractor.new(Zrip::Dictionary.new(bytes: @bytes)) { |d| [d.bytes, d.id] }
    got_bytes, got_id = ractor_value(r)
    assert_equal @bytes.b, got_bytes
    assert_kind_of Integer, got_id
  end
end


describe Zrip::BlockCodec, "without a dictionary" do
  before do
    @codec = Zrip::BlockCodec.new
  end


  it "has dict is false" do
    refute_predicate @codec, :has_dict?
  end


  it "round trips empty string" do
    ct = @codec.compress("")
    assert_equal "", @codec.decompress(ct)
  end


  it "round trips ascii text" do
    pt = "the quick brown fox jumps over the lazy dog"
    ct = @codec.compress(pt)
    assert_equal pt, @codec.decompress(ct)
  end


  it "round trips repetitive input and compresses" do
    pt = "A" * 100_000
    ct = @codec.compress(pt)
    assert_operator ct.bytesize, :<, pt.bytesize / 10
    assert_equal pt, @codec.decompress(ct)
  end


  it "round trips across size buckets" do
    [0, 1, 64, 255, 256, 1024, 4096, 65_536, 1024 * 1024].each do |n|
      pt = Random.bytes(n)
      ct = @codec.compress(pt)
      assert_equal pt, @codec.decompress(ct),
        "round-trip failed at size #{n}"
    end
  end


  it "emits binary encoded output" do
    ct = @codec.compress("hello")
    assert_equal Encoding::ASCII_8BIT, ct.encoding
  end


  it "reuses context across many calls" do
    500.times do |i|
      pt = "message #{i} " * (1 + i % 10)
      ct = @codec.compress(pt)
      assert_equal pt, @codec.decompress(ct)
    end
  end


  it "default level" do
    assert_equal Zrip::DEFAULT_LEVEL, @codec.level
  end


  it "size is zero without dict" do
    assert_equal 0, @codec.size
  end
end


describe Zrip::DictTrainer do
  def json_msg(i)
    %Q({"ts":"2026-04-27T12:00:00.#{format("%04d", i)}Z","level":"INFO","service":"api-gw","trace":"#{format("%08x", i)}","method":"GET","path":"/v1/users/#{format("%04d", i)}","status":200,"latency_ms":#{10 + i % 490},"region":"us-east-1"})
  end


  it "starts with zero samples" do
    t = Zrip::DictTrainer.new(2048)
    assert_equal 0, t.sample_count
    assert_equal 0, t.total_bytes
    refute_predicate t, :trained?
  end


  it "sample coercion can call back into trainer" do
    trainer = Zrip::DictTrainer.new(2048)
    sample = Object.new
    sample.define_singleton_method(:to_str) do
      trainer.sample_count
      "training sample"
    end

    assert_nil trainer.add_sample(sample)
    assert_equal 1, trainer.sample_count
    assert_equal 15, trainer.total_bytes
  end


  it "sample coercion preserves throw" do
    trainer = Zrip::DictTrainer.new(2048)
    sample = Object.new
    sample.define_singleton_method(:to_str) { throw :sample, :escaped }

    assert_equal :escaped, catch(:sample) { trainer.add_sample(sample) }
    assert_equal 0, trainer.sample_count
    assert_nil trainer.add_sample("usable sample")
  end


  it "sample coercion can consume trainer" do
    trainer = Zrip::DictTrainer.new(2048)
    sample = Object.new
    sample.define_singleton_method(:to_str) do
      trainer.train
      "training sample"
    end

    assert_raises(RuntimeError) { trainer.add_sample(sample) }
    assert_predicate trainer, :trained?
  end


  it "trains nonempty dict from 100 json samples" do
    t = Zrip::DictTrainer.new(8192)
    100.times { |i| t.add_sample(json_msg(i)) }
    assert_operator t.sample_count, :>, 0
    assert_operator t.total_bytes, :>, 0

    dict = t.train
    assert_predicate t, :trained?
    refute_empty dict
    assert_equal Encoding::ASCII_8BIT, dict.encoding
  end


  it "trained dict is zdict format" do
    t = Zrip::DictTrainer.new(8192)
    200.times { |i| t.add_sample(json_msg(i)) }
    dict = t.train
    assert_equal Zrip::Dictionary::ZDICT_MAGIC, dict.byteslice(0, 4)
    header_id = dict.byteslice(4, 4).unpack1("V")
    refute_equal 0, header_id
  end


  it "trained dict improves frame codec compression" do
    t = Zrip::DictTrainer.new(8192)
    200.times { |i| t.add_sample(json_msg(i)) }
    dict_bytes = t.train
    d = Zrip::Dictionary.new(bytes: dict_bytes)

    codec      = Zrip::FrameCodec.new(dict: d)
    no_dict    = Zrip::FrameCodec.new
    msg        = json_msg(9999)
    ct_with    = codec.compress(msg)
    ct_without = no_dict.compress(msg)
    assert_operator ct_with.bytesize, :<, ct_without.bytesize
    assert_equal msg, codec.decompress(ct_with)
  end


  it "returns empty dict with fewer than 2 samples" do
    t = Zrip::DictTrainer.new(2048)
    t.add_sample("hello world")
    assert_equal "", t.train
  end


  it "skips samples shorter than 4 bytes" do
    t = Zrip::DictTrainer.new(2048)
    t.add_sample("hi")
    assert_equal 0, t.sample_count
  end


  it "raises runtime error on double train" do
    t = Zrip::DictTrainer.new(2048)
    10.times { |i| t.add_sample(json_msg(i)) }
    t.train
    assert_raises(RuntimeError) { t.train }
  end


  it "raises runtime error on add sample after train" do
    t = Zrip::DictTrainer.new(2048)
    t.add_sample("hello world")
    t.train
    assert_raises(RuntimeError) { t.add_sample("more data") }
  end


  it "raises runtime error on sample count after train" do
    t = Zrip::DictTrainer.new(2048)
    t.train
    assert_raises(RuntimeError) { t.sample_count }
  end


  it "raises runtime error on total bytes after train" do
    t = Zrip::DictTrainer.new(2048)
    t.train
    assert_raises(RuntimeError) { t.total_bytes }
  end


  it "cannot cross ractor boundaries" do
    require_ractor

    t = Zrip::DictTrainer.new(2048)
    assert_raises(TypeError, Ractor::IsolationError) do
      Ractor.new(t) { |tr| tr.add_sample("data") }
    end
  end
end


describe Zrip::FrameCodec, "with a dictionary" do
  before do
    t = Zrip::DictTrainer.new(8192)
    200.times do |i|
      t.add_sample("user_#{i}@example.com|status=active|tier=gold|region=eu-west-#{i % 4}")
    end
    dict_bytes = t.train
    @dict = Zrip::Dictionary.new(bytes: dict_bytes)
    @codec = Zrip::FrameCodec.new(dict: @dict)
    @no_dict = Zrip::FrameCodec.new
  end


  it "frame codec with dict is ractor shareable" do
    require_ractor
    bytes = @dict.bytes.dup
    codec = Zrip::FrameCodec.new(dict: bytes)
    refute_predicate bytes, :frozen?
    bytes.replace("changed")
    assert Ractor.shareable?(codec)
    r = Ractor.new(codec) do |shared|
      input = "shared dictionary payload " * 100
      shared.decompress(shared.compress(input)) == input
    end
    assert_equal true, ractor_value(r)
  end


  it "uses dicts cached id" do
    assert_equal @dict.id, @codec.id
    assert_predicate @codec, :has_dict?
  end


  it "raises type error for bad dict" do
    assert_raises(TypeError) { Zrip::FrameCodec.new(dict: 42) }
  end


  it "round trips message sharing dict prefix" do
    msg = "user_4242@example.com|status=active|tier=gold|region=eu-west-1"
    ct  = @codec.compress(msg)
    assert_equal msg, @codec.decompress(ct)
  end


  it "round trips random bytes" do
    msg = Random.bytes(4096)
    assert_equal msg, @codec.decompress(@codec.compress(msg))
  end


  it "round trips empty string" do
    ct = @codec.compress("")
    assert_equal "", @codec.decompress(ct)
  end


  it "emits zstd frame magic" do
    ct = @codec.compress("user_4242@example.com|status=active|tier=gold|region=eu-west-1")
    assert_equal [0x28, 0xB5, 0x2F, 0xFD], ct.bytes.first(4)
  end


  it "raises on dict mismatch" do
    t2 = Zrip::DictTrainer.new(8192)
    200.times { |i| t2.add_sample("totally different content #{i}" * 5) }
    other_dict = Zrip::Dictionary.new(bytes: t2.train)
    other = Zrip::FrameCodec.new(dict: other_dict)
    ct = @codec.compress("user_4242@example.com|status=active")
    assert_raises(Zrip::DecompressError) { other.decompress(ct) }
  end


  it "raises on garbage input" do
    assert_raises(Zrip::DecompressError) { @codec.decompress("garbage") }
  end


  it "dict compresses better" do
    msg = "user_4242@example.com|status=active|tier=gold|region=eu-west-1"
    ct_with = @codec.compress(msg)
    ct_without = @no_dict.compress(msg)
    assert_operator ct_with.bytesize, :<, ct_without.bytesize
  end
end


describe Zrip::BlockCodec, "with a dictionary" do
  before do
    t = Zrip::DictTrainer.new(8192)
    200.times do |i|
      t.add_sample("user_#{i}@example.com|status=active|tier=gold|region=eu-west-#{i % 4}")
    end
    dict_bytes = t.train
    @dict = Zrip::Dictionary.new(bytes: dict_bytes)
    @codec = Zrip::BlockCodec.new(dict: @dict)
    @no_dict = Zrip::BlockCodec.new
  end


  it "has dict is true" do
    assert_predicate @codec, :has_dict?
  end


  it "round trips" do
    msg = "user_4242@example.com|status=active|tier=gold|region=eu-west-1"
    ct  = @codec.compress(msg)
    assert_equal msg, @codec.decompress(ct)
  end


  it "dict compresses better" do
    msg = "user_4242@example.com|status=active|tier=gold|region=eu-west-1"
    ct_with    = @codec.compress(msg)
    ct_without = @no_dict.compress(msg)
    assert_operator ct_with.bytesize, :<, ct_without.bytesize
  end


  it "reuses context 500 times" do
    msgs = 500.times.map { |i| "user_#{i}@example.com|status=active" }
    ciphertexts = msgs.map { |m| @codec.compress(m) }
    msgs.zip(ciphertexts).each do |msg, ct|
      assert_equal msg, @codec.decompress(ct)
    end
  end
end


describe "Zrip Ractor safety" do
  before do
    require_ractor
  end


  it "compress decompress inside ractor" do
    r = Ractor.new do
      codec = Zrip::FrameCodec.new
      pt    = "hello from inside a ractor " * 100
      ct    = codec.compress(pt)
      [ct.bytesize, codec.decompress(ct) == pt]
    end
    size, ok = ractor_value(r)
    assert_equal true, ok
    assert_operator size, :>, 0
  end


  it "dictionary shareable across ractors" do
    dict = Zrip::Dictionary.new(bytes: "shared dict prefix " * 4)
    r = Ractor.new(dict) do |d|
      [d.bytes, d.id]
    end
    got_bytes, got_id = ractor_value(r)
    assert_equal ("shared dict prefix " * 4).b, got_bytes
    assert_kind_of Integer, got_id
  end


  it "shares one frame codec across ractors" do
    codec = Zrip::FrameCodec.new
    assert_predicate codec, :frozen?
    assert Ractor.shareable?(codec)
    ractors = 4.times.map do |i|
      Ractor.new(codec, i) do |shared, index|
        input = "shared payload #{index} " * 20_000
        10.times do
          raise "round trip mismatch" unless shared.decompress(shared.compress(input)) == input
        end
        :ok
      end
    end
    assert_equal [:ok] * 4, ractors.map { |ractor| ractor_value(ractor) }
  end


  it "block codec is per ractor" do
    r = Ractor.new do
      c   = Zrip::BlockCodec.new
      msg = "ractor local payload " * 50
      ct  = c.compress(msg)
      c.decompress(ct) == msg
    end
    assert_equal true, ractor_value(r)
  end


  it "block codec cannot cross ractor boundaries" do
    codec = Zrip::BlockCodec.new
    assert_raises(TypeError, Ractor::IsolationError) do
      Ractor.new(codec) { |c| c.compress("payload") }
    end
  end


  it "multiple ractors compress in parallel" do
    ractors = 4.times.map do |i|
      Ractor.new(i) do |idx|
        codec = Zrip::FrameCodec.new
        pt    = "ractor #{idx} payload " * 100
        100.times do
          ct = codec.compress(pt)
          raise "mismatch in ractor #{idx}" unless codec.decompress(ct) == pt
        end
        :ok
      end
    end
    results = ractors.map { |r| ractor_value(r) }
    assert_equal [:ok, :ok, :ok, :ok], results
  end
end


describe "Zrip native interruptions" do
  it "interrupted compression unlocks input and releases codec" do
    interrupted_error = Class.new(StandardError)
    skip "requires MRI GVL" unless RUBY_ENGINE == "ruby"

    codecs = [Zrip::FrameCodec.new, Zrip::BlockCodec.new]
    codecs.each do |codec|
      input = Random.bytes(16 * 1024 * 1024)
      worker = Thread.new do
        codec.compress(input)
      rescue interrupted_error => error
        error
      end
      worker.report_on_exception = false

      begin
        assert wait_for_input_lock(input, worker), "compression did not lock input"
        worker.raise(interrupted_error)
        assert_instance_of interrupted_error, worker.value
        input.replace("usable")
        assert_equal "usable", input
        assert_equal input, decompress(codec, codec.compress(input), input.bytesize)
      ensure
        worker.kill if worker.alive?
        worker.join
      end
    end
  end


  it "killed compression unlocks input" do
    skip "requires MRI GVL" unless RUBY_ENGINE == "ruby"

    codec = Zrip::FrameCodec.new
    input = Random.bytes(64 * 1024 * 1024)
    worker = Thread.new { codec.compress(input) }
    worker.report_on_exception = false
    begin
      assert wait_for_input_lock(input, worker), "compression did not lock input"
      worker.kill
      assert_nil worker.value
      input.replace("usable")
      assert_equal "usable", input
    ensure
      worker.kill if worker.alive?
      worker.join
    end
  end


  it "contended small compression locks input" do
    skip "requires MRI GVL" unless RUBY_ENGINE == "ruby"

    codec = Zrip::FrameCodec.new
    large_input = Random.bytes(128 * 1024 * 1024)
    small_input = +"small mutable input"
    large_worker = Thread.new { codec.compress(large_input) }
    small_worker = nil

    begin
      assert wait_for_input_lock(large_input, large_worker), "large compression did not lock input"
      small_worker = Thread.new { codec.compress(small_input) }
      assert wait_for_input_lock(small_input, small_worker), "contended small input was not locked"
      assert_raises(RuntimeError) { small_input.replace("changed") }
      small_compressed = small_worker.value
      large_worker.value
      assert_equal small_input, decompress(codec, small_compressed, small_input.bytesize)
      small_input.replace("usable")
      assert_equal "usable", small_input
    ensure
      [small_worker, large_worker].compact.each do |worker|
        worker.kill if worker.alive?
        worker.join
      end
    end
  end

  private

  def decompress(codec, compressed, size)
    codec.decompress(compressed)
  end

  def wait_for_input_lock(input, worker)
    deadline = Process.clock_gettime(Process::CLOCK_MONOTONIC) + 5
    while worker.alive? && Process.clock_gettime(Process::CLOCK_MONOTONIC) < deadline
      begin
        input.setbyte(0, input.getbyte(0))
      rescue RuntimeError
        return true
      end
      Thread.pass
    end
    false
  end
end


describe "Zrip native coercion" do
  it "decompression borrows input after size coercion" do
    [Zrip::FrameCodec.new, Zrip::BlockCodec.new].each do |codec|
      input = codec.compress("old")
      expected = "replacement payload " * 500
      size = Object.new
      size.define_singleton_method(:to_int) do
        input.replace(codec.compress(expected))
        expected.bytesize
      end

      actual = codec._native_decompress(input, size)
      assert_equal expected, actual
    end
  end
end
