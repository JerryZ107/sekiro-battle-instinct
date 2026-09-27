# Paper doll (spirit emblem) param fields and one-shot patcher

> Conclusion: the paper-doll cap, the starting doll count and the Paper Doll Drift amount
> are **not** things the DLL can change directly. Every similar mod (including the other
> mod pack on this machine) takes the ready-made route of editing the regulation param
> table: fixed fields inside `mods\param\gameparam\gameparam.parambnd.dcx`.
> This repo ships `tools\patch_paper_params.ps1` to patch exactly those fields
> (automatic backup, rollback supported; measured: only 9 bytes change).

## Field map (measured on Resurrection / the current pack)

| Target | param | row | offset | original | patched to |
| --- | --- | --- | --- | --- | --- |
| Paper doll cap | EquipParamGoods | 1000 paper doll | +0x36 (u16) | 15 | 30 |
| Temporary paper doll cap | EquipParamGoods | 1001 temporary doll | +0x36 (u16) | 5 | 15 |
| Paper Doll Drift amount | EquipParamGoods | 3800 drift | +0x8C (u16) | 5 | 15 |
| Starting paper dolls | ResourceItemParam | row goodsId=1000 | +0x10 (u32) | 6 | 30 |
| Growth f1 | ResourceItemParam | row goodsId=1000 | +0x14 (f32) | 3.75 | base cap / 4 |
| Growth f2 | ResourceItemParam | row goodsId=1000 | +0x18 (f32) | 6.25 | max cap / 4 |
| Growth f3 | ResourceItemParam | row goodsId=1000 | +0x1C (f32) | 5.0 | number of cap skills |
| Growth f4 | ResourceItemParam | row goodsId=1000 | +0x20 (f32) | 25.0 | max cap |

Evidence (reproducible):

- The debug names of EquipParamGoods 1000 / 1001 are the white doll / refill-doll resource
  items, and 3800 is the "refill doll conversion" row, i.e. Paper Doll Drift. The values at
  +0x36 / +0x8C match the HUD cap and the per-use refill amount exactly.
- The other mod pack on this machine sets those three fields to 20 / 15 / 10 with every
  other byte identical, so this is the standard way emblem-cap mods work.

## Per cap skill +N

The five skills live in SkillParam, rows 75 / 76 / 175 / 176 / 603 (the "form cap" upgrades).

- The game grants +1 per skill by counting *learned* skills: a SkillParam row only carries a
  flag (bit 0x01000000 at +0x34 = this skill raises the cap). There is **no** amount field,
  so plain param edits cannot express "per skill +10".
- The only param-side entry point is the 4 floats in ResourceItemParam
  (+0x14/+0x18/+0x1C/+0x20). The vanilla pack values 3.75 / 6.25 / 5 / 25 are exactly
  15/4, 25/4, 5 skills, 25 max cap: base cap 15, max 25, 5 skills, so (25-15)/5 = 2 per
  skill, matching the "1-2" you observe in game.
- The tool therefore writes base cap 30 + per skill 10 x 5 = 80, i.e. 7.5 / 20 / 5 / 80.
  If the game follows that formula, learning all 5 skills gives a cap of 30 + 50 = 80.
- These are **inferred** fields: verify by watching the HUD cap in game. If it is wrong,
  roll back, or pass `-SkipGrowthFloats` to write only the certain fields.

## Usage

```powershell
# dry run (nothing written)
powershell -ExecutionPolicy Bypass -File tools\patch_paper_params.ps1

# apply (auto-backup to <file>.bak-<timestamp>)
powershell -ExecutionPolicy Bypass -File tools\patch_paper_params.ps1 -Apply

# certain fields only
powershell -ExecutionPolicy Bypass -File tools\patch_paper_params.ps1 -Apply -SkipGrowthFloats

# custom values
... -Apply -InitialPaper 30 -BaseCap 30 -PerSkillBonus 10 -TempCap 15 -DriftAmount 15

# roll back (restores the most recent backup)
powershell -ExecutionPolicy Bypass -File tools\patch_paper_params.ps1 -Revert

# console language: -Lang en | -Lang zh, or set PAPERDOLL_LANG=en|zh
```

Flow: expand DCX (DFLT/zlib) -> locate the EquipParamGoods / ResourceItemParam rows -> patch
the fields in the table -> repack to DCX -> re-read to verify.

Note: this is a global param change, unrelated to per-save state; if another mod overwrites
the same param file you have to run the tool again.

## Old saves: runtime fix (src/emblem.rs)

Param edits only change the starting value for **new saves**: the game writes the cap into the
save when a skill is learned, so a finished old save will not grow from a param edit. That part
is handled by `src/emblem.rs`:

- Probe (on by default in cfg): once a second it logs every PlayerData field equal to the HUD
  cap/count and every (id,count) record for items 1000/1001/1020/1021 in InventoryData to
  `battle_instinct.log` (prefix `emblem probe:`), along with "small integer that looks like a
  cap" candidates (`candidate` lines) and changed fields (`u32 a -> b` lines).
- Fix (needs the cap offset filled in *and* `paper cap fix: on`): every frame it writes the cap
  back as `base cap + per cap-skill bonus x learned skills`. The skill count can be configured
  or inferred from the current cap (30+10k first, then the old-save 15+2k); learning a new skill
  (a delta of 1-4 over the last written value) bumps it by 1.
- Without the offset filled in it **only probes and never writes memory**.

Cfg keys (end of `battle_instinct.cfg`, both the EN and ZH files): paper cap fix / paper probe /
paper initial cap / per skill bonus / cap skill count / learned cap skills / cap offset /
cap width / learned-skill offset / old base cap / old per-skill bonus / probe cap value /
probe count value / probe window.

Locating trick: set the probe cap value to the cap shown on the HUD and the probe count value
to your current doll count; the probe then prints the in-memory locations of those values
(`== HUD cap/count` lines) and you can locate them in one shot.

## Runtime fields (located, measured 2026-09-27)

| Target | Location | Note |
| --- | --- | --- |
| Paper doll cap (the one the game uses) | `PlayerData + 0x146` (u16) | same value in the save = `sen(0x7C) + 0xCA`; this is the one that matters for old saves |
| Paper doll cap (save copy, useless) | `PlayerData + 0x170` (u16) | previously mistaken for the cap; cfg restore offset/value writes 25 back |
| Cap growth reference | `ResourceItemParam goodsId=1000` | the 15 -> 25 set; the real displayed value comes from the field above |

Relationship (measured): cap = **base cap + per "form cap" skill bonus x learned skills**;
vanilla is `15 + 1x5 = 20`, this mod defaults to `25 + 5x5 = 50` (both configurable in cfg).

## cfg knobs

```ini
# paper initial cap: 25    <- cap = this + per skill bonus x learned "form cap" skills (DLL, live; restart the game)
# per skill bonus: 5
# drift: 9                 <- blood/temporary doll cap and the amount granted per use (param side; run the tools script)
```

Technical parameters are fixed in code (normally no need to touch them, but the same-named cfg
comment still overrides them):

| Parameter | Fixed value | Note |
| --- | --- | --- |
| paper cap fix | on | on by default even if the line is absent |
| cap offset / width | `0x146` / 2 | the field actually used at runtime |
| restore offset / value | `0x170` / 25 | one-time fix for the field written by mistake earlier |
| cap skill count | 5 | total number of "form cap" skills |
| old base cap / old per-skill | 15 / 1 | used to infer old saves (vanilla 15+1xn) |
| probe / probe window | off / 0x1000 | only enable when re-locating fields |

Also: `battle_instinct_emblem.state` (auto-generated in the game folder) records the learned
cap-skill count, so changing `paper initial cap` / `per skill bonus` will not recompute it
wrongly; delete the file to re-infer from the current cap.

## Important: in-memory param patching is experimental and off by default (2026-09-27)

The DLL used to try scanning memory for the param rows and rewriting them. It is **not viable**:

- Private heap scan: all 6.7 GB scanned, zero anchor hits (param is not on the heap).
- File-mapping scan (`MEM_MAPPED` <= 64 MB): touches pages the game mmap-ed, causing heavy
  paging I/O -> **the game freezes**.
- Where the param actually lives: either on disk as `gameparam.parambnd.dcx` (DCX/DFLT
  compressed, and the DLL has no zlib) or inside a buffer the game decoded itself (unstable
  location).

Final split of responsibilities:

| Item | Handled by |
| --- | --- |
| Cap (incl. old-save fix), per-skill bonus, master switch | **DLL, live** (writes `PlayerData+0x146`, verified) |
| Drift amount/cap, starting dolls, growth floats | **param file**, written once by `tools/patch_paper_params.ps1` |

The experimental in-memory param switch still exists in the cfg, **off by default** - do not
enable it.

## Player install (3 files)

```
dinput8.dll
battle_instinct.cfg
paperman.bat      <- edit the cfg, then double-click this (Chinese release: the ZH-named bat)
```

Drop the three files into the Sekiro folder (next to `sekiro.exe`) and double-click the launcher
once.

- Switch: `paper cap fix: on/off` in the cfg (off = no param is written)
- Roll back: `paperman.bat revert`
- Only changing `paper initial cap` / `per skill bonus`: edit the cfg -> restart the game
  (DLL, live; no need to run the bat)
- Only changing `drift`: edit the cfg -> double-click `paperman.bat`

## Release shape (since 2026-09-27: 3 files)

Release = `dinput8.dll` + `battle_instinct.cfg` + the launcher (`paperman.bat` for the English
cfg, the ZH-named bat for the Chinese cfg).

The launcher embeds `tools/patch_paper_params.ps1` **verbatim at the end of the file**. At
runtime it:

1. runs `chcp 936` and sets `PAPERDOLL_CFG=%~dp0battle_instinct.cfg` (plus `PAPERDOLL_LANG`,
   which selects the console language);
2. uses PowerShell to extract everything after the marker line into `%TEMP%\zhiren_*.ps1`;
3. runs `powershell -File <temp script> -Apply` (the script reads `PAPERDOLL_CFG` itself;
   `<launcher> revert` sets `PAPERDOLL_REVERT=1` to trigger the rollback);
4. deletes the temporary file.

Upside: players never handle a `.ps1` and the package is three files; the source stays in
`tools/patch_paper_params.ps1` (open, readable) - run `just bat` (or `tools/make_bat.ps1`) to
regenerate both launchers after editing it.

