@echo off
node ../scripts/pack-local.mjs --config json/lcjm.json --target electron-all
if errorlevel 1 pause
