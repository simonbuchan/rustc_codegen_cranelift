@echo off
echo [BUILD] build system >&2
mkdir build 2>nul
rustc build_system/main.rs -o build\y.exe -Cdebuginfo=1 --edition 2021
build\y.exe %*
