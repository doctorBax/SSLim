param()
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$scriptPath = $PSCommandPath
$exe = Join-Path $PSScriptRoot 'sslim.exe'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    try {
        Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $scriptPath + '"')) | Out-Null
    } catch {
        [Windows.Forms.MessageBox]::Show('Для запуска Blade нужны права администратора.', 'Blade') | Out-Null
    }
    exit
}
$created = $false
$mutex = New-Object Threading.Mutex($true, 'Local\SSLimBladeTray', [ref]$created)
if (-not $created) { $mutex.Dispose(); exit }

$script:child = $null
$script:stopPath = $null
$script:selected = $null
$script:stopping = $false
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runName = 'SSLim Blade Tray'
$icon = New-Object Windows.Forms.NotifyIcon
$icon.Icon = [Drawing.SystemIcons]::Application
$icon.Text = 'Blade: клиент остановлен'
$icon.Visible = $true
$menu = New-Object Windows.Forms.ContextMenuStrip
$status = $menu.Items.Add('Клиент остановлен')
$status.Enabled = $false
$profiles = New-Object Windows.Forms.ToolStripMenuItem('Выбрать профиль')
[void]$menu.Items.Add($profiles)
$connect = $menu.Items.Add('Запустить клиент')
$disconnect = $menu.Items.Add('Остановить клиент')
$disconnect.Enabled = $false
$console = $menu.Items.Add('Открыть консоль (импорт профиля)')
$startup = New-Object Windows.Forms.ToolStripMenuItem('Автозапуск трея')
[void]$menu.Items.Add($startup)
[void]$menu.Items.Add('-')
$quit = $menu.Items.Add('Выход')
$icon.ContextMenuStrip = $menu

function Refresh-Profiles {
    $profiles.DropDownItems.Clear()
    $folder = Join-Path $env:APPDATA 'sslim\profiles'
    $files = @(Get-ChildItem -LiteralPath $folder -Filter '*.toml' -File -ErrorAction SilentlyContinue | Sort-Object Name)
    foreach ($file in $files) {
        $name = $file.BaseName
        if ($name -notmatch '^[A-Za-z0-9._-]+$') { continue }
        $item = New-Object Windows.Forms.ToolStripMenuItem($name)
        $item.Checked = ($name -eq $script:selected)
        $item.Add_Click({ $script:selected = $this.Text; Refresh-Profiles })
        [void]$profiles.DropDownItems.Add($item)
    }
    if ($profiles.DropDownItems.Count -eq 0) {
        $empty = $profiles.DropDownItems.Add('Нет профилей; импортируйте через консоль')
        $empty.Enabled = $false
    }
}
function Refresh-Startup {
    $entry = Get-ItemProperty -LiteralPath $runKey -Name $runName -ErrorAction SilentlyContinue
    $startup.Checked = $null -ne $entry
}
function Show-State([string]$text) {
    $status.Text = $text
    $icon.Text = 'Blade: ' + $text
}
$menu.Add_Opening({ Refresh-Profiles; Refresh-Startup })
$console.Add_Click({
    try { Start-Process -FilePath $exe -WorkingDirectory $PSScriptRoot | Out-Null }
    catch { [Windows.Forms.MessageBox]::Show('Не удалось открыть консоль Blade.', 'Blade') | Out-Null }
})
$connect.Add_Click({
    if ($script:child) { return }
    if (-not $script:selected) {
        [Windows.Forms.MessageBox]::Show('Сначала выберите профиль.', 'Blade') | Out-Null
        return
    }
    try {
        $directory = Join-Path $env:LOCALAPPDATA 'sslim\tray-control'
        [void](New-Item -ItemType Directory -Path $directory -Force)
        $script:stopPath = Join-Path $directory ([Guid]::NewGuid().ToString('N') + '.stop')
        $script:child = Start-Process -FilePath $exe -WorkingDirectory $PSScriptRoot -WindowStyle Hidden -PassThru -ArgumentList @('blade', '--profile', $script:selected, '--stop-file', ('"' + $script:stopPath + '"'))
        $connect.Enabled = $false
        $profiles.Enabled = $false
        $disconnect.Enabled = $true
        Show-State 'клиент запущен (состояние туннеля неизвестно)'
    } catch {
        $script:child = $null
        [Windows.Forms.MessageBox]::Show('Не удалось запустить клиент.', 'Blade') | Out-Null
    }
})
$disconnect.Add_Click({
    if (-not $script:child -or $script:stopping) { return }
    try {
        [IO.File]::WriteAllText($script:stopPath, '')
        $script:stopping = $true
        $disconnect.Enabled = $false
        Show-State 'остановка клиента'
    } catch { [Windows.Forms.MessageBox]::Show('Не удалось запросить остановку клиента.', 'Blade') | Out-Null }
})
$startup.Add_Click({
    try {
        if ($startup.Checked) {
            Remove-ItemProperty -LiteralPath $runKey -Name $runName -ErrorAction Stop
        } else {
            $value = 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "' + $scriptPath + '"'
            [void](New-Item -Path $runKey -Force)
            New-ItemProperty -LiteralPath $runKey -Name $runName -Value $value -PropertyType String -Force | Out-Null
            [Windows.Forms.MessageBox]::Show('При входе Windows запросит права администратора для трея.', 'Blade') | Out-Null
        }
        Refresh-Startup
    } catch { [Windows.Forms.MessageBox]::Show('Не удалось изменить автозапуск.', 'Blade') | Out-Null }
})
$quit.Add_Click({
    if ($script:child) {
        [Windows.Forms.MessageBox]::Show('Сначала остановите клиент и дождитесь завершения.', 'Blade') | Out-Null
        return
    }
    [Windows.Forms.Application]::Exit()
})
$timer = New-Object Windows.Forms.Timer
$timer.Interval = 1000
$timer.Add_Tick({
    if ($script:child -and $script:child.HasExited) {
        $code = $script:child.ExitCode
        $script:child.Dispose()
        $script:child = $null
        if ($script:stopPath) { Remove-Item -LiteralPath $script:stopPath -Force -ErrorAction SilentlyContinue }
        $script:stopPath = $null
        $wasStopping = $script:stopping
        $script:stopping = $false
        $connect.Enabled = $true
        $profiles.Enabled = $true
        $disconnect.Enabled = $false
        if ($code -eq 0 -and $wasStopping) { Show-State 'клиент остановлен' }
        else { Show-State 'клиент завершился'; $icon.ShowBalloonTip(5000, 'Blade', 'Клиент завершился. Состояние туннеля неизвестно.', [Windows.Forms.ToolTipIcon]::Warning) }
    }
})
try {
    $timer.Start()
    [Windows.Forms.Application]::Run()
} finally {
    $timer.Stop(); $timer.Dispose()
    $icon.Visible = $false; $icon.Dispose(); $menu.Dispose()
    $mutex.ReleaseMutex(); $mutex.Dispose()
}
