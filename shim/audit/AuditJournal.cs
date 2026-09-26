using System;
using System.IO;
using System.Text;
using System.Text.Json;

namespace SpireProfiler;

// JSONL is an append-only diagnostic journal, not a statistics record. Each
// flushed line is independently readable; a missing footer means incomplete.
// Sequence numbers order facts. Native snapshots are claims to audit, not facts.
internal sealed class AuditJournal : IDisposable
{
    private const int MaxEventBytes = 1024 * 1024;
    private readonly StreamWriter writer;
    private readonly int eventLimit;
    private readonly long byteLimit;
    private readonly Action<string> report;
    private long bytes;
    private ulong sequence;
    private bool closed;
    internal bool Complete { get; private set; } = true;
    internal bool Open => !closed;

    internal AuditJournal(string path, Action<string> diagnostic, int maxEvents = 20_000, long maxBytes = 32 * 1024 * 1024)
    {
        eventLimit = maxEvents;
        byteLimit = maxBytes;
        report = message => { try { diagnostic(message); } catch (Exception) { } };
        // Independent per-combat limits bound developer capture, including verbose
        // supplier snapshots; one KiB remains available for an explicit cutoff.
        if (maxEvents < 1 || maxBytes < 2048) throw new ArgumentOutOfRangeException(nameof(maxEvents));
        Directory.CreateDirectory(Path.GetDirectoryName(path) ?? throw new ArgumentException("Audit path needs a directory", nameof(path)));
        writer = new StreamWriter(new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.Read), new UTF8Encoding(false)) { AutoFlush = true, NewLine = "\n" };
    }

    internal ulong Append(uint turn, string kind, object data)
    {
        if (closed) return 0;
        try
        {
            string line = JsonSerializer.Serialize(new { Seq = sequence + 1, Turn = turn, Event = kind, Data = data }, StatisticsJson.Options);
            int length = Encoding.UTF8.GetByteCount(line) + 1;
            if (sequence >= (ulong)eventLimit || length > MaxEventBytes || bytes + length > byteLimit - 1024)
            {
                Complete = false;
                writer.WriteLine(JsonSerializer.Serialize(new { Seq = ++sequence, Turn = turn, Event = "trace_truncated", Data = new { Reason = "audit-capacity" } }, StatisticsJson.Options));
                Dispose();
                report("audit trace truncated: capacity reached");
                return 0;
            }
            writer.WriteLine(line);
            bytes += length;
            sequence++;
            if (kind == "diagnostic") Complete = false;
            return sequence;
        }
        catch (Exception ex)
        {
            Complete = false;
            Dispose();
            report($"cannot write audit trace: {ex.Message}");
            return 0;
        }
    }

    public void Dispose()
    {
        if (closed) return;
        closed = true;
        try { writer.Dispose(); }
        catch (Exception ex) { Complete = false; report($"cannot close audit trace: {ex.Message}"); }
    }
}
