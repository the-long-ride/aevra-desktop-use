using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.AccessControl;
using System.Security.Principal;
using System.Text;

public static partial class AevraMediumToken {
    const int CREATE_SUSPENDED = 4;
    // READ_CONTROL/WRITE_DAC require READOBJECTS/WRITEOBJECTS on desktop handles.
    const int DesktopAclAccess = 0x60000 | 0x0001 | 0x0080;
    [StructLayout(LayoutKind.Sequential)]
    struct JobBasic {
        public long processTime, jobTime; public uint flags;
        public UIntPtr minWorkingSet, maxWorkingSet; public uint activeProcesses;
        public UIntPtr affinity; public uint priority, scheduling;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct IoCounters { public ulong reads, writes, other, readBytes, writeBytes, otherBytes; }
    [StructLayout(LayoutKind.Sequential)]
    struct JobLimits {
        public JobBasic basic; public IoCounters io;
        public UIntPtr processMemory, jobMemory, peakProcessMemory, peakJobMemory;
    }
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int info, ref JobLimits limits, uint size);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcessWithLogonW(string user, string domain, string password, int logonFlags, string application, StringBuilder command, int flags, IntPtr environment, string directory, ref Startup startup, out ProcessInfo process);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool OpenProcessToken(IntPtr process, int access, out IntPtr token);
    [DllImport("kernel32.dll", SetLastError = true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr OpenWindowStation(string name, bool inherit, int access);
    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr OpenDesktop(string name, int flags, bool inherit, int access);
    [DllImport("user32.dll", SetLastError = true)] static extern bool GetUserObjectSecurity(IntPtr handle, ref uint info, byte[] data, uint size, out uint needed);
    [DllImport("user32.dll", SetLastError = true)] static extern bool SetUserObjectSecurity(IntPtr handle, ref uint info, byte[] data);
    [DllImport("user32.dll")] static extern bool CloseWindowStation(IntPtr handle);
    [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr handle);

    public static int RunAsStandardUser(string executable, string script, string directory, string user, string password) {
        if (executable.Contains("\"") || script.Contains("\"")) throw new ArgumentException("Invalid launcher path");
        var start = new Startup { size = Marshal.SizeOf<Startup>(), desktop = @"winsta0\default" };
        ProcessInfo child;
        var command = new StringBuilder("\"" + executable + "\" -NoProfile -NonInteractive -ExecutionPolicy RemoteSigned -File \"" + script + "\"");
        // A new standard-account logon has real non-elevated token metadata.
        Check(CreateProcessWithLogonW(user, ".", password, 1, executable, command,
            CREATE_SUSPENDED | 0x08000000, IntPtr.Zero, directory, ref start, out child));
        bool finished = false;
        IntPtr job = IntPtr.Zero;
        try {
            // Own the complete test tree, including compiler servers and GUI fixtures.
            job = CreateJobObject(IntPtr.Zero, null);
            Check(job != IntPtr.Zero);
            var limits = new JobLimits { basic = new JobBasic { flags = 0x2000 } }; // KILL_ON_JOB_CLOSE
            Check(SetInformationJobObject(job, 9, ref limits, (uint)Marshal.SizeOf<JobLimits>()));
            Check(AssignProcessToJobObject(job, child.process));
            IntPtr token;
            Check(OpenProcessToken(child.process, 8, out token));
            try {
                int elevated, needed;
                Check(GetTokenInformation(token, 20, out elevated, 4, out needed));
                if (elevated != 0) throw new InvalidOperationException("Standard-account child is elevated; refusing execution");
            } finally { CloseHandle(token); }
            // The test cannot execute any instruction before the token check.
            if (ResumeThread(child.thread) == uint.MaxValue) throw new Win32Exception(Marshal.GetLastWin32Error());
            if (WaitForSingleObject(child.process, 240000) != 0) throw new TimeoutException("Native standard-user test exceeded 240 seconds");
            int code; Check(GetExitCodeProcess(child.process, out code)); finished = true; return code;
        } finally {
            if (!finished) { TerminateProcess(child.process, 1); WaitForSingleObject(child.process, 5000); }
            if (job != IntPtr.Zero) CloseHandle(job);
            CloseHandle(child.thread); CloseHandle(child.process);
        }
    }

    // Only the ephemeral account receives these grants; remove its ACEs on exit.
    public sealed class DesktopGrant : IDisposable {
        readonly SecurityIdentifier sid;
        readonly List<IntPtr> handles = new List<IntPtr>();
        bool disposed;
        internal DesktopGrant(string accountSid) {
            sid = new SecurityIdentifier(accountSid);
            try {
                Add(OpenWindowStation("winsta0", false, 0x60000), 0xF037F);
                Add(OpenDesktop("default", 0, false, DesktopAclAccess), 0xF01FF);
            } catch { Dispose(); throw; }
        }
        internal static void ValidateAccess() {
            IntPtr station = OpenWindowStation("winsta0", false, 0x60000);
            Check(station != IntPtr.Zero);
            try {
                Read(station);
                IntPtr desktop = OpenDesktop("default", 0, false, DesktopAclAccess);
                Check(desktop != IntPtr.Zero);
                try { Read(desktop); } finally { CloseDesktop(desktop); }
            } finally { CloseWindowStation(station); }
        }
        static RawSecurityDescriptor Read(IntPtr handle) {
            uint info = 4, needed;
            GetUserObjectSecurity(handle, ref info, null, 0, out needed);
            if (needed == 0) throw new Win32Exception(Marshal.GetLastWin32Error());
            var bytes = new byte[needed];
            Check(GetUserObjectSecurity(handle, ref info, bytes, needed, out needed));
            return new RawSecurityDescriptor(bytes, 0);
        }
        static void Write(IntPtr handle, RawSecurityDescriptor descriptor) {
            var bytes = new byte[descriptor.BinaryLength]; descriptor.GetBinaryForm(bytes, 0);
            uint info = 4; Check(SetUserObjectSecurity(handle, ref info, bytes));
        }
        void Add(IntPtr handle, int mask) {
            if (handle == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
            handles.Add(handle);
            var descriptor = Read(handle);
            if (descriptor.DiscretionaryAcl == null) return; // Null DACL already permits access.
            descriptor.DiscretionaryAcl.InsertAce(descriptor.DiscretionaryAcl.Count,
                new CommonAce(AceFlags.None, AceQualifier.AccessAllowed, mask, sid, false, null));
            Write(handle, descriptor);
        }
        public void Dispose() {
            if (disposed) return; disposed = true;
            Exception failure = null;
            for (int i = handles.Count - 1; i >= 0; i--) {
                try {
                    var descriptor = Read(handles[i]);
                    var acl = descriptor.DiscretionaryAcl;
                    if (acl != null) {
                        for (int a = acl.Count - 1; a >= 0; a--) {
                            var ace = acl[a] as CommonAce;
                            if (ace != null && ace.SecurityIdentifier.Equals(sid)) acl.RemoveAce(a);
                        }
                        Write(handles[i], descriptor);
                    }
                } catch (Exception error) { failure = error; }
                finally { if (i == 0) CloseWindowStation(handles[i]); else CloseDesktop(handles[i]); }
            }
            if (failure != null) throw failure;
        }
    }
    // Exercise hosted prerequisites locally too, without changing any permissions.
    public static void ValidateDesktopAccess() { DesktopGrant.ValidateAccess(); }
    public static DesktopGrant GrantDesktop(string sid) { return new DesktopGrant(sid); }
}
