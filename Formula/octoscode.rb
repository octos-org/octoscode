class Octoscode < Formula
  desc "Terminal UI client for the Octos UI Protocol"
  homepage "https://github.com/octos-org/octoscode"
  version "0.3.1"
  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/octos-org/octoscode/releases/download/v0.3.1/octoscode-aarch64-apple-darwin.tar.xz"
    sha256 "81155cf67dac609c1ad57379b9ee9b445faa736a7f7b7d056333f31eee0e3518"
  end
  if OS.linux?
    if Hardware::CPU.arm?
      url "https://github.com/octos-org/octoscode/releases/download/v0.3.1/octoscode-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "63a140b1a7e0ba441f34c348e684bfc6ba0c4ed93bf27796f712962fc78c7fc3"
    end
    if Hardware::CPU.intel?
      url "https://github.com/octos-org/octoscode/releases/download/v0.3.1/octoscode-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "5c8cb1fd208e991ab7299fd348a53ab2471f621d2418c4d58b964dd19ef3848e"
    end
  end
  license "Apache-2.0"

  # octoscode is a CLIENT; a local launch spawns `octos serve --stdio` as its
  # backend. We deliberately do NOT `depends_on "octos-org/octos/octos"`: Homebrew
  # does not auto-tap third-party dependency taps, so that would abort the
  # install with "tap must be installed explicitly". Instead the tui
  # auto-installs the octos server on first run if it's missing (see caveats).
  def caveats
    <<~EOS
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
