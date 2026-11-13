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
    # Build release binary with parakeet support on macOS
    if OS.mac?
      system "cargo", "install", *std_cargo_args, "--features", "parakeet"
    else
      system "cargo", "install", *std_cargo_args
    end
  end

  def post_install
    # Create config directory
    (var/"voxtype").mkpath

    # Create app bundle for macOS permissions
    if OS.mac?
      # Create app bundle in Homebrew prefix (writable by Homebrew)
      app_path = prefix/"Voxtype.app"
      contents_path = app_path/"Contents"
      macos_path = contents_path/"MacOS"
      resources_path = contents_path/"Resources"

      # Create directory structure
      macos_path.mkpath
      resources_path.mkpath

      # Copy binary to app bundle (named voxtype-bin to match CFBundleExecutable)
      cp bin/"voxtype", macos_path/"voxtype-bin"

      # Create Info.plist
      info_plist = <<~PLIST
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
            <key>CFBundleExecutable</key>
            <string>voxtype-bin</string>
            <key>CFBundleIdentifier</key>
            <string>io.voxtype.daemon</string>
            <key>CFBundleName</key>
            <string>Voxtype</string>
            <key>CFBundleDisplayName</key>
            <string>Voxtype</string>
            <key>CFBundlePackageType</key>
            <string>APPL</string>
            <key>CFBundleShortVersionString</key>
            <string>#{version}</string>
            <key>CFBundleVersion</key>
            <string>#{version}</string>
            <key>LSMinimumSystemVersion</key>
            <string>13.0</string>
            <key>LSUIElement</key>
            <true/>
            <key>NSHighResolutionCapable</key>
            <true/>
            <key>NSMicrophoneUsageDescription</key>
            <string>Voxtype needs microphone access for speech-to-text transcription.</string>
            <key>NSAppleEventsUsageDescription</key>
            <string>Voxtype needs accessibility access to type transcribed text.</string>
            <key>NSInputMonitoringUsageDescription</key>
            <string>Voxtype monitors keyboard input to detect your push-to-talk hotkey.</string>
        </dict>
        </plist>
      PLIST

      (contents_path/"Info.plist").write(info_plist)

      # Sign the app bundle
      system "codesign", "--force", "--deep", "--sign", "-", app_path

      # Create symlink in ~/Applications for easy access
      user_apps = Pathname.new(Dir.home)/"Applications"
      user_apps.mkpath rescue nil
      user_app_link = user_apps/"Voxtype.app"

      # Remove old symlink/app if exists
      user_app_link.rmtree if user_app_link.exist? || user_app_link.symlink?

      # Create symlink
      begin
        FileUtils.ln_sf(app_path, user_app_link)
        ohai "Created #{user_app_link} -> #{app_path}"
      rescue => e
        opoo "Could not create symlink in ~/Applications: #{e.message}"
      end
    end
  end

  def caveats
    <<~EOS
      Voxtype.app has been installed and linked to ~/Applications.

      To get started, open Voxtype.app:
        open ~/Applications/Voxtype.app

      Voxtype will automatically:
        - Download a speech model on first launch
        - Prompt for Microphone and Accessibility permissions

      Default hotkey: fn (Globe key)
      More info: voxtype --help
    EOS
  end

  service do
    # Use app bundle path for proper macOS permissions
    run [opt_prefix/"Voxtype.app/Contents/MacOS/voxtype-bin", "daemon"]
    keep_alive true
    log_path var/"log/voxtype.log"
    error_log_path var/"log/voxtype.log"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/voxtype --version")
  end
end
