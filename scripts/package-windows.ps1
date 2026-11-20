param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $root
try {
    if (-not $SkipBuild) {
        pnpm tauri build --bundles nsis
        if ($LASTEXITCODE -ne 0) { throw "Windows 打包失败（$LASTEXITCODE）" }
    }

    $config = Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    $version = $config.version
    $release = Join-Path $root 'releases'
    $folder = Join-Path $release "StoryForge-$version-win-x64-portable"
    $null = New-Item -ItemType Directory -Path $folder -Force
    $exe = Join-Path $root 'src-tauri/target/release/storyforge.exe'
    $installer = Join-Path $root "src-tauri/target/release/bundle/nsis/StoryForge_${version}_x64-setup.exe"
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw '找不到已构建程序' }
    if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) { throw '找不到已构建安装包' }

    $portableExe = Join-Path $folder 'StoryForge.exe'
    $portableZip = "$folder.zip"
    $installerCopy = Join-Path $release "StoryForge-$version-win-x64-setup.exe"
    Copy-Item -LiteralPath $exe -Destination $portableExe -Force
    Copy-Item -LiteralPath $installer -Destination $installerCopy -Force
    Set-Content -LiteralPath (Join-Path $folder 'portable.flag') -Value 'StoryForge portable' -Encoding ascii
    Set-Content -LiteralPath (Join-Path $folder '使用说明.txt') -Encoding UTF8 -Value @'
将文件夹解压到可写入位置，双击 StoryForge.exe。需要 Microsoft Edge WebView2 Runtime。
模型档案保存在本目录的 settings，API 密钥保存在 Windows 凭据管理器。迁移到其他电脑后需重新在系统凭据窗口设置密钥。
通过「新建小说」或「打开工程」，选择本机 .story 文件。
WebView2 官方下载 https://developer.microsoft.com/microsoft-edge/webview2/
'@
    $contents = @($portableExe, (Join-Path $folder 'portable.flag'), (Join-Path $folder '使用说明.txt'))
    Compress-Archive -LiteralPath $contents -DestinationPath $portableZip -Force
    $checksums = foreach ($item in @($installerCopy, $portableZip)) {
        $hash = Get-FileHash -LiteralPath $item -Algorithm SHA256
        "$($hash.Hash.ToLowerInvariant())  $(Split-Path -Leaf $item)"
    }
    $checksums | Set-Content -LiteralPath (Join-Path $release 'SHA256SUMS.txt') -Encoding ascii
    Get-Item -LiteralPath $installerCopy, $portableZip | Select-Object FullName, Length
} finally {
    Pop-Location
}
