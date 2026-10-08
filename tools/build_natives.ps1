# Builds extractor/natives/CUE4Parse-Natives.dll (ACL animation decompression for CUE4Parse; not published
# on NuGet). Needs git and the MSVC Build Tools (C++ workload, which includes CMake).
# Clones into a short path: the ACL submodule has paths longer than Windows' default limit.
param([string]$Work = "$env:TEMP\c4n")
$ErrorActionPreference = 'Stop'
$cmake = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
if (Test-Path $Work) { Remove-Item $Work -Recurse -Force }
git -c core.longpaths=true clone -q --depth 1 https://github.com/FabianFG/CUE4Parse.git "$Work\src"
git -C "$Work\src" -c core.longpaths=true submodule update --init --depth 1 --recursive CUE4Parse-Natives/ACL/external/acl
& $cmake -S "$Work\src\CUE4Parse-Natives" -B "$Work\build" -G "Visual Studio 17 2022" -A x64
& $cmake --build "$Work\build" --config Release
Copy-Item "$Work\build\Release\CUE4Parse-Natives.dll" "$PSScriptRoot\..\extractor\natives\" -Force
Write-Host "extractor\natives\CUE4Parse-Natives.dll updated"
