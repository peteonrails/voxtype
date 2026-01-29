class Voxtype < Formula
  desc "Push-to-talk voice-to-text for macOS and Linux"
  homepage "https://voxtype.io"
  url "https://github.com/peteonrails/voxtype/archive/refs/tags/v0.6.0-rc.1.tar.gz"
  sha256 "PLACEHOLDER_SHA256"
  license "MIT"
  head "https://github.com/peteonrails/voxtype.git", branch: "feature/macos-release"

  depends_on "cmake" => :build
  depends_on "rust" => :build
  depends_on "pkg-config" => :build

  # macOS dependencies
  on_macos do
    depends_on "portaudio"
  end

  # Linux dependencies
  on_linux do
    depends_on "alsa-lib"
    depends_on "libxkbcommon"
  end

  def install
    # Build release binary
    system "cargo", "install", *std_cargo_args
  end

  def post_install
    # Create config directory
    (var/"voxtype").mkpath
  end

  def caveats
    <<~EOS
      To start using voxtype:

      1. Run the setup wizard:
         voxtype setup macos

      2. Or start the daemon directly:
         voxtype daemon

      To have voxtype start at login:
         voxtype setup launchd

      Default hotkey: Right Option (⌥)

      For more information:
         voxtype --help
         https://voxtype.io/docs
    EOS
  end

  service do
    run [opt_bin/"voxtype", "daemon"]
    keep_alive true
    log_path var/"log/voxtype.log"
    error_log_path var/"log/voxtype.log"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/voxtype --version")
  end
end
