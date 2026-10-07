@echo off
node ../scripts/pack-local.mjs --config json/osfipin.note.json --target electron-all
if errorlevel 1 pause
