$bios = "C:\Users\gbran\OneDrive\Documents\spikeling-os\target\debug\build\spikeling-os\0c5ef03cbb2cbbbb\out\bios.img"
$persist = "C:\Users\gbran\OneDrive\Documents\spikeling-os\verify_persist.img"
$serial = "C:\Users\gbran\OneDrive\Documents\spikeling-os\verify_serial.log"
if (Test-Path $serial) { Remove-Item $serial -Force }
$cmdline = "`"C:\Program Files\qemu\qemu-system-x86_64.exe`" -serial file:$serial -display none -device isa-debug-exit,iobase=0xf4,iosize=0x04 -drive format=raw,file=$bios -drive file=$persist,if=ide,index=2,format=raw -monitor tcp:0.0.0.0:45570,server,nowait"
Write-Output $cmdline
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = $cmdline }
Write-Output ("ReturnValue=" + $r.ReturnValue + " ProcessId=" + $r.ProcessId)
