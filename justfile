install *args:
    cargo build {{args}}
    cp "./target/debug/sekiro_battle_instinct.dll" "C:/Program Files (x86)/Steam/steamapps/common/Sekiro/dinput8.dll"

logs:
    tail -f "C:/Program Files (x86)/Steam/steamapps/common/Sekiro/battle_instinct.log"

# Refresh dist/zh and dist/en (dinput8.dll + battle_instinct.cfg only).
dist:
    cargo build --release
    mkdir -p "./dist/zh" "./dist/en"
    cp "./target/release/sekiro_battle_instinct.dll" "./dist/zh/dinput8.dll"
    cp "./target/release/sekiro_battle_instinct.dll" "./dist/en/dinput8.dll"
    cp -f "./res/battle_instinct_zh.cfg" "./dist/zh/battle_instinct.cfg"
    cp -f "./res/battle_instinct.cfg" "./dist/en/battle_instinct.cfg"

pack: dist
    mkdir -p "./tmp"
    cp "./dist/zh/dinput8.dll" "./tmp/dinput8.dll"
    cp "./dist/zh/battle_instinct.cfg" "./tmp/battle_instinct.cfg"
    7z a -tzip -mx9 "./battle-instinct_zh.zip" "./tmp/dinput8.dll" "./tmp/battle_instinct.cfg"
    cp "./dist/en/dinput8.dll" "./tmp/dinput8.dll"
    cp "./dist/en/battle_instinct.cfg" "./tmp/battle_instinct.cfg"
    7z a -tzip -mx9 "./battle-instinct_en.zip" "./tmp/dinput8.dll" "./tmp/battle_instinct.cfg"
    rm -rf "./tmp"

release:
    just pack
    git tag -d nightly || true
    git push --delete origin nightly || true
    gh release create nightly "./battle-instinct_zh.zip" "./battle-instinct_en.zip" -t "Nightly Build" -n "Nightly Build"
