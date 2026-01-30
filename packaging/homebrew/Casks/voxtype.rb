cask "voxtype" do
  version "0.6.0-rc1"
  sha256 "32aaaacc37688996f68588e45da2a53bbc05783591d78a07be58be28d0c044c0"

  url "https://github.com/peteonrails/voxtype/releases/download/v#{version}/Voxtype-#{version}-macos-arm64.dmg"
  name "Voxtype"
  desc "Push-to-talk voice-to-text for macOS"
  homepage "https://voxtype.io"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :ventura"

  app "Voxtype.app"

  postflight do
    # Create config directory
    system_command "/bin/mkdir", args: ["-p", "#{ENV["HOME"]}/Library/Application Support/voxtype"]

    # Create symlink for CLI access
    system_command "/bin/ln", args: ["-sf", "/Applications/Voxtype.app/Contents/MacOS/voxtype", "#{HOMEBREW_PREFIX}/bin/voxtype"]
  end

  uninstall_postflight do
    # Remove CLI symlink
    system_command "/bin/rm", args: ["-f", "#{HOMEBREW_PREFIX}/bin/voxtype"]
  end

  uninstall quit: "io.voxtype.app"

  zap trash: [
    "~/Library/Application Support/voxtype",
    "~/Library/LaunchAgents/io.voxtype.daemon.plist",
    "~/Library/Logs/voxtype",
  ]

  caveats <<~EOS
    If macOS says the app is "damaged", run:
       xattr -cr /Applications/Voxtype.app

    To complete setup:

    1. Download a speech model:
       voxtype setup --download --model parakeet-tdt-0.6b-v3-int8

    2. Grant permissions in System Settings > Privacy & Security:
       - Microphone: Add Voxtype
       - Input Monitoring: Add Voxtype
       - Accessibility: Add Voxtype

    3. Start the daemon:
       voxtype daemon

    To start at login:
       voxtype setup launchd

    Default hotkey: Right Option
  EOS
end
