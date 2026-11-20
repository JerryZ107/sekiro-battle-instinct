# Battle Instinct (two-key combat arts fork)

![demo](demo.gif)

[简体中文](README.md) ｜ **English**

A Sekiro combat-art / prosthetic MOD forked from [dec32/sekiro-battle-instinct](https://github.com/dec32/sekiro-battle-instinct). Upstream switches arts with **direction sequences + block/attack**. This fork keeps the original slot-swap / inject core, but **redesigns art combos and prosthetic triggers** (AI-assisted): arts use a **two-key** short window; prosthetics use **first key (move/block/attack/interact/grapple) + q/t**, or bare `t` / `q`, to equip and inject use.

## Features

This build contains four feature groups:

1. **Combat-art combos** — two-key short-window activation, queueing, and instant arts.
2. **Paper doll + blood paper doll** — live emblem-cap correction in the DLL and a param writer for Paper Doll Drift (temporary/blood emblems).
3. **Flow** — perfect-deflect stacks that mitigate or restore posture.
4. **Paper-doll damage reduction** — tiers based on raw damage / max HP; each hit spends emblems to reduce 60%, with temporary emblems spent first and downgrade rules when emblems are short.

## Quick start (recommended)

The release bundle already includes this MOD, ModEngine, and the param file required by Paper Doll Drift. Extract it **with the directory structure preserved** into the folder containing `sekiro.exe`:

```text
Sekiro\
  dinput8.dll
  battle_instinct.cfg
  paperman.bat            (Chinese release: 纸人挂.bat)
  dinput8_modengine.dll
  modengine.ini
  mods\param\gameparam\gameparam.parambnd.dcx
```

For a fresh install, or after editing `battle_instinct.cfg`:

1. Double-click `paperman.bat` (Chinese release: `纸人挂.bat`) and wait for it to finish.
2. Fully exit the game, then launch it again.
3. The DLL fixes the spirit-emblem cap live; the temporary-emblem cap and Paper Doll Drift amount come from the param file written by the launcher.

> Do not change the `mods\param\gameparam\gameparam.parambnd.dcx` path. It is the loose param override loaded by ModEngine and is required for Drift changes. Without ModEngine, the DLL's live emblem-cap fix still works, but param values such as the Drift amount remain vanilla.

### English release

- [dist/en/dinput8.dll](dist/en/dinput8.dll)
- [dist/en/battle_instinct.cfg](dist/en/battle_instinct.cfg)
- [dist/en/paperman.bat](dist/en/paperman.bat)
- [dist/en/dinput8_modengine.dll](dist/en/dinput8_modengine.dll)
- [dist/en/modengine.ini](dist/en/modengine.ini)
- [dist/en/mods/param/gameparam/gameparam.parambnd.dcx](dist/en/mods/param/gameparam/gameparam.parambnd.dcx)

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
| `f` | Grapple / hook |
| `↑↓←→` | Move (WASD / stick) |
| `ee` | Double-tap interact |
| `rl` / `re` / `er` / `el` / `rf` / `fr` … | Ordered two-key pairs; one line can hold several combos separated by `/`, e.g. `↓l/l↓` |

`e`=interact and `f`=grapple both follow the in-game action bits, so keyboard remaps/gamepad work.

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
| Ichimonji | `ff` |
| Shadowrush | `fl` |

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
| Phoenix's Lilac Umbrella | `↓q-0s` |
| Mountain Echo | `et-0s` |
| Leaping Flame | `↑q-0.67s` |
| Spiral Spear | `et-0.67s` |
| Sparking Axe | `↓t-0.5s` |
| Lazulite Sabimaru | `↑t-0.5s` |
| Okinaga's Flame Vent | `←t-0.5s` |
| Long Spark | `→t-0s` |
| Finger Whistle / Divine Abduction bind | `eq-0s` |

## Paper Doll launcher (纸人挂.bat / paperman.bat)

The launcher is a one-shot param writer; it does not stay running. It reads these values from `battle_instinct.cfg`:

```ini
paper cap fix: on
paper initial cap: 20
per skill bonus: 2
drift: 9
```

It then writes the ModEngine-loaded `gameparam.parambnd.dcx`:

| Field | Purpose |
| --- | --- |
| `EquipParamGoods 1000 +0x36` | Base paper-doll cap |
| `EquipParamGoods 1001 +0x36` | Blood / temporary paper-doll cap |
| `EquipParamGoods 3800 +0x8C` | Amount granted per Paper Doll Drift use |
| `ResourceItemParam goodsId=1000` | Starting dolls and cap-growth floats |

### First install / changing values

1. Confirm `dinput8_modengine.dll`, `modengine.ini`, and `mods\param\gameparam\gameparam.parambnd.dcx` from the bundle are in the correct game-root locations.
2. Edit `drift` in `battle_instinct.cfg` if needed. It controls both the temporary-emblem cap and the amount granted per use.
3. Double-click `paperman.bat` (Chinese: `纸人挂.bat`) and wait for it to finish. It creates a timestamped param backup automatically.
4. Fully exit and relaunch the game; params are only read at startup.
5. Use Paper Doll Drift in game and verify the temporary-emblem cap / gain.

### Rollback

Run:

```bat
paperman.bat revert
```

This restores the latest param backup; restart the game afterward. To disable all paper-doll correction, set `paper cap fix: off`; this does not automatically roll back an already-written param file.

### Troubleshooting

- **Drift has no effect**: verify `dinput8_modengine.dll` is loaded and the `mods\param\gameparam\gameparam.parambnd.dcx` path is correct, then rerun the launcher and restart the game.
- **ModEngine conflict**: this MOD chain-loads `dinput8_modengine.dll`; keep this MOD as `dinput8.dll` and do not overwrite it with ModEngine's DLL.
- **Cap vs. Drift**: the spirit-emblem cap is fixed live by the DLL as `20 + 2 * i`; only the Drift amount and temporary-emblem cap come from the param file.
- Field and reverse-engineering notes: [docs/paper-doll-params.en.md](docs/paper-doll-params.en.md).

## Build yourself (optional)

```bash
cargo build --release
just dist     # refresh dist/zh and dist/en (DLL, cfg, launcher, ModEngine and param bundle)
just bat      # regenerate 纸人挂.bat / paperman.bat only
just pack     # build battle-instinct_zh.zip / battle-instinct_en.zip
```

Output: `target/release/`; release folders `dist/zh/` (Chinese cfg) and `dist/en/` (English cfg).
After extracting a release, double-click the launcher once (`纸人挂.bat` for Chinese, `paperman.bat` for English).

### ModEngine / param bundle

The release includes:

```text
dinput8_modengine.dll    <- ModEngine's dinput8.dll, renamed
modengine.ini            <- modOverrideDirectory="\mods"
mods\param\gameparam\gameparam.parambnd.dcx
```

`modengine.ini` keeps `modOverrideDirectory="\mods"`. The param file is taken from the local Sekiro: Resurrection package and is included only to provide the Drift fields. If you use another major mod, replace it with that mod's `gameparam.parambnd.dcx` and rerun the launcher.


## Config toggles (for the paper / flow keys the leading `#` is optional; timing keys are still written as `# key: value`)

| Key | Meaning |
| --- | --- |
| `# boot console: off` | Show the load-info console on startup (off by default) |
| `# rl window: 0.3s` | **`rl` (Sakura Dance)**: press `l` within this time after `r`, with both held when `l` registers; real B+A is still stripped |
| `# tool trigger window: 0.3s` | Two-key prosthetic: first key → `q`/`t` must land within this time (~0.3s) |
| `# art queue delay: 0.3s` | After the last key is released, wait this long before flushing a queued art; `0s` = flush immediately |
| `paper cap fix: on` | Master switch for the spirit-emblem cap fix (off = DLL writes nothing and the launcher bat writes no param) |
| `paper initial cap: 20` / `per skill bonus: 2` | **Only used by the launcher bat for the param file**; the DLL runtime cap is always `20 + 2 * i` and ignores these two values |
| `drift: 9` | Blood/temporary emblem cap, and the amount granted per use of the Paper Doll Drift item |
| `paper guard: on` | Paper-doll damage reduction by raw-damage / max-HP tier; temporary emblems are spent first. See [docs/纸人减伤.md](docs/纸人减伤.md) |
| `paper guard debug: off` | Log tier, raw/reduced damage, actual HP loss, and emblem changes for each hit |
| `flow: on` | Perfect-deflect stack flow master switch; `flow duration` / `flow max stacks` / `flow refund ratios` are documented in [docs/心流.md](docs/心流.md) |

> Both releases use `20 + 2 * i` at runtime; `paper initial cap` / `per skill bonus` only affect the param patcher.

A prosthetic bind can append `-time` for multi-hit lock, e.g. `↑q-0.5s` or `↑q-multi-hit0.5s`; default 1s if omitted.


## Credits

- **[dec32](https://github.com/dec32)**: [Battle Instinct](https://github.com/dec32/sekiro-battle-instinct) original author — this MOD is based on their code and architecture
- **[Katalash](https://github.com/katalash)**: [ModEngine](https://github.com/katalash/ModEngine) original author — DLL and config are bundled in the release
- Param package taken from the local **Sekiro: Resurrection** installation, bundled only as the Paper Doll Drift param override
- [Tmsrise](https://github.com/tmsrise): [Sekiro Weapon Wheel](https://www.nexusmods.com/sekiro/mods/1058)
- [ReaperAnon](https://github.com/ReaperAnon): [Sekiro Hotkey System](https://www.nexusmods.com/sekiro/mods/1648)
- [Yuzheng Wu](https://github.com/Persona-woo): input-feel testing and improvements on the original

See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for bundled third-party components.

## Disclaimer

Please follow the upstream license and Sekiro MOD conventions. This repo is a personal derivative focused on a different combo philosophy; report issues here, and do not bother the original author.
