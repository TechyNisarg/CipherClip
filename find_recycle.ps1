$shell = New-Object -ComObject Shell.Application
$recycleBin = $shell.Namespace(0xA)
foreach ($item in $recycleBin.Items()) {
    if ($item.Name -eq 'android') {
        Write-Output $item.Path
    }
}
