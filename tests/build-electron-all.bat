@echo off
node ../scripts/pack-local.mjs --config json/osfipin.mark.json --target electron-all
if errorlevel 1 pause
