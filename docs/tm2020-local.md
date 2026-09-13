# TM2020 as a local oracle (Ubuntu)

`deploy/tm2020-local.sh` walks through this and prints the steps it cannot do itself
(anything needing root or the Steam GUI). Layout on Ubuntu's `steam-installer` package:
Steam root is `~/.steam/root` (→ `~/.steam/debian-installation`), the game lands in
`<library>/steamapps/common/Trackmania`, its Wine prefix in
`<library>/steamapps/compatdata/2225070/pfx`.

1. `sudo apt install steam-installer protontricks vulkan-tools` (Mesa Vulkan drivers are
   already part of a desktop install; check with `vulkaninfo --summary`).
2. Launch `steam` once, log in. Settings → Compatibility → enable Steam Play for all titles.
3. GE-Proton: the script downloads the latest x86_64 release into
   `~/.steam/root/compatibilitytools.d/` and verifies the SHA-512. Restart Steam afterwards.
4. Install Trackmania (free, app id 2225070): `steam steam://install/2225070`. Right-click →
   Properties → Compatibility → force GE-Proton. First launch installs Ubisoft Connect inside
   the prefix; log into Ubisoft, close, launch again. If Ubisoft Connect hangs, delete
   `compatdata/2225070` and retry with Proton Experimental.
5. Openplanet: the script downloads `OpenplanetNext_<version>.exe` to `~/Downloads/rti-setup/`.
   Run it inside the game's prefix: `protontricks-launch --appid 2225070 <exe>` and point the
   installer at the Trackmania folder. In game, F3 opens the overlay. Plugins live in
   `pfx/drive_c/users/steamuser/OpenplanetNext/Plugins`.
6. Milestone zero: drive one imported TMX map by hand and read the car position from an
   Openplanet script. Then write the bridge plugin against `docs/oracle.md` (the mock server in
   `crates/rti-oracle/tests/oracle.rs` is the reference), set `[oracle] kind = "tm2020"`, and run
   `rti oracle`, then `rti verify <trajectory>`.

The Radeon iGPU is enough for verification at low settings. Video rendering through the game
belongs on the GPU pod (`docs/oracle-pod.md`).
