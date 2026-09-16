# Prerelease-channel formula for octoscode (class OctoscodeDev).
#
# This is the DEV/prerelease sibling of Formula/octoscode.rb. It is rendered by
# .github/workflows/publish-homebrew.yml on a PRERELEASE tag push (a tag with a
# '-', e.g. v0.2.2-rc.15) into Formula/octoscode-dev.rb, filling the same
# 0.3.0-rc.10/v0.3.0-rc.10/__SHA_*__ placeholders from that prerelease's assets. The
# stable Formula/octoscode.rb is NEVER touched by a prerelease tag, so
# `brew install octos-org/octoscode/octoscode` stays on the latest STABLE while
# `brew install octos-org/octoscode/octoscode-dev` tracks the latest prerelease.
#
# MUTUALLY EXCLUSIVE with the stable formula: both install a binary named
# `octoscode`, so only one may be linked at a time (see `conflicts_with` below).
# This is the standard `foo` vs `foo-dev` pattern — install one or the other.
class OctoscodeDev < Formula
  desc "Terminal UI client for the Octos UI Protocol (prerelease channel)"
  homepage "https://github.com/octos-org/octoscode"
  version "0.3.0-rc.10"
  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/octos-org/octoscode/releases/download/v0.3.0-rc.10/octoscode-aarch64-apple-darwin.tar.xz"
    sha256 "41fc01eb0207b57518e759a70362416d249276b54042fbb36d66e9fd88bee706"
  end
  if OS.linux?
    if Hardware::CPU.arm?
      url "https://github.com/octos-org/octoscode/releases/download/v0.3.0-rc.10/octoscode-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "89937eccd493feb0c193c4d21861093bf6c446df7c2f22820078f40089b81e08"
    end
    if Hardware::CPU.intel?
      url "https://github.com/octos-org/octoscode/releases/download/v0.3.0-rc.10/octoscode-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "2cf0124a8ada23f1633fc27d959bf103eb1f817a13362b079097ca670cadd63c"
    end
  end
  license "Apache-2.0"

  # Dev and stable both provide `bin/octoscode`; they cannot be linked together.
  # Installing this formula while `octoscode` is linked (or vice versa) prompts
  # to `brew unlink` the other first, keeping the two channels cleanly separate.
  conflicts_with "octoscode", because: "both install the octoscode binary (prerelease vs stable channel)"

  # octoscode is a CLIENT; a local launch spawns `octos serve --stdio` as its
  # backend. We deliberately do NOT `depends_on "octos-org/octos/octos"`: Homebrew
  # does not auto-tap third-party dependency taps, so that would abort the
  # install with "tap must be installed explicitly". Instead the tui
  # auto-installs the octos server on first run if it's missing (see caveats).
  def caveats
    <<~EOS
      octoscode-dev is the PRERELEASE (rc/beta) channel; the stable formula is
      `octos-org/octoscode/octoscode`. Only one may be linked at a time.

      octoscode talks to the `octos` server backend. If octos isn't installed,
      octoscode installs the latest release automatically on first run
      (set OCTOSCODE_NO_AUTO_INSTALL=1 to disable). To install it up front:
        brew install octos-org/octos/octos
    EOS
  end

  BINARY_ALIASES = {
    "aarch64-apple-darwin":      {},
    "aarch64-unknown-linux-gnu": {},
    "x86_64-pc-windows-gnu":     {},
    "x86_64-unknown-linux-gnu":  {},
  }.freeze

  def target_triple
    cpu = Hardware::CPU.arm? ? "aarch64" : "x86_64"
    os = OS.mac? ? "apple-darwin" : "unknown-linux-gnu"

    "#{cpu}-#{os}"
  end

  def install_binary_aliases!
    BINARY_ALIASES[target_triple.to_sym].each do |source, dests|
      dests.each do |dest|
        bin.install_symlink bin/source.to_s => dest
      end
    end
  end

  def install
    bin.install "octoscode" if OS.mac? && Hardware::CPU.arm?
    bin.install "octoscode" if OS.linux? && Hardware::CPU.arm?
    bin.install "octoscode" if OS.linux? && Hardware::CPU.intel?

    install_binary_aliases!

    # Homebrew will automatically install these, so we don't need to do that
    doc_files = Dir["README.*", "readme.*", "LICENSE", "LICENSE.*", "CHANGELOG.*"]
    leftover_contents = Dir["*"] - doc_files

    # Install any leftover files in pkgshare; these are probably config or
    # sample files.
    pkgshare.install(*leftover_contents) unless leftover_contents.empty?
  end
end
