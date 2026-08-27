Typr v0.1.5 — an updater-etiquette release: the update prompt now appears exactly once per release and settings checks stay in the panel where they belong.

## Highlights

### The update banner shows once — then it is gone
On startup Typr quietly asks GitHub whether a newer build exists. When it finds one, the titlebar prompt ("Typr X is available." with **Update** and **Later**) appears exactly once. Press **Later** and that version never nags you again — not later that session, not after a restart. The preference is remembered per version, so refusing 0.1.4 does not silence 0.1.5 when it lands.

### "Check for updates" answers in the panel, not over your head
Clicking **Check for updates** in **General → Updates** used to have a quirk: if the background startup check was still in flight, a second check fired, and when the startup check finished it could pop the titlebar banner right over the panel you were looking at. Both checks now share a single request, a check you asked for is answered only as the panel's status line and **Download & install** button, and any banner clears the moment you press the button.

### Under the hood
* Serialized update checks — the startup check and the button check can no longer race each other or query GitHub twice.
* Release-manifest hygiene — `latest.json` is published without a byte-order mark, after a stray BOM briefly broke the 0.1.4 updater's response decoding.

## Install

- **`Typr_0.1.5_x64-setup.exe`** — NSIS installer (recommended)
- **`Typr_0.1.5_x64_en-US.msi`** — MSI installer

Windows 10/11, 64-bit. An NVIDIA GPU is optional — use Local Parakeet or Groq Cloud without one.

**On 0.1.4?** Typr will offer this update in its title bar — press **Update**. You can also install from **General → Updates**.

**On 0.1.3 or earlier?** Download the installer below and run it over your existing copy — settings, history, dictionary, and downloaded models are all preserved.

Windows SmartScreen will warn about an unrecognised publisher — choose **More info → Run anyway**.

**Full changelog:** https://github.com/sanirudh17/Typr/compare/v0.1.4...v0.1.5