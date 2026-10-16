cask "oxidal" do
  arch arm: "apple-silicon", intel: "intel"

  version "0.7.0"
  sha256 arm:   "384c35f3ba4dbebbf1ef3064318fe8d5ea0f7aa9ef7774a0d05497bf689aac95",
         intel: "79003c05ca800e3ad9c029a97e8eb95fd76cfc438398e3dae350deb43b56db1a"

  url "https://github.com/sh4den/Oxidal/releases/download/v#{version}/Oxidal-#{version}-macos-#{arch}.dmg"
  name "Oxidal"
  desc "SSH, SFTP, telnet, serial and local shells in one window"
  homepage "https://github.com/sh4den/Oxidal"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  depends_on macos: ">= :big_sur"

  app "Oxidal.app"

  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/Oxidal.app"]
  end

  zap trash: [
    "~/Library/Application Support/Oxidal",
    "~/Library/Saved Application State/com.sh4den.oxidal.savedState",
  ]
end
