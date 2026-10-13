using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

// CI-only launcher. The production helper continues to refuse elevation.
public static partial class AevraMediumToken {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct Startup {
        public int size; public string reserved, desktop, title;
        public int x, y, width, height, charsX, charsY, fill, flags;
        public short show, reservedSize; public IntPtr reservedBytes, input, output, error;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInfo { public IntPtr process, thread; public int pid, tid; }
    [StructLayout(LayoutKind.Sequential)]
    struct Label { public IntPtr sid; public int attributes; }
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool SaferCreateLevel(int scope, int level, int flags, out IntPtr handle, IntPtr reserved);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool SaferComputeTokenFromLevel(IntPtr level, IntPtr source, out IntPtr token, int flags, IntPtr reserved);
    [DllImport("advapi32.dll")] static extern bool SaferCloseLevel(IntPtr level);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool ConvertStringSidToSid(string value, out IntPtr sid);
    [DllImport("advapi32.dll")] static extern int GetLengthSid(IntPtr sid);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool SetTokenInformation(IntPtr token, int info, ref Label value, int size);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool GetTokenInformation(IntPtr token, int info, out int value, int size, out int needed);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool CreateProcessAsUser(IntPtr token, string application, StringBuilder command, IntPtr processAttributes, IntPtr threadAttributes, bool inherit, int flags, IntPtr environment, string directory, ref Startup startup, out ProcessInfo process);
    [DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int which);
    [DllImport("kernel32.dll", SetLastError = true)] static extern uint WaitForSingleObject(IntPtr handle, uint timeout);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool GetExitCodeProcess(IntPtr handle, out int code);
    [DllImport("kernel32.dll")] static extern bool TerminateProcess(IntPtr process, uint code);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr value);
    static void Check(bool ok) { if (!ok) throw new Win32Exception(Marshal.GetLastWin32Error()); }
    public static int Run(string executable, string directory) {
        if (executable.Contains("\"")) throw new ArgumentException("Invalid executable path");
        IntPtr level = IntPtr.Zero, token = IntPtr.Zero, sid = IntPtr.Zero;
        try {
            // NORMALUSER strips administrator rights while keeping the same logon/session.
            Check(SaferCreateLevel(1, 0x20000, 1, out level, IntPtr.Zero));
            Check(SaferComputeTokenFromLevel(level, IntPtr.Zero, out token, 0, IntPtr.Zero));
            Check(ConvertStringSidToSid("S-1-16-8192", out sid));
            var label = new Label { sid = sid, attributes = 0x20 };
            Check(SetTokenInformation(token, 25, ref label, Marshal.SizeOf<Label>() + GetLengthSid(sid)));
            int elevated, needed;
            Check(GetTokenInformation(token, 20, out elevated, 4, out needed));
            if (elevated != 0) throw new InvalidOperationException("Refusing to run native acceptance with an elevated token");
            var start = new Startup {
                size = Marshal.SizeOf<Startup>(), desktop = @"winsta0\default", flags = 0x100,
                input = GetStdHandle(-10), output = GetStdHandle(-11), error = GetStdHandle(-12)
            };
            ProcessInfo child;
            Check(CreateProcessAsUser(token, executable, new StringBuilder("\"" + executable + "\" --nocapture"), IntPtr.Zero, IntPtr.Zero, true, 0, IntPtr.Zero, directory, ref start, out child));
            try {
                // A native fixture must fail visibly rather than leave an unbounded CI process.
                if (WaitForSingleObject(child.process, 180000) != 0) {
                    TerminateProcess(child.process, 1);
                    WaitForSingleObject(child.process, 5000);
                    throw new TimeoutException("Native fixture did not finish in 180 seconds");
                }
                int code; Check(GetExitCodeProcess(child.process, out code)); return code;
            } finally { CloseHandle(child.thread); CloseHandle(child.process); }
        } finally {
            if (sid != IntPtr.Zero) LocalFree(sid);
            if (token != IntPtr.Zero) CloseHandle(token);
            if (level != IntPtr.Zero) SaferCloseLevel(level);
        }
    }
}
