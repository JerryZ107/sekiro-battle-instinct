install *args:
    cargo build {{args}}
    cp "./target/debug/sekiro_battle_instinct.dll" "C:/Program Files (x86)/Steam/steamapps/common/Sekiro/dinput8.dll"

logs:
    tail -f "C:/Program Files (x86)/Steam/steamapps/common/Sekiro/battle_instinct.log"

# 重新生成一键脚本：dist/zh/纸人挂.bat（中文界面）+ dist/en/paperman.bat（英文界面）。
# Regenerate both launch scripts (embeds tools/patch_paper_params.ps1 into each bat).
bat:
    powershell -NoProfile -ExecutionPolicy Bypass -File ./tools/make_bat.ps1

# 刷新 dist/zh 与 dist/en：DLL、cfg、纸人挂、ModEngine 和 param 整包。
# Refresh dist/zh and dist/en with the full player bundle.
dist: bat
    cargo build --release
    mkdir -p "./dist/zh/mods/param/gameparam" "./dist/en/mods/param/gameparam"
    cp "./target/release/sekiro_battle_instinct.dll" "./dist/zh/dinput8.dll"
    cp "./target/release/sekiro_battle_instinct.dll" "./dist/en/dinput8.dll"
    cp -f "./res/battle_instinct_zh.cfg" "./dist/zh/battle_instinct.cfg"
    cp -f "./res/battle_instinct.cfg" "./dist/en/battle_instinct.cfg"
    cp -f "./third_party/modengine/dinput8_modengine.dll" "./dist/zh/dinput8_modengine.dll"
    cp -f "./third_party/modengine/dinput8_modengine.dll" "./dist/en/dinput8_modengine.dll"
    cp -f "./third_party/modengine/modengine.ini" "./dist/zh/modengine.ini"
    cp -f "./third_party/modengine/modengine.ini" "./dist/en/modengine.ini"
    cp -f "./third_party/gameparam/gameparam.parambnd.dcx" "./dist/zh/mods/param/gameparam/gameparam.parambnd.dcx"
    cp -f "./third_party/gameparam/gameparam.parambnd.dcx" "./dist/en/mods/param/gameparam/gameparam.parambnd.dcx"

# 打两个发布 zip：中文包用 纸人挂.bat，英文包用 paperman.bat。
pack: dist
    rm -rf "./tmp"
    mkdir -p "./tmp/zh" "./tmp/en"
    cp -r "./dist/zh/." "./tmp/zh/"
    cp -r "./dist/en/." "./tmp/en/"
    powershell -NoProfile -Command "Compress-Archive -Path './tmp/zh/*' -DestinationPath './battle-instinct_zh.zip' -Force"
    powershell -NoProfile -Command "Compress-Archive -Path './tmp/en/*' -DestinationPath './battle-instinct_en.zip' -Force"
    rm -rf "./tmp"

release:
    just pack
    git tag -d nightly || true
    git push --delete origin nightly || true
    gh release create nightly "./battle-instinct_zh.zip" "./battle-instinct_en.zip" -t "Nightly Build" -n "Nightly Build"

# ---- 纸人 / 血纸人（Spirit Emblem）参数补丁 -----------------------------------------
# DLL 实时修正纸人上限；脚本写入初始纸人、临时纸人上限、纸人漂流数量与成长浮点。
# 详见 docs\纸人参数.md；写入前自动备份，可 params-revert 回滚。
# 界面语言见 tools/patch_paper_params.ps1 的 -Lang（或环境变量 PAPERDOLL_LANG=en|zh）。

params-dry:
    powershell -NoProfile -ExecutionPolicy Bypass -File ./tools/patch_paper_params.ps1

params:
    powershell -NoProfile -ExecutionPolicy Bypass -File ./tools/patch_paper_params.ps1 -Apply

params-revert:
    powershell -NoProfile -ExecutionPolicy Bypass -File ./tools/patch_paper_params.ps1 -Revert
