# frozen_string_literal: true

require_relative "lib/zrip/version"

Gem::Specification.new do |s|
  s.name     = "zrip"
  s.version  = Zrip::VERSION
  s.authors  = ["Patrik Wenger"]
  s.email    = ["paddor@gmail.com"]
  s.summary  = "Ractor-safe Zstandard bindings for Ruby (pure-Rust zrip backend)"
  s.description = "Ruby bindings (via Rust/magnus) for zrip, a pure-Rust " \
                  "Zstandard implementation. Frame-format and block-format " \
                  "compress/decompress with optional dictionary support, " \
                  "configurable compression levels, and FastCOVER-based " \
                  "dictionary training. Ractor-safe."
  s.homepage = "https://github.com/paddor/zrip-rb"
  s.license  = "MIT"

  s.required_ruby_version = ">= 4.0.0"

  s.metadata["homepage_uri"]      = s.homepage
  s.metadata["source_code_uri"]   = s.homepage
  s.metadata["changelog_uri"]     = "#{s.homepage}/blob/main/CHANGELOG.md"
  s.metadata["rubygems_mfa_required"] = "true"

  s.files = Dir[
    "lib/**/*.rb",
    "ext/**/*.{rs,rb}",
    "ext/**/Cargo.toml",
    "Cargo.toml",
    "Cargo.lock",
    "LICENSE",
    "README.md",
    "CHANGELOG.md",
  ]

  s.require_paths = ["lib"]
  s.extensions    = ["ext/zrip/extconf.rb"]

  s.add_dependency "rb_sys", "~> 0.9"
end
