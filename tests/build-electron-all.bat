@echo off
node ../scripts/pack-local.mjs --config osfipin.note.json --target electron-all
if errorlevel 1 pause
