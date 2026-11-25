<p align="center">
  <img src="docs/assets/banner.svg" alt="Neptune VI" width="100%">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Windows%2011-0b0a15?style=flat-square&logo=windows11&logoColor=white" alt="Windows 11">
  <img src="https://img.shields.io/badge/Ryzen%207%209800X3D-0b0a15?style=flat-square&logo=amd&logoColor=ed1c24" alt="Ryzen 7 9800X3D">
  <img src="https://img.shields.io/badge/RTX%205070%20Ti-0b0a15?style=flat-square&logo=nvidia&logoColor=76b900" alt="RTX 5070 Ti">
  <img src="https://img.shields.io/badge/GPL--3.0-0b0a15?style=flat-square&logo=gnu&logoColor=white" alt="GPL-3.0">
</p>

<p align="center">
  Runs the Xbox Series build of <b>GTA VI</b> on a Windows PC.<br>
  A port of <b>Neptune (VIctory Edit.)</b> by <b>Traffic</b> &amp; <b>imgone</b> for AMD Ryzen + NVIDIA.
</p>

<br>

## ◆ Why this port

Upstream froze the GPU right before the first gameplay scene on a Ryzen 7 9800X3D with an RTX 5070 Ti. The cause: the game's indirect draw arguments ran unchecked, and some of them hold garbage.

This port checks every such record on the CPU first. Sane records run as fast as before, broken ones go through the safe path. **The game now gets into gameplay.**

<br>

## ◆ Ryzen notes

| | |
|:--|:--|
| **Console instructions** | The Xbox CPU is a Zen 2. Instructions Intel lacks (`MONITORX`, `MWAITX`, `CLZERO`, `INSERTQ`) have to be emulated on Intel. A Ryzen has them in hardware. |
| **Indirect draws** | `VICTORY_INDIRECT_RAW=3` is the default: argument records are checked on the CPU before the GPU runs them. `-RawIndirect` brings back upstream's mode. |
| **32 GB RAM** | Upstream was tuned on 64 GB. `-Ram32` shrinks the list pool, the texture staging and the card copies. |

<br>

## ◆ Quick start

> [!IMPORTANT]
> No game files are included. You need your own copy of the game.

```powershell
# first launch: translates the shaders (~7 min), then starts the game
.\play-zm.ps1 -Game 'D:\GTA6\game_scarlett_master.exe' -SeriesX -Ram32 -Warm

# every launch after that
.\play-zm.ps1 -Game 'D:\GTA6\game_scarlett_master.exe' -SeriesX -Ram32
```

| Switch | |
|:--|:--|
| `-SeriesX` | 4K Series X path |
| `-Ram32` | profile for 32 GB RAM |
| `-Stats -Tag x` | log + memory curve to files |
| `-Dred` | names the shader when the GPU hangs |

**Windows 10** also works: run `cargo xtask agility` once (it fetches Microsoft's D3D12 Agility SDK, hash-checked) and keep the `D3D12` folder next to `victory.exe`. Windows 11 needs nothing.

**Controls** · F9 mouse look · WASD move · Enter / Backspace / X / Y as A / B / X / Y · F10 overlay · F12 screenshot

<br>

## ◆ Overlay &amp; screenshots

A small translucent performance panel by **zm0kie** sits in the top-left corner of the game window. It shows FPS and frame time, the high, low and average of the last 30 seconds, draws per frame, CPU and GPU load, RAM and VRAM, and your hardware.

| Key | |
|:--|:--|
| `F10` | show / hide the overlay |
| `F12` | save a screenshot to `Pictures\Neptune` (with the overlay in the picture while it is shown) |

`VICTORY_OVERLAY=0` turns it off completely.

<br>

## ◆ Under the hood

<details>
<summary><b>Where a frame goes</b> &nbsp;·&nbsp; open world, about 6,500 draws, 90 ms (upstream measurement)</summary>
<br>

| Part | Share |
|:--|--:|
| Render thread in the submit step (memory copies, heap sync, write watch, textures) | ~41 % |
| Render thread waiting for the game's recording threads | ~48 % |
| Game's own code on the render thread | ~11 % |
| GPU busy | 35 % |

The CPU side is the limit, not the GPU.

</details>

<details>
<summary><b>Switches worth knowing</b></summary>
<br>

| Switch | Default | Does |
|:--|:--|:--|
| `VICTORY_AMBIENT` | `0.6` | ambient light level in place of ray-traced light; `off` disables it |
| `VICTORY_PIPELINE_WAIT_MS` | `2500` | longest a frame waits for a shader still compiling |
| `VICTORY_LEVELS_ON_DEMAND` | `1` | textures take card memory only for the mip levels in use |
| `VICTORY_VRAM_IDLE_MS` | `15000,2500,1000` | how long an unused texture stays on the card |
| `VICTORY_HEAP_SYNC` | `1` | mirrors descriptor records the game copies as bytes |
| `VICTORY_INDIRECT_RAW` | `3` | `0` clamp all, `2` trust all, `3` check on the CPU |

Pass any of them with `-Env 'NAME=value'`. Changing the shader translator or its switches needs a new `-Warm`.

</details>

<br>

## ◆ Build

```powershell
cargo build --release -p victory
```

Needs Visual Studio Build Tools (MSVC + Windows SDK) and [rustup](https://rustup.rs). Put `target\release\victory.exe` next to `tools\play-zm.ps1`.

<br>

## ◆ Known limits

- Ray-traced light is not running yet. An ambient pass stands in, so some rooms and water look off.
- Frame rate dips while the world streams in.
- Tested on one PC: Ryzen 7 9800X3D · RTX 5070 Ti 16 GB · 32 GB RAM.

<br>

## ◆ Credits

**Traffic** &amp; **imgone** for Neptune, the base of this port.
Built on Marvisak/VIctory · fxceless &amp; myokex (base, levels on demand, heap sync; PRs by Jonirulah, Flyander, AscilCH) · soldsawn (shader translator) · Belobug &amp; ShakiraRED-dt (ambient pass) · ZSlayerHQ · JoshuuDev · skornuta · Trexies · the DexNom FidelityFX patch · domints (Windows 10 Agility SDK loader, fxceless/victory-fork#402). Full list in [`docs/NEPTUNE-README.txt`](docs/NEPTUNE-README.txt).
Port by **zm0kie**.

<br>

<p align="center">
  <sub>GPL-3.0-or-later · not affiliated with Rockstar Games, Take-Two Interactive or Microsoft</sub>
</p>
