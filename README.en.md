# Battle Instinct (two-key combat arts fork)

![demo](demo.gif)

[简体中文](README.md) ｜ **English**

A Sekiro combat-art / prosthetic MOD forked from [dec32/sekiro-battle-instinct](https://github.com/dec32/sekiro-battle-instinct). Upstream switches arts with **direction sequences + block/attack**. This fork keeps the original slot-swap / inject core, but **redesigns art combos and prosthetic triggers** (AI-assisted): arts use a **two-key** short window; prosthetics use **first key (move/block/attack/interact) + q/t**, or bare `t` / `q`, to equip and inject use.

## Quick start (recommended)

**Only three files needed**: put `dinput8.dll`, `battle_instinct.cfg` and `纸人挂.bat` next to `sekiro.exe`, then **double-click `纸人挂.bat` once** (it has the param writer built in, no `.ps1` required). If another `dinput8.dll` exists (e.g. MOD Engine), rename **that** one to `dinput8_xxx.dll`; this MOD chain-loads it. **`version.dll` is not required.**

### English release

- [dist/en/dinput8.dll](dist/en/dinput8.dll)
- [dist/en/battle_instinct.cfg](dist/en/battle_instinct.cfg)

### Config toggles (comments at the end of cfg)

| Key | Meaning |
| --- | --- |
| `# boot console: off` | Show the load-info console on startup (off by default) |
| `# rl window: 0.3s` | **`rl` (Sakura Dance)**: press `l` within this time after `r`, with both held when `l` registers; real B+A is still stripped |
| `# tool trigger window: 0.3s` | Two-key prosthetic: first key → `q`/`t` must land within this time (~0.3s) |
| `# art queue delay: 0.3s` | After the last key is released, wait this long before flushing a queued art; `0s` = flush immediately |
| `# paper cap fix: on` | Master switch for the spirit-emblem cap fix (off = DLL writes nothing and `纸人挂.bat` writes no param) |
| `# paper initial cap: 25` | Cap = this value + `per skill bonus` x learned "form cap" skills |
| `# per skill bonus: 5` | How much each "form cap" (形代所持上限) skill adds to the cap |
| `# drift: 9` | Blood/temporary emblem cap, and the amount granted per use of 纸人漂流 |

A prosthetic bind can append `-time` for multi-hit lock, e.g. `↑q-0.5s` or `↑q-multi-hit0.5s`; default 1s if omitted.

## Paper-doll (spirit emblem cap / 纸人漂流)

Sekiro stores the **spirit-emblem cap inside the save** (it is written when a skill is learned), so param edits only affect new saves. This MOD handles it in two layers:

| Item | Handled by | Note |
| --- | --- | --- |
| Emblem cap (incl. old saves), per-skill bonus, master switch | **DLL, live** | writes `PlayerData+0x146`, using `paper initial cap + per skill bonus x learned "form cap" skills` |
| 纸人漂流 amount/cap, starting emblems, growth floats | **param file** | written once by `patch_paper_params.ps1` (called by `纸人挂.bat`) |

Usage:

1. Edit the four lines at the end of `battle_instinct.cfg`
2. Changed `paper initial cap` / `per skill bonus` → **restart the game** (live, works on old saves too)
3. Changed `drift` (纸人漂流) → double-click **`纸人挂.bat`** in the same folder (it reads the cfg switch/values and writes the param)
4. Roll back with `纸人挂.bat revert`; disable everything with `# paper cap fix: off`

Field map and reverse-engineering notes: [docs/纸人参数.md](docs/纸人参数.md) (Chinese).

## Relation to upstream

- **Upstream**: [@dec32](https://github.com/dec32)'s [Battle Instinct](https://github.com/dec32/sekiro-battle-instinct)
- **This repo**: independently maintained (two-key arts + q/t prosthetics), **not** an official upstream branch

If you prefer the original feel, use the upstream release instead.

## Combat arts (two keys)

Keys follow in-game actions (remaps still work). On a two-key match: swap art slot → briefly suppress attack → inject block+attack; hold the second key to keep injecting attack (charge), release to stop. Combos input while an art is still injecting are queued and fire after release **plus** a cfg delay (`# art queue delay`, default ~0.3s; `0s` = flush immediately). Default detect window is about **0.3s**; about **0.7s** when the first key is `l` (e.g. `l↑`); **`rl` (Sakura Dance)** uses `# rl window` (default **0.3s**): press `l` within that time after `r`, with both held when `l` registers. Avoid `r`/`l` as the first key: block has high priority; attack easily thrusts. Sources: `res/battle_instinct.cfg` (EN), `res/battle_instinct_zh.cfg` (ZH); releases: `dist/en/`, `dist/zh/`.

| Symbol | Meaning |
| --- | --- |
| `r` (right mouse) | Block |
| `l` (left mouse) | Attack |
| `e` | Interact / hold to beckon |
| `↑↓←→` | Move (WASD / stick) |
| `ee` | Double-tap interact |
| `rl` / `re` / `er` / `el` … | Ordered two-key pairs; one line can hold several combos separated by `/`, e.g. `↓l/l↓` |

Default release binds (editable in cfg):

| Combat Art | Bind |
| --- | --- |
| Ichimonji: Double | `ee` |
| Shadowfall | `r↑` |
| Nightjar Slash | `↑l` |
| Nightjar Slash Reversal | `↓l` |
| Ashina Cross (instant) | `r↓` (hold block + press direction; cfg name contains "instant") |
| High Monk | `↑r` |
| Whirlwind Slash | `e↑` |
| Sakura Dance | `rl` |
| Floating Passage | `l↑` |
| Spiral Cloud Passage | `↑e` |
| Dragon Flash | `er` |
| One Mind | `el` |
| Empowered Mortal Draw | `re` |

## Prosthetics (q / t)

- `q` = in-game "Switch Prosthetic"; `t` = "Use Prosthetic"
- Bare `t`: unique **default** tool — press/hold to equip and inject use; other tools return to it afterward
- Bare `q`: optional second one-key tool with the **same fire style as `t`** (not the return-default target)

Because both Use (`t`) and Switch (`q`) have default tools, they cannot be first keys — only tails. Also avoid `r`/`l` as first keys: block has high priority; attack easily thrusts. Prefer `↑q` / `→t`; never write `q↑`. After releasing the tail key, about **1s** of lock by default (override per bind with `-time`, e.g. `↑q-0.5s`): cannot switch tools; pressing `t`/`q` again refreshes the lock and keeps injecting (multi-hit). After lock ends, about **1.4s** later it returns to the bare-`t` default.

Default release prosthetic binds:

| Prosthetic | Bind |
| --- | --- |
| Lazulite Shuriken | `t-0s` |
| Aged Feather Mist Raven | `q-0s` |
| Suzaku's Lotus Umbrella | `↓q-0s` |
| Phoenix's Lilac Umbrella | `↓t-0s` |
| Mountain Echo | `et-0s` |
| Leaping Flame | `↑q-0.67s` |
| Spiral Spear | `et-0.67s` |
| Sparking Axe | `←q-0.5s` |
| Lazulite Sabimaru | `↑t-0.5s` |
| Okinaga's Flame Vent | `←t-0.5s` |
| Long Spark | `→q-0s` |
| Finger Whistle / Divine Abduction bind | `eq-0s` |

## Build yourself (optional)

```bash
cargo build --release
just dist     # 刷新 dist/zh 与 dist/en（各含 dinput8.dll + battle_instinct.cfg + 纸人挂.bat）
just bat    # regenerate 纸人挂.bat only (embeds tools/patch_paper_params.ps1 into the bat)
just pack     # build battle-instinct_zh.zip / battle-instinct_en.zip (5 files each)
```

Output: `target/release/`; release folders `dist/zh/` (Chinese cfg) and `dist/en/` (English cfg).
Players only need `纸人挂.bat`; its source (PowerShell) lives in `tools/patch_paper_params.ps1` — run `just bat` to regenerate the bat.

## Credits

- **[dec32](https://github.com/dec32)**: [Battle Instinct](https://github.com/dec32/sekiro-battle-instinct) original author — this MOD is based on their code and architecture
- [Tmsrise](https://github.com/tmsrise): [Sekiro Weapon Wheel](https://www.nexusmods.com/sekiro/mods/1058)
- [ReaperAnon](https://github.com/ReaperAnon): [Sekiro Hotkey System](https://www.nexusmods.com/sekiro/mods/1648)
- [Yuzheng Wu](https://github.com/Persona-woo): input-feel testing and improvements on the original

## Disclaimer

Please follow the upstream license and Sekiro MOD conventions. This repo is a personal derivative focused on a different combo philosophy; report issues here, and do not bother the original author.
