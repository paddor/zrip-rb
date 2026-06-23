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
