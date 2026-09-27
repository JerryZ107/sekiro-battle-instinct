<#
  make_bat.ps1 —— 生成 dist/{zh,en}/纸人挂.bat
  把 tools/patch_paper_params.ps1 原样嵌进 bat 尾部；bat 运行时用 more +N 自解到 %TEMP% 交给 PowerShell 执行。
  这样发布包只需 dinput8.dll + battle_instinct.cfg + 纸人挂.bat（无需任何 .ps1）。
  改完 tools/patch_paper_params.ps1 后跑一次本脚本即可重新生成。
#>
[CmdletBinding()]
param(
    [string]$Source = '',
    [string]$RepoRoot = ''
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $Source) { $Source = Join-Path $here 'patch_paper_params.ps1' }
if (-not $RepoRoot) { $RepoRoot = Split-Path -Parent $here }
if (-not (Test-Path $Source)) { throw "找不到源脚本: $Source" }
$psCode = ([System.IO.File]::ReadAllText($Source)) -replace "`r`n", "`n"

$h = @(
    '@echo off',
    'setlocal',
    'chcp 936 >nul',
    'title 只狼 · 纸人挂',
    'set "HERE=%~dp0"',
    'set "PAPERDOLL_CFG=%HERE%battle_instinct.cfg"',
    'if /i "%~1"=="revert" set "PAPERDOLL_REVERT=1"',
    'if /i "%~1"=="rollback" set "PAPERDOLL_REVERT=1"',
    'echo ==========================================================',
    'echo   只狼 · 纸人挂   （内置逻辑，无需其它脚本）',
    'echo ----------------------------------------------------------',
    'echo   读取同目录 battle_instinct.cfg 里的这几行：',
    'echo     纸人上限功能: 开 / 关',
    'echo     纸人初始上限: 25     技能增加上限: 5     纸人漂流: 9',
    'echo   「纸人上限功能: 关」= 什么都不写。',
    'echo   改完 cfg 双击本文件；回滚：纸人挂.bat revert',
    'echo ==========================================================',
    'echo.',
    'if not exist "%PAPERDOLL_CFG%" (',
    '  echo [错误] 同目录找不到 battle_instinct.cfg，请把它和本文件放在一起。',
    '  goto done',
    ')',
    'set "TMPPS=%TEMP%\zhiren_%RANDOM%%RANDOM%.ps1"',
    'powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-Content -LiteralPath ''%~f0'' -Encoding Default | Select-Object -Skip __SKIP__ | Set-Content -LiteralPath ''%TMPPS%'' -Encoding Default"',
    'if not exist "%TMPPS%" (',
    '  echo [错误] 解出临时脚本失败。',
    '  goto done',
    ')',
    'powershell -NoProfile -ExecutionPolicy Bypass -File "%TMPPS%" -Apply',
    'set "RC=%ERRORLEVEL%"',
    'del "%TMPPS%" >nul 2>nul',
    'if not "%RC%"=="0" echo [提示] 脚本退出码 %RC%',
    'goto done',
    ':done',
    'echo.',
    'echo 完成。按任意键退出。',
    'pause >nul',
    'endlocal',
    'exit /b',
    '',
    'rem ============ 以下为内嵌 PowerShell 代码（自解执行；改 tools/patch_paper_params.ps1 后跑 tools/make_bat.ps1 重新生成）============'
)
$header = $h -join "`n"
$skip = $h.Count               # Select-Object -Skip 需要「跳过多少行」
$header = $header.Replace('__SKIP__', [string]$skip)
$bat = ($header + "`n" + $psCode)
$bat = ($bat -replace "`r`n", "`n") -replace "`n", "`r`n"
$gbk = [System.Text.Encoding]::GetEncoding(936)
foreach ($dir in @('zh', 'en')) {
    $outPath = Join-Path $RepoRoot ("dist\{0}" -f $dir)
    if (-not (Test-Path $outPath)) { New-Item -ItemType Directory -Force -Path $outPath | Out-Null }
    $file = Join-Path $outPath '纸人挂.bat'
    [System.IO.File]::WriteAllText($file, $bat, $gbk)
    Write-Host ("生成 {0} ({1} bytes, 内嵌 PS 起于第 {2} 行)" -f $file, (Get-Item $file).Length, $skip)
}
