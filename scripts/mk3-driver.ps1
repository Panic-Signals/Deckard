# Which driver the screens of a Maschine MK3 or Maschine+ (their "BD" USB interface) use
# on Windows. NI's installer binds it to its own kernel driver, which only NI's software
# can talk to. DECKARD needs Windows' own WinUSB there instead, and with it NI's software
# loses the screens until the NI driver is restored. Only this one interface changes:
# pads, buttons, LEDs, audio and MIDI stay as they are. Asks for administrator approval.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/mk3-driver.ps1 -Use WinUSB
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/mk3-driver.ps1 -Use NI
param(
    [Parameter(Mandatory = $true)][ValidateSet("WinUSB", "NI")][string]$Use,
    # Set when this script has elevated itself: where to leave the error for the first copy.
    [string]$ErrorFile = ""
)
$ErrorActionPreference = "Stop"

# NI's services keep the interface open, so the driver can't change under them.
$Services = "NIHostIntegrationAgent", "NIHardwareService"
# Any fixed GUID: WinUSB publishes the interface under it, and nusb looks it up.
$InterfaceGuid = "{8e7f2d3a-4b61-4c19-9a55-3d6a1f0c7e21}"

# The controllers, the first found used (the same list as `MODELS` in crates/mk3/src/lib.rs).
$Models = @(
    @{ Name = "Maschine MK3"; Product = "1600"; Interface = "05"; Driver = "nimc3usb" },
    @{ Name = "Maschine+"; Product = "1820"; Interface = "06"; Driver = "nim3pbd" }
)

function Find-Screens {
    foreach ($m in $Models) {
        $d = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match "^USB\\VID_17CC&PID_$($m.Product)&MI_$($m.Interface)\\" } | Select-Object -First 1
        if ($d) { return @{ Model = $m; Device = $d } }
    }
}

function Get-ScreensDriver($id) {
    (Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_Device_Service).Data
}

# Forgotten and found again, the interface gets the best driver for it: NI's.
function Restore-NI($id) {
    pnputil /remove-device "$id" | Out-Null
    pnputil /scan-devices | Out-Null
    Start-Sleep -Seconds 2
}

$Found = Find-Screens
if (-not $Found) { throw "no Maschine MK3 or Maschine+ is plugged in" }
$Model = $Found.Model
$Id = $Found.Device.InstanceId

$Admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $Admin) {
    $Err = [IO.Path]::GetTempFileName()
    try {
        $P = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList @(
            "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$PSCommandPath`"", "-Use", $Use, "-ErrorFile", "`"$Err`"")
    } catch {
        throw "administrator approval was declined"
    }
    $Message = Get-Content $Err -Raw -ErrorAction SilentlyContinue
    Remove-Item $Err -ErrorAction SilentlyContinue
    if ($P.ExitCode -ne 0) {
        throw $(if ($Message) { $Message.Trim() } else { "changing the $($Model.Name)'s driver failed (exit $($P.ExitCode))" })
    }
    if ($Message) { Write-Host $Message.Trim() }
    exit 0
}

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class Mk3Driver {
    const uint DI_ENUMSINGLEINF = 0x10000;
    const uint DI_FLAGSEX_ALLOWEXCLUDEDDRVS = 0x800;
    const uint SPDIT_CLASSDRIVER = 1;
    const uint SPDRP_CLASSGUID = 8;
    const string WINUSB_CLASS = "{88BAE032-5A81-49f0-BC3D-A4FF138216D6}";
    static readonly IntPtr INVALID = new IntPtr(-1);

    [StructLayout(LayoutKind.Sequential)]
    struct DevInfo { public uint cbSize; public Guid ClassGuid; public uint DevInst; public IntPtr Reserved; }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct InstallParams {
        public uint cbSize; public uint Flags; public uint FlagsEx; public IntPtr hwndParent;
        public IntPtr InstallMsgHandler; public IntPtr InstallMsgHandlerContext; public IntPtr FileQueue;
        public IntPtr ClassInstallReserved; public uint Reserved;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string DriverPath;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct DriverInfo {
        public uint cbSize; public uint DriverType; public IntPtr Reserved;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string Description;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string MfgName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string ProviderName;
        public long DriverDate; public ulong DriverVersion;
    }

    [DllImport("setupapi", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr SetupDiCreateDeviceInfoList(IntPtr classGuid, IntPtr hwnd);
    [DllImport("setupapi", SetLastError = true)]
    static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);
    [DllImport("setupapi", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "SetupDiOpenDeviceInfoW")]
    static extern bool OpenDevice(IntPtr set, string id, IntPtr hwnd, uint flags, ref DevInfo dev);
    [DllImport("setupapi", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "SetupDiSetDeviceRegistryPropertyW")]
    static extern bool SetProperty(IntPtr set, ref DevInfo dev, uint property, byte[] value, uint size);
    [DllImport("setupapi", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "SetupDiGetDeviceInstallParamsW")]
    static extern bool GetParams(IntPtr set, ref DevInfo dev, ref InstallParams p);
    [DllImport("setupapi", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "SetupDiSetDeviceInstallParamsW")]
    static extern bool SetParams(IntPtr set, ref DevInfo dev, ref InstallParams p);
    [DllImport("setupapi", SetLastError = true, EntryPoint = "SetupDiBuildDriverInfoList")]
    static extern bool BuildList(IntPtr set, ref DevInfo dev, uint type);
    [DllImport("setupapi", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "SetupDiEnumDriverInfoW")]
    static extern bool EnumDrivers(IntPtr set, ref DevInfo dev, uint type, uint index, ref DriverInfo d);
    [DllImport("setupapi", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "SetupDiGetDriverInfoDetailW")]
    static extern bool DriverDetail(IntPtr set, ref DevInfo dev, ref DriverInfo d, byte[] detail, uint size, out uint needed);
    [DllImport("setupapi", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "SetupDiSetSelectedDriverW")]
    static extern bool SelectDriver(IntPtr set, ref DevInfo dev, ref DriverInfo d);
    [DllImport("newdev", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "DiInstallDevice")]
    static extern bool InstallDevice(IntPtr hwnd, IntPtr set, ref DevInfo dev, ref DriverInfo d, uint flags, out bool reboot);

    static Exception Fail(string what) {
        return new Exception(what + " failed (Windows error " + Marshal.GetLastWin32Error() + ")");
    }

    static IntPtr OpenDev(string id, out DevInfo dev) {
        IntPtr set = SetupDiCreateDeviceInfoList(IntPtr.Zero, IntPtr.Zero);
        if (set == INVALID) throw Fail("SetupDiCreateDeviceInfoList");
        dev = new DevInfo(); dev.cbSize = (uint)Marshal.SizeOf(typeof(DevInfo));
        if (!OpenDevice(set, id, IntPtr.Zero, 0, ref dev)) throw Fail("opening the device");
        return set;
    }

    // SP_DRVINFO_DETAIL_DATA_W on x64: SectionName starts at byte 32.
    static string Section(IntPtr set, ref DevInfo dev, ref DriverInfo d) {
        byte[] b = new byte[8192];
        BitConverter.GetBytes((uint)1584).CopyTo(b, 0);
        uint needed;
        if (!DriverDetail(set, ref dev, ref d, b, (uint)b.Length, out needed)) throw Fail("reading a driver's details");
        string s = System.Text.Encoding.Unicode.GetString(b, 32, 512);
        return s.Substring(0, s.IndexOf('\0'));
    }

    // Install winusb.inf's driver on the device; whether Windows wants a restart. Windows
    // offers a device only drivers of its own class, so it joins winusb.inf's class first.
    public static bool Install(string id, string inf, string section) {
        DevInfo dev;
        IntPtr set = OpenDev(id, out dev);
        byte[] cls = System.Text.Encoding.Unicode.GetBytes(WINUSB_CLASS + "\0");
        if (!SetProperty(set, ref dev, SPDRP_CLASSGUID, cls, (uint)cls.Length)) throw Fail("setting the device's class");
        SetupDiDestroyDeviceInfoList(set);
        set = OpenDev(id, out dev);
        InstallParams p = new InstallParams(); p.cbSize = (uint)Marshal.SizeOf(typeof(InstallParams));
        if (!GetParams(set, ref dev, ref p)) throw Fail("reading the install parameters");
        p.Flags |= DI_ENUMSINGLEINF;
        p.FlagsEx |= DI_FLAGSEX_ALLOWEXCLUDEDDRVS;
        p.DriverPath = inf;
        if (!SetParams(set, ref dev, ref p)) throw Fail("setting the install parameters");
        if (!BuildList(set, ref dev, SPDIT_CLASSDRIVER)) throw Fail("reading winusb.inf");
        DriverInfo d = new DriverInfo(); d.cbSize = (uint)Marshal.SizeOf(typeof(DriverInfo));
        string found = "";
        for (uint i = 0; EnumDrivers(set, ref dev, SPDIT_CLASSDRIVER, i, ref d); i++) {
            string name = Section(set, ref dev, ref d);
            found += " " + name;
            if (!string.Equals(name, section, StringComparison.OrdinalIgnoreCase)) continue;
            if (!SelectDriver(set, ref dev, ref d)) throw Fail("selecting WinUSB");
            bool reboot;
            if (!InstallDevice(IntPtr.Zero, set, ref dev, ref d, 0, out reboot)) throw Fail("installing WinUSB");
            SetupDiDestroyDeviceInfoList(set);
            return reboot;
        }
        throw new Exception("winusb.inf offers no " + section + " driver (found:" + found + ")");
    }
}
"@

try {
    $Running = @($Services | Where-Object { (Get-Service $_ -ErrorAction SilentlyContinue).Status -eq "Running" })
    foreach ($s in $Running) { Stop-Service $s -Force }
    $Reboot = $false
    try {
        if ($Use -eq "WinUSB") {
            $Params = "HKLM:\SYSTEM\CurrentControlSet\Enum\$Id\Device Parameters"
            New-ItemProperty -Path $Params -Name DeviceInterfaceGUIDs -PropertyType MultiString -Value @($InterfaceGuid) -Force | Out-Null
            try {
                $Reboot = [Mk3Driver]::Install($Id, (Join-Path $env:WINDIR "INF\winusb.inf"), "WINUSB")
                # WinUSB can bind and still not start (Code 31): the screens would have no
                # driver that works, so that counts as failing, and NI's goes back on.
                if (-not $Reboot) {
                    Start-Sleep -Seconds 1
                    $Problem = (Get-PnpDeviceProperty -InstanceId $Id -KeyName DEVPKEY_Device_ProblemCode).Data
                    if ($Problem) { throw "WinUSB wouldn't start on the $($Model.Name)'s screens (Windows problem code $Problem), so NI's driver is back" }
                }
            } catch {
                Restore-NI $Id
                throw
            }
        } else {
            Restore-NI $Id
        }
    } finally {
        foreach ($s in $Running) { Start-Service $s -ErrorAction SilentlyContinue }
    }
    $Now = Find-Screens
    $Driver = if ($Now) { Get-ScreensDriver $Now.Device.InstanceId } else { "" }
    $Wanted = if ($Use -eq "WinUSB") { "WinUSB" } else { $Model.Driver }
    if ($Driver -ne $Wanted) {
        throw "the $($Model.Name)'s screens use '$Driver', not $Wanted$(if ($Reboot) { '; restart Windows to finish' })"
    }
    if ($Reboot -and $ErrorFile) { "restart Windows to finish changing the $($Model.Name)'s driver" | Set-Content $ErrorFile }
    Write-Host "the $($Model.Name)'s screens use $Driver"
} catch {
    if ($ErrorFile) { "$($_.Exception.Message)" | Set-Content $ErrorFile }
    throw
}
