using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text.Json;

namespace SpireProfiler;

internal sealed class NativeStatisticsStore : IDisposable
{
    private const int MaxResponseBytes = 64 * 1024 * 1024;
    private readonly int thread = Environment.CurrentManagedThreadId;
    private ulong handle;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeStoreCreate();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeStoreDestroy(ulong store);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeStoreExecute(ulong store, [MarshalAs(UnmanagedType.LPUTF8Str)] string request);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeStoreResponse(ulong store, IntPtr buffer, int capacity);
    private readonly NativeStoreDestroy destroy;
    private readonly NativeStoreExecute execute;
    private readonly NativeStoreResponse response;

    private static T GetExport<T>(IntPtr lib, string name) where T : Delegate =>
        Marshal.GetDelegateForFunctionPointer<T>(NativeLibrary.GetExport(lib, name));

    internal NativeStatisticsStore(string path)
    {
        var lib = ProfilerNative.Library;
        var create = GetExport<NativeStoreCreate>(lib, "spire_profiler_store_create");
        destroy = GetExport<NativeStoreDestroy>(lib, "spire_profiler_store_destroy");
        execute = GetExport<NativeStoreExecute>(lib, "spire_profiler_store_execute");
        response = GetExport<NativeStoreResponse>(lib, "spire_profiler_store_response");
        handle = create();
        if (handle == 0) throw new IOException("Native statistics store unavailable");
        try
        {
            using var reply = Execute(new { op = "open", path });
            if (!reply.RootElement.GetProperty("ok").GetBoolean())
                throw new IOException(reply.RootElement.GetProperty("error").GetString());
        }
        catch { Dispose(); throw; }
    }

    internal JsonDocument Execute(object request)
    {
        ObjectDisposedException.ThrowIf(handle == 0, this);
        if (thread != Environment.CurrentManagedThreadId) throw new InvalidOperationException("Statistics store belongs to its creating thread");
        string json = JsonSerializer.Serialize(request, StatisticsJson.Options);
        if (execute(handle, json) != 1) throw new IOException("Native statistics request failed");
        int length = response(handle, IntPtr.Zero, 0);
        if (length <= 0 || length > MaxResponseBytes) throw new IOException("Invalid native statistics response size");
        var buffer = Marshal.AllocHGlobal(length);
        try
        {
            if (response(handle, buffer, length) != length) throw new IOException("Statistics response changed while copying");
            return JsonDocument.Parse(Marshal.PtrToStringUTF8(buffer, length - 1));
        }
        finally { Marshal.FreeHGlobal(buffer); }
    }

    public void Dispose()
    {
        if (handle == 0) return;
        if (thread != Environment.CurrentManagedThreadId) throw new InvalidOperationException("Statistics store belongs to its creating thread");
        destroy(handle);
        handle = 0;
    }
}
