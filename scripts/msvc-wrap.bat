@echo off
:: Initialize MSVC environment using the latest Visual Studio installation found by vswhere.
:: Usage: msvc-wrap.bat <command> [args...]

for /f "usebackq tokens=*" %%i in (
  `"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -property installationPath`
) do (
  call "%%i\VC\Auxiliary\Build\vcvarsall.bat" arm64
)

%*
