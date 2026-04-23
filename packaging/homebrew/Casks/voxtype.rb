cask "voxtype" do
  version "0.6.0-rc1"
  sha256 "791963b523e84c3569cae2e64fae02bb782e9ce1bf0f244b8f45a8149ad80dd8"

  url "file:///Users/pete/workspace/voxtype/releases/0.6.0-rc1/Voxtype-0.6.0-rc1-macos-arm64.dmg"
  name "Voxtype"
  desc "Push-to-talk voice-to-text for macOS"
  homepage "https://voxtype.io"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :ventura"
  depends_on formula: "terminal-notifier"

  app "Voxtype.app"

  postflight do
    # Remove quarantine attribute (app is unsigned)
    system_command "/usr/bin/xattr", args: ["-cr", "/Applications/Voxtype.app"]

    # Clean up any stale state from previous installs
    system_command "/bin/rm", args: ["-rf", "/tmp/voxtype"]

    # Create config directory
    system_command "/bin/mkdir", args: ["-p", "#{ENV["HOME"]}/Library/Application Support/voxtype"]

    # Create logs directory
    system_command "/bin/mkdir", args: ["-p", "#{ENV["HOME"]}/Library/Logs/voxtype"]

    # Bundle terminal-notifier for notifications with custom icon
    system_command "/bin/cp", args: [
      "-R",
      "#{HOMEBREW_PREFIX}/opt/terminal-notifier/terminal-notifier.app",
      "/Applications/Voxtype.app/Contents/Resources/"
    ]

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

    # Run initial setup to create config and download model
    # This is non-interactive and downloads the smallest fast model
    system_command "/Applications/Voxtype.app/Contents/MacOS/voxtype",
      args: ["setup", "--download", "--model", "parakeet-tdt-0.6b-v3-int8"],
      print_stdout: true

    # Load the LaunchAgent to start the daemon
    # It will work once user grants permissions
    system_command "/bin/launchctl", args: ["load", plist_path]

    # Launch Settings app to Permissions pane so user can grant access
    system_command "/usr/bin/open", args: ["/Applications/Voxtype.app/Contents/MacOS/VoxtypeSetup.app"]
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
    Voxtype is installed and the daemon is running!

    The Settings app opened to help you grant permissions:
    1. Click "Grant Access" for Input Monitoring (hotkey detection)
    2. Click "Grant Access" for Microphone (recording)

    Once permissions are granted, hold Right Option to record.

    If prompted "Voxtype was blocked", go to:
      System Settings > Privacy & Security > click "Open Anyway"
  EOS
end
