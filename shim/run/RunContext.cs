using System;
using System.Collections.Generic;
using System.Linq;
using MegaCrit.Sts2.Core.Context;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.Models.Relics;
using MegaCrit.Sts2.Core.Rooms;
using MegaCrit.Sts2.Core.Runs;
using MegaCrit.Sts2.Core.Saves;

namespace SpireProfiler;

internal static class RunContext
{
    /// <summary>
    /// The run's players in lobby order (host first) — the game's own slot
    /// numbering (RunState.GetPlayerSlotIndex is Players.IndexOf, identical
    /// on every peer). Captured at run start and immutable for the session;
    /// single player is a one-element list, so every slot resolves to 0.
    /// </summary>
    private static readonly List<Player> _runPlayers = new();
    private static RunState _runState;
    internal static object CurrentCombat => (_runState?.CurrentRoom as CombatRoom)?.CombatState;

    internal static int PlayerSlot(Player player)
    {
        if (player == null || _runPlayers.Count == 0) return 0;
        int index = _runPlayers.IndexOf(player);
        return index < 0 ? 0 : Math.Min(index, 3);
    }

    internal static uint PaperKraneSlots()
    {
        uint slots = 0;
        for (int index = 0; index < Math.Min(_runPlayers.Count, 4); index++)
            if (_runPlayers[index].GetRelic<PaperKrane>() != null) slots |= 1u << index;
        return slots;
    }

    internal static int CreditorSlot(Player player)
    {
        if (player == null) return ProfilerNative.TeamSlot;
        int index = _runPlayers.FindIndex(candidate => ReferenceEquals(candidate, player));
        return index >= 0 && index < 4 ? index : ProfilerNative.TeamSlot;
    }

    /// <summary>
    /// Refills the run-player registry from a run state. Every run start and
    /// resume re-fires NotifyRunStarted; run state is immutable during a
    /// run, so clear + refill is exact.
    /// </summary>
    internal static void CaptureRunPlayers(RunState state)
    {
        if (!CaptureRuntime.OnThread) return;
        _runState = state;
        _runPlayers.Clear();
        if (state?.Players != null) _runPlayers.AddRange(state.Players);
    }

    /// <summary>
    /// The only locality check left: the enemy→player hit capture in the
    /// ModifyDamage prefix, which feeds the LOCAL player's mitigation math
    /// in the core (that capture is intentionally local — see the patch).
    /// All other event forwarding is locality-free: every player's events
    /// are counted and slot-tagged instead.
    /// </summary>
    internal static bool IsLocalPlayer(Player player)
    {
        if (player == null) return false;
        return LocalContext.NetId.HasValue ? LocalContext.IsMe(player) : true;
    }

    /// Comma-joined character ids of the players in a run state (one id in
    /// single player, several in multiplayer).
    internal static string CharacterIds(RunState state)
    {
        if (state?.Players == null) return "";
        return string.Join(",", state.Players.Select(p => p.Character?.Id?.Entry ?? "?"));
    }

    /// Comma-joined player NetIds (ulong) of the players in a run state,
    /// positional with CharacterIds. The NetIds are the roster's
    /// cross-session identity — slots are session-immutable, NetIds survive
    /// — so the persistence side can match players across runs.
    internal static string NetIds(RunState state)
    {
        if (state?.Players == null) return "";
        return string.Join(",", state.Players.Select(p => p.NetId));
    }

    /// The active game profile id (SaveManager.CurrentProfileId), or -1
    /// when the save manager is not initialized yet (safe at run start).
    internal static int CurrentProfileId()
    {
        try { return SaveManager.Instance.CurrentProfileId; }
        catch (Exception) { return -1; }
    }
}
