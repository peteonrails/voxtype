cask "voxtype" do
  version "0.6.0-rc1"
  sha256 "ad5c4f2531ed50ed028ec7e85062abeb2e64c27e8d1becb84b4946b631ba7aeb"

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
    # Remove quarantine attribute (app is unsigned)
    system_command "/usr/bin/xattr", args: ["-cr", "/Applications/Voxtype.app"]

    # Create config directory
    system_command "/bin/mkdir", args: ["-p", "#{ENV["HOME"]}/Library/Application Support/voxtype"]

    # Create logs directory
    system_command "/bin/mkdir", args: ["-p", "#{ENV["HOME"]}/Library/Logs/voxtype"]

    # Create symlink for CLI access
    system_command "/bin/ln", args: ["-sf", "/Applications/Voxtype.app/Contents/MacOS/voxtype", "#{HOMEBREW_PREFIX}/bin/voxtype"]

    # Install LaunchAgent for auto-start
    launch_agents_dir = "#{ENV["HOME"]}/Library/LaunchAgents"
    system_command "/bin/mkdir", args: ["-p", launch_agents_dir]

    plist_path = "#{launch_agents_dir}/io.voxtype.daemon.plist"
    plist_content = <<~PLIST
      <?xml version="1.0" encoding="UTF-8"?>
      <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
      <plist version="1.0">
      <dict>
          <key>Label</key>
          <string>io.voxtype.daemon</string>
          <key>ProgramArguments</key>
          <array>
              <string>/Applications/Voxtype.app/Contents/MacOS/voxtype</string>
              <string>daemon</string>
          </array>
          <key>RunAtLoad</key>
          <true/>
          <key>KeepAlive</key>
          <true/>
          <key>StandardOutPath</key>
          <string>#{ENV["HOME"]}/Library/Logs/voxtype/stdout.log</string>
          <key>StandardErrorPath</key>
          <string>#{ENV["HOME"]}/Library/Logs/voxtype/stderr.log</string>
          <key>EnvironmentVariables</key>
          <dict>
              <key>PATH</key>
              <string>/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin</string>
          </dict>
          <key>ProcessType</key>
          <string>Interactive</string>
          <key>Nice</key>
          <integer>-10</integer>
      </dict>
      </plist>
    PLIST

    File.write(plist_path, plist_content)

    # Load the LaunchAgent
    system_command "/bin/launchctl", args: ["load", plist_path]
  end

  uninstall_postflight do
    # Unload and remove LaunchAgent
    plist_path = "#{ENV["HOME"]}/Library/LaunchAgents/io.voxtype.daemon.plist"
    system_command "/bin/launchctl", args: ["unload", plist_path] if File.exist?(plist_path)
    system_command "/bin/rm", args: ["-f", plist_path]

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
    Voxtype is installed and the daemon will start automatically.

    To complete setup:

    1. Download a speech model:
       voxtype setup --download --model parakeet-tdt-0.6b-v3-int8

    2. Grant permissions when prompted, or manually in System Settings:
       Privacy & Security > Microphone, Input Monitoring, Accessibility

    Default hotkey: Right Option (hold to record, release to transcribe)

    For menu bar status icon: voxtype menubar
  EOS
end
