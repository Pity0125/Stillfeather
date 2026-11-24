# Stillfeather：診斷「在資料夾中顯示」（瀏覽器的下載等）為什麼沒有選取檔案。
# 只讀取，不修改任何設定。會做三件事：
#   1. 列出 Windows 的殼層視窗清單（Stillfeather 要登記在這裡，Windows 才找得到它）
#   2. 用 Windows 自己找視窗的方式（FindWindowSW）找正在顯示「下載」資料夾的視窗
#   3. 呼叫瀏覽器用的同一個函式（SHOpenFolderAndSelectItems）顯示「下載」裡最新的檔案，並計時
# 結果寫到桌面的 Stillfeather-ShowInFolder.txt，內容包含使用者名稱（出現在路徑裡）。
param([string]$File)

$ErrorActionPreference = 'Continue'
$out = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Stillfeather-ShowInFolder.txt'

if (-not $File) {
    $downloads = $null
    try { $downloads = (New-Object -ComObject Shell.Application).NameSpace('shell:Downloads').Self.Path } catch { }
    if (-not $downloads) { $downloads = Join-Path $env:USERPROFILE 'Downloads' }
    $newest = Get-ChildItem -LiteralPath $downloads -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($newest) { $File = $newest.FullName }
}
if (-not $File -or -not (Test-Path -LiteralPath $File)) {
    Write-Host '找不到要測試的檔案。請把檔案路徑當參數傳入，例如：'
    Write-Host '  DiagnoseShowInFolder.cmd "C:\Users\你\Downloads\某個檔案.zip"'
    exit 1
}
# 和 Chrome 相同：資料夾結尾加上 \
$folder = [System.IO.Path]::GetDirectoryName($File)
if (-not $folder.EndsWith('\')) { $folder += '\' }

$code = @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

[ComImport, Guid("85CB6900-4D95-11CF-960C-0080C7F4EE85"), InterfaceType(ComInterfaceType.InterfaceIsDual)]
public interface IShellWindowsDiag {
    int Count { get; }
    [return: MarshalAs(UnmanagedType.IDispatch)] object Item(object index);
    [return: MarshalAs(UnmanagedType.IUnknown)] object _NewEnum();
    [PreserveSig] int Register([MarshalAs(UnmanagedType.IDispatch)] object disp, int hwnd, int swClass, out int cookie);
    [PreserveSig] int RegisterPending(int threadId, ref object loc, ref object root, int swClass, out int cookie);
    [PreserveSig] int Revoke(int cookie);
    [PreserveSig] int OnNavigate(int cookie, ref object loc);
    [PreserveSig] int OnActivated(int cookie, [MarshalAs(UnmanagedType.VariantBool)] bool active);
    [PreserveSig] int FindWindowSW(ref object loc, ref object root, int swClass, out int hwnd, int options, [MarshalAs(UnmanagedType.IDispatch)] out object disp);
    [PreserveSig] int OnCreated(int cookie, [MarshalAs(UnmanagedType.IUnknown)] object punk);
    [PreserveSig] int ProcessAttachDetach([MarshalAs(UnmanagedType.VariantBool)] bool attach);
}

[ComImport, Guid("6D5140C1-7436-11CE-8034-00AA006009FA"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IServiceProviderDiag {
    [PreserveSig] int QueryService(ref Guid service, ref Guid riid, out IntPtr ppv);
}

[ComImport, Guid("000214E6-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IShellFolderDiag {
    [PreserveSig] int ParseDisplayName(IntPtr hwnd, IntPtr bc, [MarshalAs(UnmanagedType.LPWStr)] string name, IntPtr eaten, out IntPtr pidl, IntPtr attributes);
}

[ComImport, Guid("00000114-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IOleWindowDiag {
    [PreserveSig] int GetWindow(out IntPtr hwnd);
    [PreserveSig] int ContextSensitiveHelp(int enter);
}

public static class ShowInFolderDiag {
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    static extern int SHParseDisplayName(string name, IntPtr bc, out IntPtr pidl, uint sfgaoIn, out uint sfgaoOut);
    [DllImport("shell32.dll")]
    static extern int SHOpenFolderAndSelectItems(IntPtr folder, uint count, IntPtr[] items, uint flags);
    [DllImport("shell32.dll")]
    static extern uint ILGetSize(IntPtr pidl);
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr ILCreateFromPathW(string path);
    [DllImport("shell32.dll")]
    static extern int SHGetDesktopFolder(out IShellFolderDiag folder);
    [DllImport("ole32.dll")]
    static extern void CoTaskMemFree(IntPtr p);
    [DllImport("user32.dll")]
    static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")]
    static extern bool IsWindowVisible(IntPtr hwnd);

    static readonly Guid SID_STopLevelBrowser = new Guid("4C96BE40-915C-11CF-99D3-00AA004AE837");
    static readonly Guid IID_IOleWindow = new Guid("00000114-0000-0000-C000-000000000046");
    static readonly Guid IID_IShellView = new Guid("000214E3-0000-0000-C000-000000000046");
    static readonly Guid IID_IShellBrowser = new Guid("000214E2-0000-0000-C000-000000000046");
    static readonly Guid IID_IWebBrowserApp = new Guid("0002DF05-0000-0000-C000-000000000046");

    static string Hr(int hr) { return "0x" + hr.ToString("X8"); }

    static byte[] Bytes(IntPtr pidl) {
        if (pidl == IntPtr.Zero) return null;
        byte[] b = new byte[ILGetSize(pidl)];
        Marshal.Copy(pidl, b, 0, b.Length);
        return b;
    }

    static bool Same(byte[] a, byte[] b) {
        if (a == null || b == null || a.Length != b.Length) return false;
        for (int i = 0; i < a.Length; i++) if (a[i] != b[i]) return false;
        return true;
    }

    // 每一層的大小與前 24 個位元組（看得出是哪一種根：本機、使用者資料夾…）
    static string Describe(byte[] b) {
        if (b == null) return "（沒有）";
        StringBuilder s = new StringBuilder(b.Length + " 位元組：");
        int off = 0, level = 0;
        while (off + 2 <= b.Length) {
            int cb = b[off] | (b[off + 1] << 8);
            if (cb == 0) break;
            s.Append(" [" + level + "] " + cb + " 位元組 ");
            for (int i = off + 2; i < Math.Min(off + cb, off + 26); i++) s.Append(b[i].ToString("X2"));
            off += cb;
            level++;
        }
        return s.ToString();
    }

    // 和 Chrome 相同：桌面資料夾的 ParseDisplayName
    static IntPtr DesktopParse(string path) {
        IShellFolderDiag desktop;
        if (SHGetDesktopFolder(out desktop) < 0 || desktop == null) return IntPtr.Zero;
        IntPtr pidl;
        int hr = desktop.ParseDisplayName(IntPtr.Zero, IntPtr.Zero, path, IntPtr.Zero, out pidl, IntPtr.Zero);
        Marshal.ReleaseComObject(desktop);
        return hr < 0 ? IntPtr.Zero : pidl;
    }

    static string Owner(IntPtr hwnd) {
        if (hwnd == IntPtr.Zero) return "（沒有）";
        uint pid;
        GetWindowThreadProcessId(hwnd, out pid);
        string name = "?";
        try { name = Process.GetProcessById((int)pid).ProcessName; } catch { }
        return "0x" + hwnd.ToInt64().ToString("X") + "，" + name + "（" + pid + "）" + (IsWindowVisible(hwnd) ? "，看得見" : "，隱藏");
    }

    static string Services(object disp) {
        IServiceProviderDiag sp = disp as IServiceProviderDiag;
        if (sp == null) return "沒有 IServiceProvider";
        StringBuilder s = new StringBuilder("IServiceProvider：");
        Guid service = SID_STopLevelBrowser;
        Guid[] wanted = { IID_IShellView, IID_IShellBrowser, IID_IWebBrowserApp };
        string[] names = { "IShellView", "IShellBrowser", "IWebBrowserApp" };
        for (int i = 0; i < wanted.Length; i++) {
            Guid riid = wanted[i];
            IntPtr p;
            int hr = sp.QueryService(ref service, ref riid, out p);
            s.Append(names[i] + " " + Hr(hr) + "　");
            if (p != IntPtr.Zero) Marshal.Release(p);
        }
        return s.ToString();
    }

    static IntPtr WindowOf(object disp) {
        IServiceProviderDiag sp = disp as IServiceProviderDiag;
        if (sp == null) return IntPtr.Zero;
        Guid service = SID_STopLevelBrowser, riid = IID_IOleWindow;
        IntPtr p;
        if (sp.QueryService(ref service, ref riid, out p) < 0 || p == IntPtr.Zero) return IntPtr.Zero;
        IntPtr hwnd = IntPtr.Zero;
        try {
            IOleWindowDiag w = (IOleWindowDiag)Marshal.GetObjectForIUnknown(p);
            w.GetWindow(out hwnd);
        } catch { }
        Marshal.Release(p);
        return hwnd;
    }

    public static string Run(string folder, string file) {
        string result = null;
        Thread t = new Thread(delegate () {
            try { result = RunSta(folder, file); } catch (Exception e) { result = "錯誤：" + e; }
        });
        t.SetApartmentState(ApartmentState.STA);
        t.Start();
        t.Join();
        return result;
    }

    static string RunSta(string folder, string file) {
        StringBuilder r = new StringBuilder();
        IntPtr parsePidl;
        uint attrs;
        int hr = SHParseDisplayName(folder, IntPtr.Zero, out parsePidl, 0, out attrs);
        // Chrome 的做法：資料夾與檔案都用桌面資料夾的 ParseDisplayName
        IntPtr folderPidl = DesktopParse(folder);
        IntPtr filePidl = DesktopParse(file);
        IntPtr createPidl = ILCreateFromPathW(folder);
        byte[] parseBytes = Bytes(parsePidl), desktopBytes = Bytes(folderPidl), createBytes = Bytes(createPidl);
        r.AppendLine("---- 資料夾的識別碼（三種取得方式）");
        r.AppendLine("  SHParseDisplayName（Stillfeather、多數程式）：" + Describe(parseBytes));
        r.AppendLine("  桌面 ParseDisplayName（Chrome、Edge）：" + Describe(desktopBytes));
        r.AppendLine("  ILCreateFromPath（Firefox）：" + Describe(createBytes));
        r.AppendLine("  SHParseDisplayName 與 Chrome 的" + (Same(parseBytes, desktopBytes) ? "相同" : "不同") + "；與 ILCreateFromPath 的" + (Same(parseBytes, createBytes) ? "相同" : "不同"));
        if (folderPidl == IntPtr.Zero || filePidl == IntPtr.Zero) return r.ToString();

        IShellWindowsDiag sw = (IShellWindowsDiag)Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("9BA05972-F6A8-11CF-A442-00A0C90A8F39")));
        int count = sw.Count;
        r.AppendLine();
        r.AppendLine("---- 殼層視窗清單：" + count + " 個");
        for (int i = 0; i < count; i++) {
            try {
                object item = sw.Item(i);
                if (item == null) { r.AppendLine("  #" + i + "：（空）"); continue; }
                r.AppendLine("  #" + i + "：" + Owner(WindowOf(item)));
                Marshal.ReleaseComObject(item);
            } catch (Exception e) {
                r.AppendLine("  #" + i + "：" + e.GetType().Name + " " + Hr(Marshal.GetHRForException(e)));
            }
        }

        r.AppendLine();
        r.AppendLine("---- FindWindowSW（找正在顯示這個資料夾的視窗）");
        int[] classes = { 1, 1, 1, 0 };
        int[] options = { 3, 3, 0, 3 };
        byte[][] locs = { desktopBytes, parseBytes, desktopBytes, desktopBytes };
        string[] labels = { "Chrome 的識別碼，SWC_BROWSER，含準備中、要物件（Windows 自己用的）", "SHParseDisplayName 的識別碼，同上", "Chrome 的識別碼，SWC_BROWSER", "Chrome 的識別碼，SWC_EXPLORER，含準備中、要物件" };
        for (int i = 0; i < classes.Length; i++) {
            if (locs[i] == null) continue;
            object loc = locs[i], root = null, disp;
            int hwnd;
            hr = sw.FindWindowSW(ref loc, ref root, classes[i], out hwnd, options[i], out disp);
            r.AppendLine("  " + labels[i] + "：" + Hr(hr) + "，視窗 " + Owner(new IntPtr(hwnd)));
            if (disp != null) {
                r.AppendLine("    物件：" + Services(disp));
                Marshal.ReleaseComObject(disp);
            }
        }
        Marshal.ReleaseComObject(sw);

        r.AppendLine();
        r.AppendLine("---- SHOpenFolderAndSelectItems（瀏覽器用的函式，參數和 Chrome 相同；找不到視窗時會等 15～20 秒）");
        // 和 Chrome 相同：傳完整的檔案識別碼
        IntPtr[] items = { filePidl };
        Stopwatch watch = Stopwatch.StartNew();
        hr = SHOpenFolderAndSelectItems(folderPidl, 1, items, 0);
        watch.Stop();
        r.AppendLine("  結果：" + Hr(hr) + "，花了 " + watch.ElapsedMilliseconds + " 毫秒");
        CoTaskMemFree(filePidl);
        CoTaskMemFree(folderPidl);
        if (parsePidl != IntPtr.Zero) CoTaskMemFree(parsePidl);
        if (createPidl != IntPtr.Zero) CoTaskMemFree(createPidl);
        return r.ToString();
    }
}
'@

Write-Host '正在診斷「在資料夾中顯示」…'
Write-Host "測試的檔案：$File"
Write-Host '最後一步會呼叫瀏覽器用的同一個函式，Stillfeather 或檔案總管會開啟並選取這個檔案（最多等 20 秒）。'

$report = New-Object System.Collections.Generic.List[string]
$report.Add('==== Stillfeather「在資料夾中顯示」診斷 ====')
$report.Add("時間：$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')")
$cv = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -ErrorAction SilentlyContinue
$report.Add("Windows：$($cv.ProductName) $($cv.DisplayVersion)（組建 $($cv.CurrentBuild).$($cv.UBR)）")
$report.Add("檔案：$File")
$report.Add("資料夾：$folder")
$verb = (Get-ItemProperty 'HKCU:\Software\Classes\Directory\shell' -ErrorAction SilentlyContinue).'(default)'
$report.Add("開啟資料夾的預設動詞（目前使用者）：$verb")
$report.Add('---- 執行中的 Stillfeather')
Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -in 'Stillfeather.exe', 'FeatherFiles.exe' } | ForEach-Object { $report.Add("  $($_.ProcessId)：$($_.CommandLine)") }
$report.Add('')
try {
    Add-Type -TypeDefinition $code -Language CSharp -IgnoreWarnings -ErrorAction Stop
    $report.Add([ShowInFolderDiag]::Run($folder, $File))
} catch {
    $report.Add("無法執行測試：$_")
}
# Stillfeather 的記錄（開發版在 exe 旁；安裝版在 %LOCALAPPDATA%\Stillfeather\logs）：最後幾行，去掉登記時 COM 自己的查詢
Start-Sleep -Seconds 1
$logs = @()
Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -in 'Stillfeather.exe', 'FeatherFiles.exe' -and $_.ExecutablePath } | ForEach-Object { $logs += Join-Path (Split-Path $_.ExecutablePath) 'shell-windows.log' }
$logs += Join-Path $env:LOCALAPPDATA 'Stillfeather\logs\shell-windows.log'
$log = $logs | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
$report.Add('')
if ($log) {
    $report.Add("---- Stillfeather 的記錄（$log，最後 40 行）")
    Get-Content -LiteralPath $log -Encoding UTF8 | Where-Object { $_ -notmatch 'QueryInterface .*：沒有$' } | Select-Object -Last 40 | ForEach-Object { $report.Add($_) }
} else {
    $report.Add('---- 找不到 Stillfeather 的記錄 shell-windows.log')
}
$report | Out-File -LiteralPath $out -Encoding utf8
Write-Host ''
Write-Host '已寫到桌面的 Stillfeather-ShowInFolder.txt。'
Start-Process notepad $out
