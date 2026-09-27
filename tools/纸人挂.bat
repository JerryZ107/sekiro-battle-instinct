@echo off
setlocal
cd /d "%~dp0"
title 只狼 · 纸人挂
echo ==========================================================
echo   只狼 · 纸人挂   （读取同目录 battle_instinct.cfg）
echo ----------------------------------------------------------
echo   生效的只有 cfg 里这几行：
echo     纸人上限功能: 开 / 关
echo     纸人初始上限: 25
echo     技能增加上限: 5
echo     纸人漂流: 9
echo   「纸人上限功能: 关」= 不写任何 param。
echo   用法：改完 cfg 双击本文件；回滚：纸人挂.bat revert
echo ==========================================================
echo.

if /i "%~1"=="revert" goto revert

if not exist "%~dp0patch_paper_params.ps1" (
  echo [错误] 同目录找不到 patch_paper_params.ps1
  echo        请把 纸人挂.bat / patch_paper_params.ps1 / battle_instinct.cfg 放在一起（通常在游戏目录）。
  goto done
)
if not exist "%~dp0battle_instinct.cfg" (
  echo [错误] 同目录找不到 battle_instinct.cfg
  goto done
)

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0patch_paper_params.ps1" -Apply -CfgFile "%~dp0battle_instinct.cfg"
goto done

:revert
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0patch_paper_params.ps1" -Revert

:done
echo.
echo 完成。按任意键退出。
pause >nul
endlocal