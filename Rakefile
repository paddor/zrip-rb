# frozen_string_literal: true

require "bundler/gem_tasks"
require "rb_sys/extensiontask"
require "minitest/test_task"

GEMSPEC = Gem::Specification.load("zrip.gemspec") ||
          abort("Could not load zrip.gemspec")

RbSys::ExtensionTask.new("zrip", GEMSPEC) do |ext|
  ext.lib_dir = "lib/zrip"
end

Minitest::TestTask.create(:test) do |t|
  t.libs       << "lib" << "test"
  t.test_globs  = ["test/test_*.rb"]
end

namespace :soak do
  Minitest::TestTask.create(:run) do |t|
    t.libs       << "lib" << "test"
    t.test_globs  = ["test/soak_test.rb"]
  end

  begin
    require "ruby_memcheck"

    desc "Run soak tests under ruby_memcheck"
    RubyMemcheck::TestTask.new(memcheck: :compile) do |t|
      t.libs << "lib" << "test"
      t.test_files = FileList["test/soak_test.rb"]
    end
  rescue LoadError
    desc "Run soak tests under ruby_memcheck"
    task :memcheck do
      abort "Install ruby_memcheck to run rake soak:memcheck"
    end
  end
end

desc "Run native soak tests"
task soak: :compile do
  ENV["SOAK_SECONDS"] ||= "600"
  Rake::Task["soak:run"].invoke
end

desc "Run Rust unit tests"
task :cargo_test do
  sh "RUBY=#{RbConfig.ruby} cargo test --lib --manifest-path ext/zrip/Cargo.toml"
end

desc "Run Clippy lints"
task :clippy do
  sh "cargo clippy --manifest-path ext/zrip/Cargo.toml -- -D warnings"
end

desc "Format Rust code"
task :fmt do
  sh "cargo fmt --manifest-path ext/zrip/Cargo.toml"
end

desc "Run all tests (Ruby + Rust)"
task test_all: [:test, :cargo_test]

task build: :compile
task default: [:compile, :test]
