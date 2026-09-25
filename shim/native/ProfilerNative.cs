using System;
using System.IO;
using System.Runtime.InteropServices;

namespace SpireProfiler;

[StructLayout(LayoutKind.Explicit, Size = 16)]
internal readonly struct BlockModifier
{
    [FieldOffset(0)] internal readonly ulong Source;
    [FieldOffset(8)] internal readonly long Credit;

    internal BlockModifier(ulong source, long credit)
    {
        Source = source;
        Credit = credit;
    }
}

// One managed lifetime owns the native engine; only data crosses this boundary.
internal static class ProfilerNative
{
    internal const int TeamSlot = 4;
    private static ulong engine;
    // Delegate entry points remain valid after an engine is disposed.
    private static IntPtr library;
    private static string libraryPath;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeEngineCreate();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeEngineDestroy(ulong engine);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeRevision(ulong engine);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeSnapshot(ulong engine, IntPtr buffer, int capacity);
    private static NativeEngineCreate _engine_create;
    private static NativeEngineDestroy _engine_destroy;
    private static NativeEngineDestroy _combat_discard;
    private static NativeRevision _revision;
    private static NativeSnapshot _snapshot;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeRecordingBegin(ulong engine);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeReplay(ulong engine, [MarshalAs(UnmanagedType.LPUTF8Str)] string recording);
    private static NativeRecordingBegin _recording_begin;
    private static NativeSnapshot _recording;
    private static NativeReplay _replay;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate long NativeModifierCredit(ulong engine, ulong basisLow, ulong basisHigh, ulong valueLow, ulong valueHigh, ulong limitLow, ulong limitHigh, int kind);
    private static NativeModifierCredit _modifier_credit;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeWeakPrevention(ulong engine, int total, int receiverSlot, int receiverPlayer, int weak, int debilitate, uint kraneSlots);
    private static NativeWeakPrevention _weak_prevention;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeSourceCapture(ulong engine, ulong combatSeq, int captureKind, ulong instance, [MarshalAs(UnmanagedType.LPUTF8Str)] string sourceId, int sourceKind, int sourceSlot, int generationState);
    private static NativeSourceCapture _source_capture;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeSourceRelease(ulong engine, ulong handle);
    private static NativeSourceRelease _source_release;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerAttached(ulong engine, ulong combatSeq, ulong powerInstance, [MarshalAs(UnmanagedType.LPUTF8Str)] string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int amount, ulong sourceTransfer);
    private static NativePowerAttached _power_attached;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerAmountChanged(ulong engine, ulong combatSeq, ulong powerInstance, [MarshalAs(UnmanagedType.LPUTF8Str)] string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int oldAmount, int newAmount, ulong sourceTransfer);
    private static NativePowerAmountChanged _power_amount_changed;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerRemoved(ulong engine, ulong combatSeq, ulong powerInstance);
    private static NativePowerRemoved _power_removed;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerProvenanceInvalidate(ulong engine, ulong combatSeq, ulong powerInstance);
    private static NativePowerProvenanceInvalidate _power_provenance_invalidate;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCardGenerated(ulong engine, ulong combatSeq, ulong cardInstance, ulong sourceTransfer, int producerRole);
    private static NativeCardGenerated _card_generated;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeCardPlayStarted(ulong engine, ulong combatSeq, ulong executionId, ulong cardInstance, [MarshalAs(UnmanagedType.LPUTF8Str)] string cardId, int playerSlot, int playIndex, int playCount, int generationState, ulong sourceTransfer);
    private static NativeCardPlayStarted _card_play_started;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCardPlayFinished(ulong engine, ulong play);
    private static NativeCardPlayFinished _card_play_finished;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCardExecutionEnded(ulong engine, ulong combatSeq, ulong executionId);
    private static NativeCardExecutionEnded _card_execution_ended;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOrbChanneled(ulong engine, ulong combatSeq, ulong orbInstance, ulong sourceTransfer);
    private static NativeOrbChanneled _orb_channeled;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOrbContextBegin(ulong engine, ulong combatSeq, ulong orbInstance, ulong play, int ownerSlot);
    private static NativeOrbContextBegin _orb_context_begin;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeDamageCalculationBegin(ulong engine, ulong combatSeq, ulong sourceTransfer, int producerRole, int segment, ulong originalTarget);
    private static NativeDamageCalculationBegin _damage_calculation_begin;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageModifierContribution(ulong engine, ulong calculation, ulong sourceTransfer, int amount);
    private static NativeDamageModifierContribution _damage_modifier_contribution;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationEnemyHit(ulong engine, ulong calculation, ulong dealerCreature, int baseDamage, int dealerStrength);
    private static NativeDamageCalculationEnemyHit _damage_calculation_enemy_hit;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationWeakSource(ulong engine, ulong calculation, ulong sourceTransfer);
    private static NativeDamageCalculationWeakSource _damage_calculation_weak_source;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageResultAppend(ulong engine, ulong calculation, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented);
    private static NativeDamageResultAppend _damage_result_append;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationCommit(ulong engine, ulong calculation);
    private static NativeDamageCalculationCommit _damage_calculation_commit;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationAbort(ulong engine, ulong calculation);
    private static NativeDamageCalculationAbort _damage_calculation_abort;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageUnattributed(ulong engine, ulong combatSeq, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented);
    private static NativeDamageUnattributed _damage_unattributed;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeBuffMitigation(ulong engine, ulong combatSeq, ulong sourceTransfer, int prevented);
    private static NativeBuffMitigation _buff_mitigation;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeBlockGained(ulong engine, ulong combatSeq, int amount, ulong sourceTransfer, int receiverSlot, IntPtr modifiers, int count, int incomplete);
    private static NativeBlockGained _block_gained;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeForge(ulong engine, ulong combatSeq, ulong sourceTransfer, int amount);
    private static NativeForge _forge;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOstySummoned(ulong engine, ulong combatSeq, ulong sourceTransfer, int hpAmount, int ownerSlot);
    private static NativeOstySummoned _osty_summoned;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOstyKilled(ulong engine, ulong combatSeq, int ownerSlot, ulong play);
    private static NativeOstyKilled _osty_killed;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeDoomBatchBegin(ulong engine, ulong combatSeq);
    private static NativeDoomBatchBegin _doom_batch_begin;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDoomTargetCapture(ulong engine, ulong batch, ulong creatureInstance, ulong doomPowerInstance, int currentHp);
    private static NativeDoomTargetCapture _doom_target_capture;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDoomKillsCompleted(ulong engine, ulong batch);
    private static NativeDoomKillsCompleted _doom_kills_completed;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDoomBatchAbort(ulong engine, ulong batch);
    private static NativeDoomBatchAbort _doom_batch_abort;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCombatEnded(ulong engine, ulong combatSeq);
    private static NativeCombatEnded _combat_ended;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeTurnStarted(ulong engine, ulong combatSeq);
    private static NativeTurnStarted _turn_started;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeBlockPoolClear(ulong engine, ulong combatSeq, int playerSlot);
    private static NativeBlockPoolClear _block_pool_clear;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePlayerDied(ulong engine, ulong combatSeq, int playerSlot);
    private static NativePlayerDied _player_died;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePotionUsed(ulong engine, ulong combatSeq);
    private static NativePotionUsed _potion_used;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeCombatStarted(ulong engine, uint seq, [MarshalAs(UnmanagedType.LPUTF8Str)] string encounterId, [MarshalAs(UnmanagedType.LPUTF8Str)] string encounterType, long startedAt, int playerCount);
    private static NativeCombatStarted _combat_started;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeSourceAccumulate(ulong engine, ulong combatSeq, ulong first, int before, ulong second, int after);
    private static NativeSourceAccumulate _source_accumulate;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeCaptureFailed(ulong engine, [MarshalAs(UnmanagedType.LPUTF8Str)] string reason);
    private static NativeCaptureFailed _capture_failed;

    private static T GetExport<T>(IntPtr lib, string name) where T : Delegate =>
        Marshal.GetDelegateForFunctionPointer<T>(NativeLibrary.GetExport(lib, name));

    internal static void Load(string path)
    {
        if (engine != 0) return;
        path = Path.GetFullPath(path);
        if (library != IntPtr.Zero && path != libraryPath)
            throw new InvalidOperationException("The native library is already bound to another path");
        if (library == IntPtr.Zero)
        {
            var lib = NativeLibrary.Load(path);
            try
            {
                // Evaluate every binding before publishing any callable delegate.
                (_engine_create,
                    _engine_destroy,
                    _combat_discard,
                    _revision,
                    _snapshot,
                    _source_capture,
                    _source_release,
                    _power_attached,
                    _power_amount_changed,
                    _power_removed,
                    _power_provenance_invalidate,
                    _card_generated,
                    _card_play_started,
                    _card_play_finished,
                    _card_execution_ended,
                    _orb_channeled,
                    _orb_context_begin,
                    _damage_calculation_begin,
                    _damage_modifier_contribution,
                    _damage_calculation_enemy_hit,
                    _damage_calculation_weak_source,
                    _damage_result_append,
                    _damage_calculation_commit,
                    _damage_calculation_abort,
                    _damage_unattributed,
                    _buff_mitigation,
                    _block_gained,
                    _forge,
                    _osty_summoned,
                    _osty_killed,
                    _doom_batch_begin,
                    _doom_target_capture,
                    _doom_kills_completed,
                    _doom_batch_abort,
                    _combat_ended,
                    _turn_started,
                    _block_pool_clear,
                    _player_died,
                    _potion_used,
                    _combat_started,
                    _source_accumulate,
                    _capture_failed,
                    _recording_begin,
                    _recording,
                    _replay,
                    _modifier_credit,
                    _weak_prevention) =
                    (GetExport<NativeEngineCreate>(lib, "spire_profiler_engine_create"),
                    GetExport<NativeEngineDestroy>(lib, "spire_profiler_engine_destroy"),
                    GetExport<NativeEngineDestroy>(lib, "spire_profiler_combat_discard"),
                    GetExport<NativeRevision>(lib, "spire_profiler_revision"),
                    GetExport<NativeSnapshot>(lib, "spire_profiler_snapshot"),
                    GetExport<NativeSourceCapture>(lib, "spire_profiler_source_capture"),
                    GetExport<NativeSourceRelease>(lib, "spire_profiler_source_release"),
                    GetExport<NativePowerAttached>(lib, "spire_profiler_power_attached"),
                    GetExport<NativePowerAmountChanged>(lib, "spire_profiler_power_amount_changed"),
                    GetExport<NativePowerRemoved>(lib, "spire_profiler_power_removed"),
                    GetExport<NativePowerProvenanceInvalidate>(lib, "spire_profiler_power_provenance_invalidate"),
                    GetExport<NativeCardGenerated>(lib, "spire_profiler_card_generated"),
                    GetExport<NativeCardPlayStarted>(lib, "spire_profiler_card_play_started"),
                    GetExport<NativeCardPlayFinished>(lib, "spire_profiler_card_play_finished"),
                    GetExport<NativeCardExecutionEnded>(lib, "spire_profiler_card_execution_ended"),
                    GetExport<NativeOrbChanneled>(lib, "spire_profiler_orb_channeled"),
                    GetExport<NativeOrbContextBegin>(lib, "spire_profiler_orb_context_begin"),
                    GetExport<NativeDamageCalculationBegin>(lib, "spire_profiler_damage_calculation_begin"),
                    GetExport<NativeDamageModifierContribution>(lib, "spire_profiler_damage_modifier_contribution"),
                    GetExport<NativeDamageCalculationEnemyHit>(lib, "spire_profiler_damage_calculation_enemy_hit"),
                    GetExport<NativeDamageCalculationWeakSource>(lib, "spire_profiler_damage_calculation_weak_source"),
                    GetExport<NativeDamageResultAppend>(lib, "spire_profiler_damage_result_append"),
                    GetExport<NativeDamageCalculationCommit>(lib, "spire_profiler_damage_calculation_commit"),
                    GetExport<NativeDamageCalculationAbort>(lib, "spire_profiler_damage_calculation_abort"),
                    GetExport<NativeDamageUnattributed>(lib, "spire_profiler_damage_unattributed"),
                    GetExport<NativeBuffMitigation>(lib, "spire_profiler_buff_mitigation"),
                    GetExport<NativeBlockGained>(lib, "spire_profiler_block_gained"),
                    GetExport<NativeForge>(lib, "spire_profiler_forge"),
                    GetExport<NativeOstySummoned>(lib, "spire_profiler_osty_summoned"),
                    GetExport<NativeOstyKilled>(lib, "spire_profiler_osty_killed"),
                    GetExport<NativeDoomBatchBegin>(lib, "spire_profiler_doom_batch_begin"),
                    GetExport<NativeDoomTargetCapture>(lib, "spire_profiler_doom_target_capture"),
                    GetExport<NativeDoomKillsCompleted>(lib, "spire_profiler_doom_kills_completed"),
                    GetExport<NativeDoomBatchAbort>(lib, "spire_profiler_doom_batch_abort"),
                    GetExport<NativeCombatEnded>(lib, "spire_profiler_combat_ended"),
                    GetExport<NativeTurnStarted>(lib, "spire_profiler_turn_started"),
                    GetExport<NativeBlockPoolClear>(lib, "spire_profiler_block_pool_clear"),
                    GetExport<NativePlayerDied>(lib, "spire_profiler_player_died"),
                    GetExport<NativePotionUsed>(lib, "spire_profiler_potion_used"),
                    GetExport<NativeCombatStarted>(lib, "spire_profiler_combat_started"),
                    GetExport<NativeSourceAccumulate>(lib, "spire_profiler_source_accumulate"),
                    GetExport<NativeCaptureFailed>(lib, "spire_profiler_capture_failed"),
                    GetExport<NativeRecordingBegin>(lib, "spire_profiler_recording_begin"),
                    GetExport<NativeSnapshot>(lib, "spire_profiler_recording"),
                    GetExport<NativeReplay>(lib, "spire_profiler_replay"),
                    GetExport<NativeModifierCredit>(lib, "spire_profiler_modifier_credit"),
                    GetExport<NativeWeakPrevention>(lib, "spire_profiler_weak_prevention"));
                library = lib;
                libraryPath = path;
            }
            catch
            {
                NativeLibrary.Free(lib);
                throw;
            }
        }
        engine = _engine_create();
        if (engine == 0) throw new InvalidOperationException("Native attribution engine unavailable");
    }

    internal static int CalculateWeakPrevention(int total, int receiverSlot, bool receiverPlayer, bool weak, bool debilitate, uint kraneSlots)
    {
        int amount = _weak_prevention(engine, total, receiverSlot, receiverPlayer ? 1 : 0, weak ? 1 : 0, debilitate ? 1 : 0, kraneSlots);
        if (amount < 0) throw new InvalidOperationException("Weak observation rejected");
        return amount;
    }

    internal static int CalculateModifierCredit(decimal basis, decimal value, int kind, decimal limit)
    {
        var (basisLow, basisHigh) = DecimalWords.Pack(basis);
        var (valueLow, valueHigh) = DecimalWords.Pack(value);
        var (limitLow, limitHigh) = DecimalWords.Pack(limit);
        long amount = _modifier_credit(engine, basisLow, basisHigh, valueLow, valueHigh, limitLow, limitHigh, kind);
        if (amount < int.MinValue || amount > int.MaxValue) throw new OverflowException("Modifier credit is not representable");
        return (int)amount;
    }

    internal static void Dispose()
    {
        if (engine == 0) return;
        _engine_destroy(engine);
        engine = 0;
    }
    internal static void CombatDiscard() { if (engine != 0) _combat_discard(engine); }
    internal static ulong Revision => engine == 0 ? 0 : _revision(engine);
    internal static string Snapshot() => ReadJson(_snapshot, engine);
    internal static bool RecordingBegin() => engine != 0 && _recording_begin(engine) == 1;
    internal static string Recording() => ReadJson(_recording, engine);
    internal static string Replay(string recording)
    {
        ulong replay = _engine_create();
        if (replay == 0) throw new InvalidOperationException("Replay engine unavailable");
        try
        {
            if (_replay(replay, recording) != 1) throw new InvalidOperationException("Recording is incomplete or incompatible");
            return ReadJson(_snapshot, replay);
        }
        finally { _engine_destroy(replay); }
    }
    private static string ReadJson(NativeSnapshot read, ulong owner)
    {
        if (owner == 0) return "null";
        int length = read(owner, IntPtr.Zero, 0);
        if (length <= 0) return "null";
        var buffer = Marshal.AllocHGlobal(length);
        try
        {
            if (read(owner, buffer, length) != length) throw new InvalidOperationException("Snapshot changed while copying");
            return Marshal.PtrToStringUTF8(buffer, length - 1);
        }
        finally { Marshal.FreeHGlobal(buffer); }
    }
    internal static int SourceRelease(ulong handle) => _source_release(engine, handle);
    internal static ulong SourceCapture(ulong combatSeq, int captureKind, ulong instance, string sourceId, int sourceKind, int sourceSlot, int generationState) => _source_capture(engine, combatSeq, captureKind, instance, sourceId, sourceKind, sourceSlot, generationState);
    internal static int PowerAttached(ulong combatSeq, ulong powerInstance, string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int amount, ulong sourceTransfer) => _power_attached(engine, combatSeq, powerInstance, powerId, ownerCreature, ownerKind, ownerSlot, amount, sourceTransfer);
    internal static int PowerAmountChanged(ulong combatSeq, ulong powerInstance, string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int oldAmount, int newAmount, ulong sourceTransfer) => _power_amount_changed(engine, combatSeq, powerInstance, powerId, ownerCreature, ownerKind, ownerSlot, oldAmount, newAmount, sourceTransfer);
    internal static int PowerRemoved(ulong combatSeq, ulong powerInstance) => _power_removed(engine, combatSeq, powerInstance);
    internal static int PowerProvenanceInvalidate(ulong combatSeq, ulong powerInstance) => _power_provenance_invalidate(engine, combatSeq, powerInstance);
    internal static int CardGenerated(ulong combatSeq, ulong cardInstance, ulong sourceTransfer, int producerRole) => _card_generated(engine, combatSeq, cardInstance, sourceTransfer, producerRole);
    internal static ulong CardPlayStarted(ulong combatSeq, ulong executionId, ulong cardInstance, string cardId, int playerSlot, int playIndex, int playCount, int generationState, ulong sourceTransfer) => _card_play_started(engine, combatSeq, executionId, cardInstance, cardId, playerSlot, playIndex, playCount, generationState, sourceTransfer);
    internal static int CardPlayFinished(ulong play) => _card_play_finished(engine, play);
    internal static int CardExecutionEnded(ulong combatSeq, ulong executionId) => _card_execution_ended(engine, combatSeq, executionId);
    internal static int OrbChanneled(ulong combatSeq, ulong orbInstance, ulong sourceTransfer) => _orb_channeled(engine, combatSeq, orbInstance, sourceTransfer);
    internal static int OrbContextBegin(ulong combatSeq, ulong orbInstance, ulong play, int ownerSlot) => _orb_context_begin(engine, combatSeq, orbInstance, play, ownerSlot);
    internal static ulong DamageCalculationBegin(ulong combatSeq, ulong sourceTransfer, int producerRole, int segment, ulong originalTarget) => _damage_calculation_begin(engine, combatSeq, sourceTransfer, producerRole, segment, originalTarget);
    internal static int DamageModifierContribution(ulong calculation, ulong sourceTransfer, int amount) => _damage_modifier_contribution(engine, calculation, sourceTransfer, amount);
    internal static int DamageCalculationEnemyHit(ulong calculation, ulong dealerCreature, int baseDamage, int dealerStrength) => _damage_calculation_enemy_hit(engine, calculation, dealerCreature, baseDamage, dealerStrength);
    internal static int DamageCalculationWeakSource(ulong calculation, ulong sourceTransfer) => _damage_calculation_weak_source(engine, calculation, sourceTransfer);
    internal static int DamageResultAppend(ulong calculation, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented) => _damage_result_append(engine, calculation, total, unblocked, blocked, resultKind, receiverSlot, weakPrevented);
    internal static int DamageCalculationCommit(ulong calculation) => _damage_calculation_commit(engine, calculation);
    internal static int DamageCalculationAbort(ulong calculation) => _damage_calculation_abort(engine, calculation);
    internal static int DamageUnattributed(ulong combatSeq, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented) => _damage_unattributed(engine, combatSeq, total, unblocked, blocked, resultKind, receiverSlot, weakPrevented);
    internal static int BuffMitigation(ulong combatSeq, ulong sourceTransfer, int prevented) => _buff_mitigation(engine, combatSeq, sourceTransfer, prevented);
    internal static int BlockGained(ulong combatSeq, int amount, ulong sourceTransfer, int receiverSlot, BlockModifier[] modifiers, bool incomplete)
    {
        if (modifiers.Length == 0)
            return _block_gained(engine, combatSeq, amount, sourceTransfer, receiverSlot, IntPtr.Zero, 0, incomplete ? 1 : 0);
        var pinned = GCHandle.Alloc(modifiers, GCHandleType.Pinned);
        try
        {
            return _block_gained(engine, combatSeq, amount, sourceTransfer, receiverSlot, pinned.AddrOfPinnedObject(), modifiers.Length, incomplete ? 1 : 0);
        }
        finally { pinned.Free(); }
    }
    internal static int Forge(ulong combatSeq, ulong sourceTransfer, int amount) => _forge(engine, combatSeq, sourceTransfer, amount);
    internal static int OstySummoned(ulong combatSeq, ulong sourceTransfer, int hpAmount, int ownerSlot) => _osty_summoned(engine, combatSeq, sourceTransfer, hpAmount, ownerSlot);
    internal static int OstyKilled(ulong combatSeq, int ownerSlot, ulong play) => _osty_killed(engine, combatSeq, ownerSlot, play);
    internal static ulong DoomBatchBegin(ulong combatSeq) => _doom_batch_begin(engine, combatSeq);
    internal static int DoomTargetCapture(ulong batch, ulong creatureInstance, ulong doomPowerInstance, int currentHp) => _doom_target_capture(engine, batch, creatureInstance, doomPowerInstance, currentHp);
    internal static int DoomKillsCompleted(ulong batch) => _doom_kills_completed(engine, batch);
    internal static int DoomBatchAbort(ulong batch) => _doom_batch_abort(engine, batch);
    internal static int CombatEnded(ulong combatSeq) => _combat_ended(engine, combatSeq);
    internal static int TurnStarted(ulong combatSeq) => _turn_started(engine, combatSeq);
    internal static int BlockPoolClear(ulong combatSeq, int playerSlot) => _block_pool_clear(engine, combatSeq, playerSlot);
    internal static int PlayerDied(ulong combatSeq, int playerSlot) => _player_died(engine, combatSeq, playerSlot);
    internal static int PotionUsed(ulong combatSeq) => _potion_used(engine, combatSeq);
    internal static ulong CombatStarted(uint seq, string encounterId, string encounterType, long startedAt, int playerCount) => _combat_started(engine, seq, encounterId, encounterType, startedAt, playerCount);
    internal static ulong SourceAccumulate(ulong combatSeq, ulong first, int before, ulong second, int after) => _source_accumulate(engine, combatSeq, first, before, second, after);
    internal static void CaptureFailed(string reason) => _capture_failed(engine, reason);
}
