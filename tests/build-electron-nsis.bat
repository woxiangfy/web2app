@echo off
rem NSIS 安装版（*-setup.exe）。刻意与 build-electron-all.bat 分开：
rem 安装版必须带 distribution=nsis 标记（打进 app.asar），与 zip/portable 同次构建会把
rem 绿色产物一起误标成安装版（数据目录被强制到 %%APPDATA%%）。故必须单独跑这一次。
node ../scripts/pack-local.mjs --config json/osfipin.mark.json --target electron-nsis
if errorlevel 1 pause
