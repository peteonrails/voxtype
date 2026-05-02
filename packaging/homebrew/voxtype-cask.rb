cask "voxtype" do
  version "0.7.0"
  sha256 "PLACEHOLDER_SHA256"

  url "https://github.com/peteonrails/voxtype/releases/download/v#{version}/Voxtype-#{version}-macos-universal.dmg"
  name "Voxtype"
  desc "Push-to-talk voice-to-text"
  homepage "https://voxtype.io"

  depends_on macos: ">= :ventura"

  app "Voxtype.app"

  postflight do
    # Ensure CLI is accessible from PATH
    binary = "#{appdir}/Voxtype.app/Contents/MacOS/voxtype-bin"
    if File.exist?(binary)
      FileUtils.ln_sf(binary, "/usr/local/bin/voxtype")
    end
  end

  uninstall quit: "io.voxtype.daemon",
            login_item: "Voxtype"

  zap trash: [
    "~/.config/voxtype",
    "~/Library/Logs/voxtype",
  ]

  caveats <<~EOS
    Open Voxtype to get started:
      open /Applications/Voxtype.app

    Voxtype will automatically:
      - Download a speech model on first launch
      - Prompt for Microphone and Accessibility permissions

    Default hotkey: fn (Globe key)
    More info: voxtype --help
  EOS
end
