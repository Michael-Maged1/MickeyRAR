@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvarsx86_amd64.bat"
if errorlevel 1 exit /b %errorlevel%
set RUSTFLAGS=-C debuginfo=0 -C link-arg=/DEBUG:NONE
"C:\Users\micha\.cargo\bin\cargo.exe" test
