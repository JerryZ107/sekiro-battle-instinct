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

## Old saves: strict runtime rule (src/emblem.rs)

Param edits only change the starting value for **new saves**: the game writes the cap into the
save when a skill is learned, so a finished old save will not grow from a param edit. That part
is handled by `src/emblem.rs`:

- Every frame it reads the five learned-skill flags from the **live inventory**:
  `650000 / 650100 / 650200 / 650300 / 650400` (the `virtualWeaponId` of SkillParam
  75 / 76 / 175 / 176 / 603, i.e. 形代所持上限+1A…+1E).
- `i` = how many of those five items are present (0–5); the cap is **exactly `20 + 2 * i`**,
  written to `PlayerData+0x146` in the same frame.
- Learning a cap skill adds its inventory item, so the next frame `i` increases and the cap is
  rewritten immediately. Learning a non-cap skill does not change any of the five flags.
- There is no state file, no cfg learned-count seed, and no inference from the old cap value.
- If the live inventory is not ready (loading screen), it writes nothing and waits.
- Log line on change: `emblem: skills A=1 B=1 C=0 D=1 E=0 -> i=3 cap=26`.

The only DLL cfg key is the `paper cap fix` master switch; the cap offset and width are fixed constants.
The `paper initial cap` and `per skill bonus` lines are only read by the launcher bat for the
param patch; the DLL ignores them and always uses `20 + 2*i`.

## Runtime fields (located, measured 2026-09-27)

| Target | Location | Note |
| --- | --- | --- |
| Paper doll cap (the one the game uses) | `PlayerData + 0x146` (u16) | same value in the save = `sen(0x7C) + 0xCA`; this is the one that matters for old saves |
| Learned cap-skill flags (runtime) | `InventoryData + 0x10`, 16-byte entries | scan for item IDs `650000/650100/650200/650300/650400`; present = learned |
| Cap growth reference | `ResourceItemParam goodsId=1000` | the 15 -> 25 set; the real displayed value comes from the field above |

Relationship (strict): cap = **20 + 2 × i**, where i = number of learned items among the five
`virtualWeaponId`s above. Vanilla is `15 + 1×5 = 20`; this mod always uses the 20/2 rule.

## cfg knobs

```ini
paper cap fix: on        <- master switch (DLL writes nothing when off)
paper initial cap: 20    <- only for the launcher bat's param patch; DLL ignores it
per skill bonus: 2       <- only for the launcher bat's param patch; DLL ignores it
drift: 9                 <- blood/temporary doll cap and the amount granted per use (param side)
```

Technical parameters fixed in code:

| Parameter | Fixed value | Note |
| --- | --- | --- |
| cap offset / width | `0x146` / 2 (fixed constants) | the field actually used at runtime |
| cap rule | `20 + 2*i` | i from the five live inventory items |
| cap skill items | `650000/650100/650200/650300/650400` | SkillParam 75/76/175/176/603 |

The old `battle_instinct_emblem.state` fallback file is no longer read or written; delete it if
it exists.

## In-memory param patching (removed)

The DLL no longer tries to scan memory for the param rows and rewrite them; that experiment was
not viable:

- Private heap scan: all 6.7 GB scanned, zero anchor hits (param is not on the heap).
- File-mapping scan (`MEM_MAPPED` <= 64 MB): touches pages the game mmap-ed, causing heavy
  paging I/O -> **the game freezes**.
- Where the param actually lives: either on disk as `gameparam.parambnd.dcx` (DCX/DFLT
  compressed, and the DLL has no zlib) or inside a buffer the game decoded itself (unstable
  location).

Final split of responsibilities:

| Item | Handled by |
| --- | --- |
| Cap (incl. old-save fix), master switch | **DLL, live** (writes `PlayerData+0x146`, verified) |
| Drift amount/cap, starting dolls, growth floats | **param file**, written once by `tools/patch_paper_params.ps1` |

## Player install (full bundle)

The release includes ModEngine and the param file. Extract it into the Sekiro root folder with
the directory structure preserved:

```
dinput8.dll
battle_instinct.cfg
paperman.bat                  <- Chinese release: 纸人挂.bat
dinput8_modengine.dll         <- ModEngine's dinput8.dll, renamed
modengine.ini                 <- modOverrideDirectory="\mods"
mods\param\gameparam\gameparam.parambnd.dcx
```

Double-click the launcher once after installation or after editing the cfg. It reads the cfg and
writes the Drift values into the param package; fully exit and relaunch the game afterward.

- Switch: `paper cap fix: on/off` in the cfg (off = no DLL writes and no param writes)
- Roll back: `paperman.bat revert` (Chinese: `纸人挂.bat revert`)
- The cap itself is fixed at `20 + 2*i`; no cfg value changes it. `paper initial cap` /
  `per skill bonus` only affect the param patcher.
- Only changing `drift`: edit the cfg -> double-click the launcher -> restart the game

## Release shape (full bundle)

Release = `dinput8.dll` + `battle_instinct.cfg` + the launcher + `dinput8_modengine.dll` +
`modengine.ini` + `mods\param\gameparam\gameparam.parambnd.dcx`.

The launcher embeds `tools/patch_paper_params.ps1` **verbatim at the end of the file**. At
runtime it:

1. runs `chcp 936` and sets `PAPERDOLL_CFG=%~dp0battle_instinct.cfg` (plus `PAPERDOLL_LANG`,
   which selects the console language);
2. uses PowerShell to extract everything after the marker line into `%TEMP%\zhiren_*.ps1`;
3. runs `powershell -File <temp script> -Apply` (the script reads `PAPERDOLL_CFG` itself;
   `<launcher> revert` sets `PAPERDOLL_REVERT=1` to trigger the rollback);
4. deletes the temporary file.

Players never handle a `.ps1`; the source stays in `tools/patch_paper_params.ps1` - run
`just bat` (or `tools/make_bat.ps1`) to regenerate both launchers after editing it.

