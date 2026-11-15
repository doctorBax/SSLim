param(
    [Parameter(Mandatory)][string]$Sdk,
    [Parameter(Mandatory)][string]$Jdk,
    [string]$SigningStore
)

$ErrorActionPreference = 'Stop'
$env:JAVA_HOME = (Resolve-Path -LiteralPath $Jdk).Path
$env:Path = "$env:JAVA_HOME\bin;$env:Path"
$sdkPath = (Resolve-Path -LiteralPath $Sdk).Path
$tools = Join-Path $sdkPath 'build-tools\35.0.0'
$platform = Join-Path $sdkPath 'platforms\android-35\android.jar'
$build = Join-Path $PSScriptRoot 'build'
$tmp = New-Item -ItemType Directory -Path (Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid()))
$keyStore = if ($SigningStore) { (Resolve-Path -LiteralPath $SigningStore).Path } else { Join-Path $build 'blade-signing.jks' }
$signPassword = if ($SigningStore) { 'env:BLADE_SIGN_PASS' } else { 'pass:android' }

function Run([string]$exe, [string[]]$arguments) {
    & $exe @arguments
    if ($LASTEXITCODE -ne 0) { throw "Build command failed: $exe ($LASTEXITCODE)" }
}

try {
    $classes = New-Item -ItemType Directory -Path (Join-Path $tmp.FullName 'classes')
    $unsigned = Join-Path $tmp.FullName 'unsigned.apk'
    $aligned = Join-Path $tmp.FullName 'aligned.apk'
    $generated = New-Item -ItemType Directory -Path (Join-Path $tmp.FullName 'generated')
    $resources = Join-Path $tmp.FullName 'resources.zip'
    $aapt = Join-Path $tools 'aapt2.exe'
    Run $aapt @('compile','--dir',(Join-Path $PSScriptRoot 'res'),'-o',$resources)
    Run $aapt @('link','-I',$platform,'--manifest',(Join-Path $PSScriptRoot 'AndroidManifest.xml'),'--java',$generated.FullName,'-o',$unsigned,$resources)
    $sources = @(Get-ChildItem @((Join-Path $PSScriptRoot 'src'), $generated.FullName) -Recurse -Filter '*.java' | ForEach-Object FullName)
    Run 'javac.exe' (@('-encoding','UTF-8','--release','8','-classpath',$platform,'-d',$classes.FullName) + $sources)
    $classFiles = @(Get-ChildItem $classes.FullName -Recurse -Filter '*.class' | ForEach-Object FullName)
    Run (Join-Path $tools 'd8.bat') (@('--lib',$platform,'--min-api','29','--output',$tmp.FullName) + $classFiles)
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::Open($unsigned, [IO.Compression.ZipArchiveMode]::Update)
    try {
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive,(Join-Path $tmp.FullName 'classes.dex'),'classes.dex') | Out-Null
    } finally { $archive.Dispose() }
    Run (Join-Path $tools 'zipalign.exe') @('-f','4',$unsigned,$aligned)
    New-Item -ItemType Directory -Path $build -Force | Out-Null
    if (!(Test-Path -LiteralPath $keyStore)) {
        Run 'keytool.exe' @('-genkeypair','-keystore',$keyStore,'-storepass','android','-keypass','android','-alias','blade','-dname','CN=Blade debug','-keyalg','RSA','-validity','3650')
    }
    $apk = Join-Path $build 'Blade.apk'
    Run (Join-Path $tools 'apksigner.bat') @('sign','--ks',$keyStore,'--ks-key-alias','blade','--ks-pass',$signPassword,'--out',$apk,$aligned)
    Run (Join-Path $tools 'apksigner.bat') @('verify',$apk)
    Get-Item -LiteralPath $apk
} finally {
    if ($tmp.Parent.FullName -eq (Get-Item -LiteralPath ([IO.Path]::GetTempPath())).FullName) {
        Remove-Item -LiteralPath $tmp.FullName -Recurse -Force
    }
}
