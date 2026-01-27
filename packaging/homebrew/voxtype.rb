# Homebrew Cask formula for Voxtype
#
# To use this cask:
#   1. Create a tap: brew tap peteonrails/voxtype
#   2. Install: brew install --cask voxtype
#
# Or install directly:
#   brew install --cask peteonrails/voxtype/voxtype

cask "voxtype" do
  version "0.5.0"
  sha256 "PLACEHOLDER_SHA256"

  url "https://github.com/peteonrails/voxtype/releases/download/v#{version}/voxtype-#{version}-macos-universal.dmg",
      verified: "github.com/peteonrails/voxtype/"
  name "Voxtype"
  desc "Push-to-talk voice-to-text using Whisper"
  homepage "https://voxtype.io"

  livecheck do
    url :url
    strategy :github_latest
  end

  # Universal binary supports both Intel and Apple Silicon
  depends_on macos: ">= :big_sur"

  binary "voxtype"

  postflight do
    # Remind user about Accessibility permissions
    ohai "Voxtype requires Accessibility permissions to detect hotkeys."
    ohai "Grant access in: System Preferences > Privacy & Security > Accessibility"
    ohai ""
    ohai "Quick start:"
    ohai "  voxtype setup         # Check dependencies, download model"
    ohai "  voxtype setup launchd # Install as LaunchAgent (auto-start)"
    ohai "  voxtype daemon        # Start manually"
  end

  uninstall launchctl: "io.voxtype.daemon"

  zap trash: [
    "~/Library/LaunchAgents/io.voxtype.daemon.plist",
    "~/Library/Logs/voxtype",
    "~/.config/voxtype",
    "~/.local/share/voxtype",
  ]

  caveats <<~EOS
    Voxtype requires Accessibility permissions to detect global hotkeys.

    After installation:
    1. Open System Preferences > Privacy & Security > Accessibility
    2. Add and enable voxtype (or the Terminal app if running from terminal)

    To install as a LaunchAgent (auto-start on login):
      voxtype setup launchd

    To start manually:
      voxtype daemon
  EOS
end
