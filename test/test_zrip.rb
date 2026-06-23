# frozen_string_literal: true

require_relative "test_helper"
require "objspace"
require "digest"


class TestVersion < Minitest::Test
  def test_version_is_a_non_empty_string
    assert_instance_of String, Zrip::VERSION
    refute_empty Zrip::VERSION
  end
end


class TestFrameCodecNoDict < Minitest::Test
  def setup
    @codec = Zrip::FrameCodec.new
  end


  def test_round_trips_empty_string
    ct = @codec.compress("")
    assert_equal "", @codec.decompress(ct)
  end


  def test_round_trips_single_byte
    ct = @codec.compress("x")
    assert_equal "x", @codec.decompress(ct)
  end


  def test_round_trips_ascii_text
    pt = "the quick brown fox jumps over the lazy dog"
    assert_equal pt, @codec.decompress(@codec.compress(pt))
  end


  def test_round_trips_repetitive_input_and_compresses
    pt = "A" * 100_000
    ct = @codec.compress(pt)
    assert_operator ct.bytesize, :<, pt.bytesize / 10
    assert_equal pt, @codec.decompress(ct)
  end


  def test_round_trips_random_bytes_1mib
    pt = Random.bytes(1_048_576)
    assert_equal pt, @codec.decompress(@codec.compress(pt))
  end


  def test_round_trips_binary_data_with_nul_bytes
    pt = (0..255).map(&:chr).join * 16
    pt.force_encoding(Encoding::ASCII_8BIT)
    assert_equal pt, @codec.decompress(@codec.compress(pt))
  end


  def test_emits_zstd_frame_magic
    ct = @codec.compress("anything")
    assert_equal [0x28, 0xB5, 0x2F, 0xFD], ct.bytes.first(4)
  end


  def test_accepts_explicit_compression_level
    c1 = Zrip::FrameCodec.new(level: -3)
    c3 = Zrip::FrameCodec.new(level: 4)
    pt = ("X" * 100_000).b
    refute_equal c1.compress(pt), c3.compress(pt)
    assert_equal pt, c1.decompress(c1.compress(pt))
    assert_equal pt, c3.decompress(c3.compress(pt))
    assert_equal(-3, c1.level)
    assert_equal 4, c3.level
  end


  def test_compress_returns_binary_encoding
    ct = @codec.compress("hello")
    assert_equal Encoding::ASCII_8BIT, ct.encoding
  end


  def test_decompress_returns_binary_encoding
    pt = @codec.decompress(@codec.compress("hello"))
    assert_equal Encoding::ASCII_8BIT, pt.encoding
  end


  def test_raises_decompress_error_on_garbage
    assert_raises(Zrip::DecompressError) { @codec.decompress("not a valid zstd frame") }
  end


  def test_raises_decompress_error_on_empty_input
    assert_raises(Zrip::DecompressError) { @codec.decompress("") }
  end


  def test_decompress_error_is_standard_error_subclass
    assert_includes Zrip::DecompressError.ancestors, StandardError
  end


  def test_compress_error_is_standard_error_subclass
    assert_includes Zrip::CompressError.ancestors, StandardError
  end


  def test_has_dict_false_and_id_nil
    refute_predicate @codec, :has_dict?
    assert_nil @codec.id
    assert_equal 0, @codec.size
  end
end


class TestGetFrameContentSize < Minitest::Test
  def setup
    @codec = Zrip::FrameCodec.new
  end


  def test_returns_declared_content_size
    pt = "X" * 12_345
    ct = @codec.compress(pt)
    assert_equal pt.bytesize, Zrip::FrameCodec.get_frame_content_size(ct)
  end


  def test_raises_decompress_error_on_garbage
    assert_raises(Zrip::DecompressError) do
      Zrip::FrameCodec.get_frame_content_size("not a valid zstd frame")
    end
  end


  def test_raises_decompress_error_on_empty_input
    assert_raises(Zrip::DecompressError) do
      Zrip::FrameCodec.get_frame_content_size("")
    end
  end
end


class TestDosResistance < Minitest::Test
  def test_no_large_output_string_on_failed_decompress
    codec   = Zrip::FrameCodec.new
    size    = 1_048_576
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


class TestDictionary < Minitest::Test
  def setup
    @bytes = "header version=1 type=message field1=" * 4
  end


  def test_stores_bytes_binary_encoded_and_frozen
    d = Zrip::Dictionary.new(bytes: @bytes)
    assert_equal @bytes.b, d.bytes
    assert_predicate d.bytes, :frozen?
    assert_equal Encoding::ASCII_8BIT, d.bytes.encoding
  end


  def test_defaults_id_to_sha256_mapped_into_public_range
    raw = Digest::SHA256.digest(@bytes)[0, 4].unpack1("V")
    expected = Zrip::Dictionary::USER_DICT_ID_MIN +
      (raw % Zrip::Dictionary::USER_DICT_ID_SIZE)
    assert_equal expected, Zrip::Dictionary.new(bytes: @bytes).id
  end


  def test_auto_generated_ids_stay_within_public_range
    range = Zrip::Dictionary::USER_DICT_ID_MIN..Zrip::Dictionary::USER_DICT_ID_MAX
    200.times do
      dd = Zrip::Dictionary.new(bytes: Random.bytes(128))
      assert_includes range, dd.id
    end
  end


  def test_accepts_caller_supplied_id
    d = Zrip::Dictionary.new(bytes: @bytes, id: 0xDEAD_BEEF)
    assert_equal 0xDEAD_BEEF, d.id
  end


  def test_size_reports_dict_size_in_bytes
    assert_equal @bytes.bytesize, Zrip::Dictionary.new(bytes: @bytes).size
  end


  def test_immutability_and_value_equality
    assert_predicate Zrip::Dictionary.new(bytes: @bytes), :frozen?
    assert_equal Zrip::Dictionary.new(bytes: @bytes), Zrip::Dictionary.new(bytes: @bytes.dup)
  end


  def test_shareable_across_ractors
    r = Ractor.new(Zrip::Dictionary.new(bytes: @bytes)) { |d| [d.bytes, d.id] }
    got_bytes, got_id = r.value
    assert_equal @bytes.b, got_bytes
    assert_kind_of Integer, got_id
  end
end


class TestBlockCodecNoDict < Minitest::Test
  def setup
    @codec = Zrip::BlockCodec.new
  end


  def test_has_dict_is_false
    refute_predicate @codec, :has_dict?
  end


  def test_round_trips_empty_string
    ct = @codec.compress("")
    assert_equal "", @codec.decompress(ct)
  end


  def test_round_trips_ascii_text
    pt = "the quick brown fox jumps over the lazy dog"
    ct = @codec.compress(pt)
    assert_equal pt, @codec.decompress(ct)
  end


  def test_round_trips_repetitive_input_and_compresses
    pt = "A" * 100_000
    ct = @codec.compress(pt)
    assert_operator ct.bytesize, :<, pt.bytesize / 10
    assert_equal pt, @codec.decompress(ct)
  end


  def test_round_trips_across_size_buckets
    [0, 1, 64, 255, 256, 1024, 4096, 65_536, 1_048_576].each do |n|
      pt = Random.bytes(n)
      ct = @codec.compress(pt)
      assert_equal pt, @codec.decompress(ct),
        "round-trip failed at size #{n}"
    end
  end


  def test_emits_binary_encoded_output
    ct = @codec.compress("hello")
    assert_equal Encoding::ASCII_8BIT, ct.encoding
  end


  def test_reuses_context_across_many_calls
    500.times do |i|
      pt = "message #{i} " * (1 + i % 10)
      ct = @codec.compress(pt)
      assert_equal pt, @codec.decompress(ct)
    end
  end


  def test_default_level
    assert_equal Zrip::DEFAULT_LEVEL, @codec.level
  end


  def test_size_is_zero_without_dict
    assert_equal 0, @codec.size
  end
end


class TestDictTrainer < Minitest::Test
  def json_msg(i)
    %Q({"ts":"2026-04-27T12:00:00.#{format("%04d", i)}Z","level":"INFO","service":"api-gw","trace":"#{format("%08x", i)}","method":"GET","path":"/v1/users/#{format("%04d", i)}","status":200,"latency_ms":#{10 + i % 490},"region":"us-east-1"})
  end


  def test_starts_with_zero_samples
    t = Zrip::DictTrainer.new(2048)
    assert_equal 0, t.sample_count
    assert_equal 0, t.total_bytes
    refute_predicate t, :trained?
  end


  def test_trains_nonempty_dict_from_100_json_samples
    t = Zrip::DictTrainer.new(8192)
    100.times { |i| t.add_sample(json_msg(i)) }
    assert_operator t.sample_count, :>, 0
    assert_operator t.total_bytes, :>, 0

    dict = t.train
    assert_predicate t, :trained?
    refute_empty dict
    assert_equal Encoding::ASCII_8BIT, dict.encoding
  end


  def test_trained_dict_is_zdict_format
    t = Zrip::DictTrainer.new(8192)
    200.times { |i| t.add_sample(json_msg(i)) }
    dict = t.train
    assert_equal Zrip::Dictionary::ZDICT_MAGIC, dict.byteslice(0, 4)
    header_id = dict.byteslice(4, 4).unpack1("V")
    refute_equal 0, header_id
  end


  def test_trained_dict_improves_frame_codec_compression
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


  def test_returns_empty_dict_with_fewer_than_2_samples
    t = Zrip::DictTrainer.new(2048)
    t.add_sample("hello world")
    assert_equal "", t.train
  end


  def test_skips_samples_shorter_than_4_bytes
    t = Zrip::DictTrainer.new(2048)
    t.add_sample("hi")
    assert_equal 0, t.sample_count
  end


  def test_raises_runtime_error_on_double_train
    t = Zrip::DictTrainer.new(2048)
    10.times { |i| t.add_sample(json_msg(i)) }
    t.train
    assert_raises(RuntimeError) { t.train }
  end


  def test_raises_runtime_error_on_add_sample_after_train
    t = Zrip::DictTrainer.new(2048)
    t.add_sample("hello world")
    t.train
    assert_raises(RuntimeError) { t.add_sample("more data") }
  end


  def test_raises_runtime_error_on_sample_count_after_train
    t = Zrip::DictTrainer.new(2048)
    t.train
    assert_raises(RuntimeError) { t.sample_count }
  end


  def test_raises_runtime_error_on_total_bytes_after_train
    t = Zrip::DictTrainer.new(2048)
    t.train
    assert_raises(RuntimeError) { t.total_bytes }
  end


  def test_cannot_cross_ractor_boundaries
    t = Zrip::DictTrainer.new(2048)
    assert_raises(TypeError, Ractor::IsolationError) do
      Ractor.new(t) { |tr| tr.add_sample("data") }
    end
  end
end


class TestFrameCodecWithDict < Minitest::Test
  def setup
    t = Zrip::DictTrainer.new(8192)
    200.times do |i|
      t.add_sample("user_#{i}@example.com|status=active|tier=gold|region=eu-west-#{i % 4}")
    end
    dict_bytes = t.train
    @dict = Zrip::Dictionary.new(bytes: dict_bytes)
    @codec = Zrip::FrameCodec.new(dict: @dict)
    @no_dict = Zrip::FrameCodec.new
  end


  def test_uses_dicts_cached_id
    assert_equal @dict.id, @codec.id
    assert_predicate @codec, :has_dict?
  end


  def test_raises_type_error_for_bad_dict
    assert_raises(TypeError) { Zrip::FrameCodec.new(dict: 42) }
  end


  def test_round_trips_message_sharing_dict_prefix
    msg = "user_4242@example.com|status=active|tier=gold|region=eu-west-1"
    ct  = @codec.compress(msg)
    assert_equal msg, @codec.decompress(ct)
  end


  def test_round_trips_random_bytes
    msg = Random.bytes(4096)
    assert_equal msg, @codec.decompress(@codec.compress(msg))
  end


  def test_round_trips_empty_string
    ct = @codec.compress("")
    assert_equal "", @codec.decompress(ct)
  end


  def test_emits_zstd_frame_magic
    ct = @codec.compress("user_4242@example.com|status=active|tier=gold|region=eu-west-1")
    assert_equal [0x28, 0xB5, 0x2F, 0xFD], ct.bytes.first(4)
  end


  def test_raises_on_dict_mismatch
    t2 = Zrip::DictTrainer.new(8192)
    200.times { |i| t2.add_sample("totally different content #{i}" * 5) }
    other_dict = Zrip::Dictionary.new(bytes: t2.train)
    other = Zrip::FrameCodec.new(dict: other_dict)
    ct = @codec.compress("user_4242@example.com|status=active")
    assert_raises(Zrip::DecompressError) { other.decompress(ct) }
  end


  def test_raises_on_garbage_input
    assert_raises(Zrip::DecompressError) { @codec.decompress("garbage") }
  end


  def test_dict_compresses_better
    msg = "user_4242@example.com|status=active|tier=gold|region=eu-west-1"
    ct_with = @codec.compress(msg)
    ct_without = @no_dict.compress(msg)
    assert_operator ct_with.bytesize, :<, ct_without.bytesize
  end
end


class TestBlockCodecWithDict < Minitest::Test
  def setup
    t = Zrip::DictTrainer.new(8192)
    200.times do |i|
      t.add_sample("user_#{i}@example.com|status=active|tier=gold|region=eu-west-#{i % 4}")
    end
    dict_bytes = t.train
    @dict = Zrip::Dictionary.new(bytes: dict_bytes)
    @codec = Zrip::BlockCodec.new(dict: @dict)
    @no_dict = Zrip::BlockCodec.new
  end


  def test_has_dict_is_true
    assert_predicate @codec, :has_dict?
  end


  def test_round_trips
    msg = "user_4242@example.com|status=active|tier=gold|region=eu-west-1"
    ct  = @codec.compress(msg)
    assert_equal msg, @codec.decompress(ct)
  end


  def test_dict_compresses_better
    msg = "user_4242@example.com|status=active|tier=gold|region=eu-west-1"
    ct_with    = @codec.compress(msg)
    ct_without = @no_dict.compress(msg)
    assert_operator ct_with.bytesize, :<, ct_without.bytesize
  end


  def test_reuses_context_500_times
    msgs = 500.times.map { |i| "user_#{i}@example.com|status=active" }
    ciphertexts = msgs.map { |m| @codec.compress(m) }
    msgs.zip(ciphertexts).each do |msg, ct|
      assert_equal msg, @codec.decompress(ct)
    end
  end
end


class TestRactorSafety < Minitest::Test
  def test_compress_decompress_inside_ractor
    r = Ractor.new do
      codec = Zrip::FrameCodec.new
      pt    = "hello from inside a ractor " * 100
      ct    = codec.compress(pt)
      [ct.bytesize, codec.decompress(ct) == pt]
    end
    size, ok = r.value
    assert_equal true, ok
    assert_operator size, :>, 0
  end


  def test_dictionary_shareable_across_ractors
    dict = Zrip::Dictionary.new(bytes: "shared dict prefix " * 4)
    r = Ractor.new(dict) do |d|
      [d.bytes, d.id]
    end
    got_bytes, got_id = r.value
    assert_equal ("shared dict prefix " * 4).b, got_bytes
    assert_kind_of Integer, got_id
  end


  def test_block_codec_is_per_ractor
    r = Ractor.new do
      c   = Zrip::BlockCodec.new
      msg = "ractor local payload " * 50
      ct  = c.compress(msg)
      c.decompress(ct) == msg
    end
    assert_equal true, r.value
  end


  def test_block_codec_cannot_cross_ractor_boundaries
    codec = Zrip::BlockCodec.new
    assert_raises(TypeError, Ractor::IsolationError) do
      Ractor.new(codec) { |c| c.compress("payload") }
    end
  end


  def test_multiple_ractors_compress_in_parallel
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
    results = ractors.map(&:value)
    assert_equal [:ok, :ok, :ok, :ok], results
  end
end
