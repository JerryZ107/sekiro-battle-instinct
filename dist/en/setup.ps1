<#
  setup.ps1 —— 一键安装 / 卸载（玩家只需跑这一次）
  ------------------------------------------------------------------
  做的事：
    1. 找 Sekiro 目录（可 -GameDir 指定）
    2. 备份并放入 dinput8.dll + battle_instinct.cfg
    3. 调用同目录的 patch_paper_params.ps1，把 cfg 里的
       「纸人漂流 / 初始纸人 / 纸人初始上限」写进 param（可回滚）

  用法：
    安装：  powershell -ExecutionPolicy Bypass -File setup.ps1
    预览：  powershell -ExecutionPolicy Bypass -File setup.ps1 -DryRun
    指定目录： ... -GameDir "X:\Steam\steamapps\common\Sekiro"
    卸载：  powershell -ExecutionPolicy Bypass -File setup.ps1 -Uninstall
#>
[CmdletBinding()]
param(
    [string]$GameDir = '',
    [string]$Package = '',
    [switch]$DryRun,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
if (-not $Package) { $Package = Split-Path -Parent $MyInvocation.MyCommand.Path }
function Info($m) { Write-Host $m }

function Find-GameDir {
    $cands = @(
        'D:\game\steam\steamapps\common\Sekiro',
        'C:\Program Files (x86)\Steam\steamapps\common\Sekiro',
        'D:\Steam\steamapps\common\Sekiro',
        'E:\Steam\steamapps\common\Sekiro'
    )
    foreach ($k in @('HKLM:\SOFTWARE\WOW6432Node\Valve\Steam', 'HKLM:\SOFTWARE\Valve\Steam')) {
        try {
            $sp = (Get-ItemProperty -Path $k -ErrorAction Stop).InstallPath
            if ($sp) { $cands += (Join-Path $sp 'steamapps\common\Sekiro') }
        } catch { }
    }
    foreach ($c in $cands) { if ($c -and (Test-Path (Join-Path $c 'sekiro.exe'))) { return $c } }
    return ''
}

if (-not $GameDir) { $GameDir = Find-GameDir }
if (-not $GameDir -or -not (Test-Path (Join-Path $GameDir 'sekiro.exe'))) {
    throw "找不到 Sekiro 目录（sekiro.exe）。请用 -GameDir 指定，例如 -GameDir `"D:\Steam\steamapps\common\Sekiro`""
}
# 仓库内调试：如果同目录没有 dll，自动改用 ../dist/zh
if (-not (Test-Path (Join-Path $Package 'dinput8.dll'))) {
    $alt = Join-Path (Split-Path -Parent $Package) 'dist\zh'
    if (Test-Path (Join-Path $alt 'dinput8.dll')) { $Package = $alt }
}

$paramFile = Join-Path $GameDir 'mods\param\gameparam\gameparam.parambnd.dcx'
$cfgFile   = Join-Path $GameDir 'battle_instinct.cfg'
$dllFile   = Join-Path $GameDir 'dinput8.dll'
$patcher   = Join-Path $Package 'patch_paper_params.ps1'
$srcDll    = Join-Path $Package 'dinput8.dll'
$srcCfg    = Join-Path $Package 'battle_instinct.cfg'


if ($Uninstall) {
    if (Test-Path $dllFile) { if ($DryRun) { Info "[dry] 移除 $dllFile" } else { Remove-Item $dllFile -Force; Info "已移除 dinput8.dll" } }
    if (Test-Path $patcher -and (Test-Path $paramFile)) {
        Info '回滚 param ...'
        if (-not $DryRun) { & powershell -NoProfile -ExecutionPolicy Bypass -File $patcher -ParamFile $paramFile -Revert }
    }
    Info '卸载完成（cfg 保留，方便再看配置；要删可手动删 battle_instinct.cfg）'
    return
}

if (-not (Test-Path $srcDll)) { throw "安装包里没有 dinput8.dll：$srcDll" }
if (-not (Test-Path $srcCfg)) { throw "安装包里没有 battle_instinct.cfg：$srcCfg" }

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
if ($DryRun) {
    Info "[dry] 备份并写入 $dllFile"
    Info "[dry] 备份并写入 $cfgFile"
    Info "[dry] 调用 $patcher -Apply -ParamFile $paramFile -CfgFile $cfgFile"
    return
}
if (Test-Path $dllFile) { Copy-Item $dllFile "$dllFile.bak-$stamp" -Force }
if (Test-Path $cfgFile) { Copy-Item $cfgFile "$cfgFile.bak-$stamp" -Force }
Copy-Item $srcDll $dllFile -Force
Copy-Item $srcCfg $cfgFile -Force
foreach ($n in @('纸人挂.bat', 'patch_paper_params.ps1')) {
    $s = Join-Path $Package $n
    if (Test-Path $s) { Copy-Item $s (Join-Path $GameDir $n) -Force; Info "已写入 $n" }
}
Info "已写入 dinput8.dll / battle_instinct.cfg"

if (Test-Path $patcher -and (Test-Path $paramFile)) {
    Info '写入 param（纸人漂流 / 初始纸人 / 上限成长）...'
    & powershell -NoProfile -ExecutionPolicy Bypass -File $patcher -Apply -ParamFile $paramFile -CfgFile $cfgFile
} else {
    Info "（找不到 $patcher 或 param 文件，跳过 param 写入）"
}

Info ''
Info '安装完成 ✅  进游戏后：'
Info '  · 纸人上限 = 纸人初始上限 + 技能增加上限 × 已学「形代所持上限」技能数（老存档也生效）'
Info '  · 改 cfg 里的数值：前两项重进游戏即可；「纸人漂流」改完要再跑一次 setup.ps1'
Info "  · 回滚 param：powershell -File `"$patcher`" -Revert"
