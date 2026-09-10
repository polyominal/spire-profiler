using System;
using System.Runtime.InteropServices;

namespace SpireProfiler;

/// <summary>
/// C ABI bindings for the native core. Function pointers are resolved
/// explicitly so no DllImport probing rules apply; the native library is
/// loaded by absolute path from the mod directory.
/// </summary>
internal static class ProfilerNative
{
    internal const int TeamSlot = 4;
    internal const int PanelCombat = 0;
    internal const int PanelRun = 1;

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeInit([MarshalAs(UnmanagedType.LPUTF8Str)] string dataDir);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeSelfTest();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeSetRunMeta(int profileId);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeRunStarted([MarshalAs(UnmanagedType.LPUTF8Str)] string characterIds, int ascension, [MarshalAs(UnmanagedType.LPUTF8Str)] string gameMode, [MarshalAs(UnmanagedType.LPUTF8Str)] string seed, int continued, [MarshalAs(UnmanagedType.LPUTF8Str)] string netIds, long startTime);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeRunEnded(int outcome);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeRunSuspended();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeRunHistorySelect([MarshalAs(UnmanagedType.LPUTF8Str)] string seed, long startTime, int profile);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeRunHistoryClear();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativePanelToggle();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeScrollInput(int panel, int buttonIndex, int pressed, double panY);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeCombatStarted([MarshalAs(UnmanagedType.LPUTF8Str)] string encounterId, [MarshalAs(UnmanagedType.LPUTF8Str)] string encounterType);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCombatEnded(ulong combatSeq);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeTurnStarted(ulong combatSeq);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeBlockPoolClear(ulong combatSeq, int playerSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePlayerDied(ulong combatSeq, int playerSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePotionUsed(ulong combatSeq);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeSourceCapture(ulong combatSeq, int captureKind, ulong instance, [MarshalAs(UnmanagedType.LPUTF8Str)] string sourceId, int sourceKind, int sourceSlot, int generationState);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeSourceCount(ulong transfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeSourceDestination(ulong transfer, int index);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeSourceWeight(ulong transfer, int index);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeSourceTransferBegin(ulong combatSeq);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeSourceTransferAdd(ulong transfer, ulong destination, ulong weight);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeSourceTransferSeal(ulong transfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeSourceTransferRelease(ulong transfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerAttached(ulong combatSeq, ulong powerInstance, [MarshalAs(UnmanagedType.LPUTF8Str)] string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int amount, ulong sourceTransfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerAmountChanged(ulong combatSeq, ulong powerInstance, [MarshalAs(UnmanagedType.LPUTF8Str)] string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int oldAmount, int newAmount, ulong sourceTransfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerRemoved(ulong combatSeq, ulong powerInstance);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerProvenanceInvalidate(ulong combatSeq, ulong powerInstance);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCardGenerated(ulong combatSeq, ulong cardInstance, ulong sourceTransfer, int producerRole);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeCardPlayStarted(ulong combatSeq, ulong executionId, ulong cardInstance, [MarshalAs(UnmanagedType.LPUTF8Str)] string cardId, int playerSlot, int playIndex, int playCount, int generationState, ulong sourceTransfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCardPlayFinished(ulong play);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCardExecutionEnded(ulong combatSeq, ulong executionId);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOrbChanneled(ulong combatSeq, ulong orbInstance, ulong sourceTransfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOrbContextBegin(ulong combatSeq, ulong orbInstance, ulong play, int ownerSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeDamageCalculationBegin(ulong combatSeq, ulong sourceTransfer, int producerRole, int segment, ulong originalTarget);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageModifierContribution(ulong calculation, ulong sourceTransfer, int amount);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationEnemyHit(ulong calculation, ulong dealerCreature, int baseDamage, int dealerStrength);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationWeakSource(ulong calculation, ulong sourceTransfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageResultAppend(ulong calculation, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationCommit(ulong calculation);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationAbort(ulong calculation);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageUnattributed(ulong combatSeq, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeBuffMitigation(ulong combatSeq, ulong sourceTransfer, int prevented);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeBlockModifierContribution(ulong combatSeq, ulong sourceTransfer, int amount, int receiverSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeBlockGained(ulong combatSeq, int amount, ulong sourceTransfer, int receiverSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeForge(ulong combatSeq, ulong sourceTransfer, int amount);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOstySummoned(ulong combatSeq, ulong sourceTransfer, int hpAmount, int ownerSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOstyKilled(ulong combatSeq, int ownerSlot, ulong play);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeDoomBatchBegin(ulong combatSeq);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDoomTargetCapture(ulong batch, ulong creatureInstance, ulong doomPowerInstance, int currentHp);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDoomKillsCompleted(ulong batch);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDoomBatchAbort(ulong batch);

    private static NativeInit _init;
    private static NativeSelfTest _self_test;
    private static NativeSetRunMeta _set_run_meta;
    private static NativeRunStarted _run_started;
    private static NativeRunEnded _run_ended;
    private static NativeRunSuspended _run_suspended;
    private static NativeRunHistorySelect _run_history_select;
    private static NativeRunHistoryClear _run_history_clear;
    private static NativePanelToggle _panel_toggle;
    private static NativeScrollInput _scroll_input;
    private static NativeCombatStarted _combat_started;
    private static NativeCombatEnded _combat_ended;
    private static NativeTurnStarted _turn_started;
    private static NativeBlockPoolClear _block_pool_clear;
    private static NativePlayerDied _player_died;
    private static NativePotionUsed _potion_used;
    private static NativeSourceCapture _source_capture;
    private static NativeSourceCount _source_count;
    private static NativeSourceDestination _source_destination;
    private static NativeSourceWeight _source_weight;
    private static NativeSourceTransferBegin _source_transfer_begin;
    private static NativeSourceTransferAdd _source_transfer_add;
    private static NativeSourceTransferSeal _source_transfer_seal;
    private static NativeSourceTransferRelease _source_transfer_release;
    private static NativePowerAttached _power_attached;
    private static NativePowerAmountChanged _power_amount_changed;
    private static NativePowerRemoved _power_removed;
    private static NativePowerProvenanceInvalidate _power_provenance_invalidate;
    private static NativeCardGenerated _card_generated;
    private static NativeCardPlayStarted _card_play_started;
    private static NativeCardPlayFinished _card_play_finished;
    private static NativeCardExecutionEnded _card_execution_ended;
    private static NativeOrbChanneled _orb_channeled;
    private static NativeOrbContextBegin _orb_context_begin;
    private static NativeDamageCalculationBegin _damage_calculation_begin;
    private static NativeDamageModifierContribution _damage_modifier_contribution;
    private static NativeDamageCalculationEnemyHit _damage_calculation_enemy_hit;
    private static NativeDamageCalculationWeakSource _damage_calculation_weak_source;
    private static NativeDamageResultAppend _damage_result_append;
    private static NativeDamageCalculationCommit _damage_calculation_commit;
    private static NativeDamageCalculationAbort _damage_calculation_abort;
    private static NativeDamageUnattributed _damage_unattributed;
    private static NativeBuffMitigation _buff_mitigation;
    private static NativeBlockModifierContribution _block_modifier_contribution;
    private static NativeBlockGained _block_gained;
    private static NativeForge _forge;
    private static NativeOstySummoned _osty_summoned;
    private static NativeOstyKilled _osty_killed;
    private static NativeDoomBatchBegin _doom_batch_begin;
    private static NativeDoomTargetCapture _doom_target_capture;
    private static NativeDoomKillsCompleted _doom_kills_completed;
    private static NativeDoomBatchAbort _doom_batch_abort;

    private static T GetExport<T>(IntPtr lib, string name) where T : Delegate =>
        Marshal.GetDelegateForFunctionPointer<T>(NativeLibrary.GetExport(lib, name));

    internal static void Load(string path)
    {
        var lib = NativeLibrary.Load(path);
        _init = GetExport<NativeInit>(lib, "spire_profiler_init");
        _self_test = GetExport<NativeSelfTest>(lib, "spire_profiler_self_test");
        _set_run_meta = GetExport<NativeSetRunMeta>(lib, "spire_profiler_set_run_meta");
        _run_started = GetExport<NativeRunStarted>(lib, "spire_profiler_run_started");
        _run_ended = GetExport<NativeRunEnded>(lib, "spire_profiler_run_ended");
        _run_suspended = GetExport<NativeRunSuspended>(lib, "spire_profiler_run_suspended");
        _run_history_select = GetExport<NativeRunHistorySelect>(lib, "spire_profiler_run_history_select");
        _run_history_clear = GetExport<NativeRunHistoryClear>(lib, "spire_profiler_run_history_clear");
        _panel_toggle = GetExport<NativePanelToggle>(lib, "spire_profiler_panel_toggle");
        _scroll_input = GetExport<NativeScrollInput>(lib, "spire_profiler_scroll_input");
        _combat_started = GetExport<NativeCombatStarted>(lib, "spire_profiler_combat_started");
        _combat_ended = GetExport<NativeCombatEnded>(lib, "spire_profiler_combat_ended");
        _turn_started = GetExport<NativeTurnStarted>(lib, "spire_profiler_turn_started");
        _block_pool_clear = GetExport<NativeBlockPoolClear>(lib, "spire_profiler_block_pool_clear");
        _player_died = GetExport<NativePlayerDied>(lib, "spire_profiler_player_died");
        _potion_used = GetExport<NativePotionUsed>(lib, "spire_profiler_potion_used");
        _source_capture = GetExport<NativeSourceCapture>(lib, "spire_profiler_source_capture");
        _source_count = GetExport<NativeSourceCount>(lib, "spire_profiler_source_count");
        _source_destination = GetExport<NativeSourceDestination>(lib, "spire_profiler_source_destination");
        _source_weight = GetExport<NativeSourceWeight>(lib, "spire_profiler_source_weight");
        _source_transfer_begin = GetExport<NativeSourceTransferBegin>(lib, "spire_profiler_source_transfer_begin");
        _source_transfer_add = GetExport<NativeSourceTransferAdd>(lib, "spire_profiler_source_transfer_add");
        _source_transfer_seal = GetExport<NativeSourceTransferSeal>(lib, "spire_profiler_source_transfer_seal");
        _source_transfer_release = GetExport<NativeSourceTransferRelease>(lib, "spire_profiler_source_transfer_release");
        _power_attached = GetExport<NativePowerAttached>(lib, "spire_profiler_power_attached");
        _power_amount_changed = GetExport<NativePowerAmountChanged>(lib, "spire_profiler_power_amount_changed");
        _power_removed = GetExport<NativePowerRemoved>(lib, "spire_profiler_power_removed");
        _power_provenance_invalidate = GetExport<NativePowerProvenanceInvalidate>(lib, "spire_profiler_power_provenance_invalidate");
        _card_generated = GetExport<NativeCardGenerated>(lib, "spire_profiler_card_generated");
        _card_play_started = GetExport<NativeCardPlayStarted>(lib, "spire_profiler_card_play_started");
        _card_play_finished = GetExport<NativeCardPlayFinished>(lib, "spire_profiler_card_play_finished");
        _card_execution_ended = GetExport<NativeCardExecutionEnded>(lib, "spire_profiler_card_execution_ended");
        _orb_channeled = GetExport<NativeOrbChanneled>(lib, "spire_profiler_orb_channeled");
        _orb_context_begin = GetExport<NativeOrbContextBegin>(lib, "spire_profiler_orb_context_begin");
        _damage_calculation_begin = GetExport<NativeDamageCalculationBegin>(lib, "spire_profiler_damage_calculation_begin");
        _damage_modifier_contribution = GetExport<NativeDamageModifierContribution>(lib, "spire_profiler_damage_modifier_contribution");
        _damage_calculation_enemy_hit = GetExport<NativeDamageCalculationEnemyHit>(lib, "spire_profiler_damage_calculation_enemy_hit");
        _damage_calculation_weak_source = GetExport<NativeDamageCalculationWeakSource>(lib, "spire_profiler_damage_calculation_weak_source");
        _damage_result_append = GetExport<NativeDamageResultAppend>(lib, "spire_profiler_damage_result_append");
        _damage_calculation_commit = GetExport<NativeDamageCalculationCommit>(lib, "spire_profiler_damage_calculation_commit");
        _damage_calculation_abort = GetExport<NativeDamageCalculationAbort>(lib, "spire_profiler_damage_calculation_abort");
        _damage_unattributed = GetExport<NativeDamageUnattributed>(lib, "spire_profiler_damage_unattributed");
        _buff_mitigation = GetExport<NativeBuffMitigation>(lib, "spire_profiler_buff_mitigation");
        _block_modifier_contribution = GetExport<NativeBlockModifierContribution>(lib, "spire_profiler_block_modifier_contribution");
        _block_gained = GetExport<NativeBlockGained>(lib, "spire_profiler_block_gained");
        _forge = GetExport<NativeForge>(lib, "spire_profiler_forge");
        _osty_summoned = GetExport<NativeOstySummoned>(lib, "spire_profiler_osty_summoned");
        _osty_killed = GetExport<NativeOstyKilled>(lib, "spire_profiler_osty_killed");
        _doom_batch_begin = GetExport<NativeDoomBatchBegin>(lib, "spire_profiler_doom_batch_begin");
        _doom_target_capture = GetExport<NativeDoomTargetCapture>(lib, "spire_profiler_doom_target_capture");
        _doom_kills_completed = GetExport<NativeDoomKillsCompleted>(lib, "spire_profiler_doom_kills_completed");
        _doom_batch_abort = GetExport<NativeDoomBatchAbort>(lib, "spire_profiler_doom_batch_abort");
    }

    internal static void Init(string dataDir) => _init(dataDir);
    internal static void SelfTest() => _self_test();
    internal static void SetRunMeta(int profileId) => _set_run_meta(profileId);
    internal static void RunStarted(string characterIds, int ascension, string gameMode, string seed, int continued, string netIds, long startTime) => _run_started(characterIds, ascension, gameMode, seed, continued, netIds, startTime);
    internal static void RunEnded(int outcome) => _run_ended(outcome);
    internal static void RunSuspended() => _run_suspended();
    internal static void RunHistorySelect(string seed, long startTime, int profile) => _run_history_select(seed, startTime, profile);
    internal static void RunHistoryClear() => _run_history_clear();
    internal static void PanelToggle() => _panel_toggle();
    internal static void ScrollInput(int panel, int buttonIndex, int pressed, double panY) => _scroll_input(panel, buttonIndex, pressed, panY);
    internal static ulong CombatStarted(string encounterId, string encounterType) => _combat_started(encounterId, encounterType);
    internal static int CombatEnded(ulong combatSeq) => _combat_ended(combatSeq);
    internal static int TurnStarted(ulong combatSeq) => _turn_started(combatSeq);
    internal static int BlockPoolClear(ulong combatSeq, int playerSlot) => _block_pool_clear(combatSeq, playerSlot);
    internal static int PlayerDied(ulong combatSeq, int playerSlot) => _player_died(combatSeq, playerSlot);
    internal static int PotionUsed(ulong combatSeq) => _potion_used(combatSeq);
    internal static ulong SourceCapture(ulong combatSeq, int captureKind, ulong instance, string sourceId, int sourceKind, int sourceSlot, int generationState) => _source_capture(combatSeq, captureKind, instance, sourceId, sourceKind, sourceSlot, generationState);
    internal static int SourceCount(ulong transfer) => _source_count(transfer);
    internal static ulong SourceDestination(ulong transfer, int index) => _source_destination(transfer, index);
    internal static ulong SourceWeight(ulong transfer, int index) => _source_weight(transfer, index);
    internal static ulong SourceTransferBegin(ulong combatSeq) => _source_transfer_begin(combatSeq);
    internal static int SourceTransferAdd(ulong transfer, ulong destination, ulong weight) => _source_transfer_add(transfer, destination, weight);
    internal static int SourceTransferSeal(ulong transfer) => _source_transfer_seal(transfer);
    internal static int SourceTransferRelease(ulong transfer) => _source_transfer_release(transfer);
    internal static int PowerAttached(ulong combatSeq, ulong powerInstance, string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int amount, ulong sourceTransfer) => _power_attached(combatSeq, powerInstance, powerId, ownerCreature, ownerKind, ownerSlot, amount, sourceTransfer);
    internal static int PowerAmountChanged(ulong combatSeq, ulong powerInstance, string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int oldAmount, int newAmount, ulong sourceTransfer) => _power_amount_changed(combatSeq, powerInstance, powerId, ownerCreature, ownerKind, ownerSlot, oldAmount, newAmount, sourceTransfer);
    internal static int PowerRemoved(ulong combatSeq, ulong powerInstance) => _power_removed(combatSeq, powerInstance);
    internal static int PowerProvenanceInvalidate(ulong combatSeq, ulong powerInstance) => _power_provenance_invalidate(combatSeq, powerInstance);
    internal static int CardGenerated(ulong combatSeq, ulong cardInstance, ulong sourceTransfer, int producerRole) => _card_generated(combatSeq, cardInstance, sourceTransfer, producerRole);
    internal static ulong CardPlayStarted(ulong combatSeq, ulong executionId, ulong cardInstance, string cardId, int playerSlot, int playIndex, int playCount, int generationState, ulong sourceTransfer) => _card_play_started(combatSeq, executionId, cardInstance, cardId, playerSlot, playIndex, playCount, generationState, sourceTransfer);
    internal static int CardPlayFinished(ulong play) => _card_play_finished(play);
    internal static int CardExecutionEnded(ulong combatSeq, ulong executionId) => _card_execution_ended(combatSeq, executionId);
    internal static int OrbChanneled(ulong combatSeq, ulong orbInstance, ulong sourceTransfer) => _orb_channeled(combatSeq, orbInstance, sourceTransfer);
    internal static int OrbContextBegin(ulong combatSeq, ulong orbInstance, ulong play, int ownerSlot) => _orb_context_begin(combatSeq, orbInstance, play, ownerSlot);
    internal static ulong DamageCalculationBegin(ulong combatSeq, ulong sourceTransfer, int producerRole, int segment, ulong originalTarget) => _damage_calculation_begin(combatSeq, sourceTransfer, producerRole, segment, originalTarget);
    internal static int DamageModifierContribution(ulong calculation, ulong sourceTransfer, int amount) => _damage_modifier_contribution(calculation, sourceTransfer, amount);
    internal static int DamageCalculationEnemyHit(ulong calculation, ulong dealerCreature, int baseDamage, int dealerStrength) => _damage_calculation_enemy_hit(calculation, dealerCreature, baseDamage, dealerStrength);
    internal static int DamageCalculationWeakSource(ulong calculation, ulong sourceTransfer) => _damage_calculation_weak_source(calculation, sourceTransfer);
    internal static int DamageResultAppend(ulong calculation, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented) => _damage_result_append(calculation, total, unblocked, blocked, resultKind, receiverSlot, weakPrevented);
    internal static int DamageCalculationCommit(ulong calculation) => _damage_calculation_commit(calculation);
    internal static int DamageCalculationAbort(ulong calculation) => _damage_calculation_abort(calculation);
    internal static int DamageUnattributed(ulong combatSeq, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented) => _damage_unattributed(combatSeq, total, unblocked, blocked, resultKind, receiverSlot, weakPrevented);
    internal static int BuffMitigation(ulong combatSeq, ulong sourceTransfer, int prevented) => _buff_mitigation(combatSeq, sourceTransfer, prevented);
    internal static int BlockModifierContribution(ulong combatSeq, ulong sourceTransfer, int amount, int receiverSlot) => _block_modifier_contribution(combatSeq, sourceTransfer, amount, receiverSlot);
    internal static int BlockGained(ulong combatSeq, int amount, ulong sourceTransfer, int receiverSlot) => _block_gained(combatSeq, amount, sourceTransfer, receiverSlot);
    internal static int Forge(ulong combatSeq, ulong sourceTransfer, int amount) => _forge(combatSeq, sourceTransfer, amount);
    internal static int OstySummoned(ulong combatSeq, ulong sourceTransfer, int hpAmount, int ownerSlot) => _osty_summoned(combatSeq, sourceTransfer, hpAmount, ownerSlot);
    internal static int OstyKilled(ulong combatSeq, int ownerSlot, ulong play) => _osty_killed(combatSeq, ownerSlot, play);
    internal static ulong DoomBatchBegin(ulong combatSeq) => _doom_batch_begin(combatSeq);
    internal static int DoomTargetCapture(ulong batch, ulong creatureInstance, ulong doomPowerInstance, int currentHp) => _doom_target_capture(batch, creatureInstance, doomPowerInstance, currentHp);
    internal static int DoomKillsCompleted(ulong batch) => _doom_kills_completed(batch);
    internal static int DoomBatchAbort(ulong batch) => _doom_batch_abort(batch);

    internal static void OnSetRunMeta(int profile) { if (CaptureRuntime.OnThread) SetRunMeta(profile); }
    internal static void OnRunStarted(string ids, int ascension, string mode, string seed, bool continued, string netIds, long startedAt)
    { if (CaptureRuntime.OnThread) RunStarted(ids, ascension, mode, seed, continued ? 1 : 0, netIds, startedAt); }
    internal static void OnRunEnded(int outcome) { if (CaptureRuntime.OnThread) RunEnded(outcome); }
    internal static void OnRunSuspended() { if (CaptureRuntime.OnThread) RunSuspended(); }
    internal static void OnRunHistorySelect(string seed, long startedAt, int profile) { if (CaptureRuntime.OnThread) RunHistorySelect(seed, startedAt, profile); }
    internal static void OnRunHistoryClear() { if (CaptureRuntime.OnThread) RunHistoryClear(); }
    internal static void OnPanelToggle() { if (CaptureRuntime.OnThread) PanelToggle(); }
    internal static void OnScrollInput(int panel, int button, bool pressed, double pan) { if (CaptureRuntime.OnThread) ScrollInput(panel, button, pressed ? 1 : 0, pan); }
}
