install *args:
    cargo build {{args}}
    cp "./target/debug/sekiro_battle_instinct.dll" "C:/Program Files (x86)/Steam/steamapps/common/Sekiro/dinput8.dll"

logs:
    tail -f "C:/Program Files (x86)/Steam/steamapps/common/Sekiro/battle_instinct.log"

# 重新生成一键脚本：dist/zh/纸人挂.bat（中文界面）+ dist/en/paperman.bat（英文界面）。
# Regenerate both launch scripts (embeds tools/patch_paper_params.ps1 into each bat).
bat:
    powershell -NoProfile -ExecutionPolicy Bypass -File ./tools/make_bat.ps1

# 刷新 dist/zh 与 dist/en：dinput8.dll + battle_instinct.cfg + 一键脚本。
# Refresh dist/zh (Chinese cfg) and dist/en (English cfg).
dist: bat
    cargo build --release
    mkdir -p "./dist/zh" "./dist/en"
    cp "./target/release/sekiro_battle_instinct.dll" "./dist/zh/dinput8.dll"
    cp "./target/release/sekiro_battle_instinct.dll" "./dist/en/dinput8.dll"
    cp -f "./res/battle_instinct_zh.cfg" "./dist/zh/battle_instinct.cfg"
    cp -f "./res/battle_instinct.cfg" "./dist/en/battle_instinct.cfg"

# 打两个发布 zip：中文包用 纸人挂.bat，英文包用 paperman.bat。
pack: dist
    mkdir -p "./tmp"
    cp "./dist/zh/dinput8.dll" "./tmp/dinput8.dll"
    cp "./dist/zh/battle_instinct.cfg" "./tmp/battle_instinct.cfg"
    cp "./dist/zh/纸人挂.bat" "./tmp/纸人挂.bat"
    7z a -tzip -mx9 "./battle-instinct_zh.zip" "./tmp/dinput8.dll" "./tmp/battle_instinct.cfg" "./tmp/纸人挂.bat"
    cp "./dist/en/dinput8.dll" "./tmp/dinput8.dll"
    cp "./dist/en/battle_instinct.cfg" "./tmp/battle_instinct.cfg"
    cp "./dist/en/paperman.bat" "./tmp/paperman.bat"
    7z a -tzip -mx9 "./battle-instinct_en.zip" "./tmp/dinput8.dll" "./tmp/battle_instinct.cfg" "./tmp/paperman.bat"
    rm -rf "./tmp"

release:
    just pack
    git tag -d nightly || true
    git push --delete origin nightly || true
    gh release create nightly "./battle-instinct_zh.zip" "./battle-instinct_en.zip" -t "Nightly Build" -n "Nightly Build"

# ---- 纸人（Spirit Emblem）参数补丁 -------------------------------------------------
# 初始纸人 30 / 纸人上限 30 / 临时纸人上限 15 / 纸人漂流 15 / 满级上限 80(每技能 10)
# 详见 docs\纸人参数.md；写入前自动备份，可 params-revert 回滚。
# 界面语言见 tools/patch_paper_params.ps1 的 -Lang（或环境变量 PAPERDOLL_LANG=en|zh）。

params-dry:
    powershell -NoProfile -ExecutionPolicy Bypass -File ./tools/patch_paper_params.ps1

params:
    powershell -NoProfile -ExecutionPolicy Bypass -File ./tools/patch_paper_params.ps1 -Apply

params-revert:
    powershell -NoProfile -ExecutionPolicy Bypass -File ./tools/patch_paper_params.ps1 -Revert

# ---- 玩家一键安装 / 卸载 ---------------------------------------------------------
# setup: 自动找 Sekiro 目录 → 备份并放 dll+cfg → 把 cfg 里的纸人数值写进 param
setup:
    powershell -NoProfile -ExecutionPolicy Bypass -File ./tools/setup.ps1

setup-dry:
    powershell -NoProfile -ExecutionPolicy Bypass -File ./tools/setup.ps1 -DryRun

uninstall:
    powershell -NoProfile -ExecutionPolicy Bypass -File ./tools/setup.ps1 -Uninstall