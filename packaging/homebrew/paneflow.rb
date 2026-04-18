# PaneFlow Homebrew cask — canonical template.
#
# US-017. This file is the SOURCE OF TRUTH that
# `.github/workflows/update-cask.yml` copies (and version-stamps) into
# the external tap repo `ArthurDEV44/homebrew-paneflow` on every
# release. The tap repo's `Casks/paneflow.rb` is a derived artifact; do
# not hand-edit it — commit changes here instead and let the workflow
# propagate on the next release.
#
# Operator setup (one-time):
#   1. Create the public repo https://github.com/ArthurDEV44/homebrew-paneflow
#   2. Initialise it with an empty `Casks/` directory (`git init && mkdir
#      Casks && git commit -m "bootstrap"`).
#   3. Create a fine-grained GitHub PAT scoped to that repo with
#      `contents: read-and-write`, add it to this repo's Actions secrets
#      as `HOMEBREW_TAP_TOKEN`.
#   4. The next `release: published` event on this repo will run
#      update-cask.yml and populate the tap's `Casks/paneflow.rb`.

cask "paneflow" do
  # `arch` maps Homebrew's Ruby-level `arch` helper (which already
  # distinguishes Apple Silicon vs Intel at `brew install` time) onto the
  # Rust target-triple tail PaneFlow uses in its .dmg filenames. The same
  # string is interpolated into `url` below via `#{arch}` — so there is a
  # single URL template, not two, and any typo is caught at cask-load
  # time by Homebrew's `brew style` linter.
  arch arm: "aarch64-apple-darwin", intel: "x86_64-apple-darwin"

  # These three lines are rewritten on every release by the CI workflow.
  # The placeholders keep the file syntactically valid (so `brew style`
  # passes in CI) and flag that a human-edited version is stale.
  version "0.0.0"
  sha256 arm:   "0000000000000000000000000000000000000000000000000000000000000000",
         intel: "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/ArthurDEV44/paneflow/releases/download/v#{version}/paneflow-#{version}-#{arch}.dmg",
      verified: "github.com/ArthurDEV44/paneflow/"

  name "PaneFlow"
  desc "Cross-platform terminal multiplexer built on GPUI"
  homepage "https://github.com/ArthurDEV44/paneflow"

  # `ventura` (macOS 13) is the floor because GPUI's macOS backend targets
  # that era. Same value as `LSMinimumSystemVersion` in assets/Info.plist
  # (US-013). Bumping this here without bumping the plist (or vice versa)
  # causes install-time confusion — keep them synchronised.
  depends_on macos: ">= :ventura"

  # Gatekeeper on macOS extracts the bundle from the DMG and installs it;
  # Homebrew handles mount/copy/unmount around this `app` stanza.
  app "PaneFlow.app"

  # `zap trash:` is Homebrew's opt-in deep-clean; `brew uninstall --zap`
  # moves these directories to the user's Trash. The paths match what
  # PaneFlow writes at runtime:
  #   ~/Library/Application Support/paneflow   → session.json, config.json
  #   ~/Library/Caches/paneflow                → scrollback, update-check cache
  # We intentionally do NOT zap ~/Library/Preferences/* — those may hold
  # Apple-system-managed state (window sizes, traffic-light geometry)
  # that shouldn't be nuked on an uninstall.
  zap trash: [
    "~/Library/Application Support/paneflow",
    "~/Library/Caches/paneflow",
  ]
end
