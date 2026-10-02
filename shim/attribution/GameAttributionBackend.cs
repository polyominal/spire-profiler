using System.Linq;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Powers;

namespace SpireProfiler;

internal abstract class GameAttributionBackend : AttributionBackend
{
    internal override ModelDescriptor Describe(object model) => model switch
    {
        CardModel card => new(ProducerRole.Card, card.Id?.Entry ?? "", RunContext.CreditorSlot(card.Owner), Combat: card.Owner?.Creature?.CombatState),
        PowerModel power => new(ProducerRole.Power, power.Id?.Entry ?? "", 4, power.Owner, power is PoisonPower, power.Owner?.CombatState),
        RelicModel relic => new(ProducerRole.Relic, relic.Id?.Entry ?? "", RunContext.CreditorSlot(relic.Owner), Combat: relic.Owner?.Creature?.CombatState),
        PotionModel potion => new(ProducerRole.Potion, potion.Id?.Entry ?? "", RunContext.CreditorSlot(potion.Owner), Combat: potion.Owner?.Creature?.CombatState),
        OrbModel orb => new(ProducerRole.Orb, orb.Id?.Entry ?? "", RunContext.PlayerSlot(orb.Owner), Combat: orb.Owner?.Creature?.CombatState),
        _ => new(ProducerRole.Unknown, "", 4)
    };
    internal override CreatureDescriptor DescribeCreature(object model)
    {
        var creature = (MegaCrit.Sts2.Core.Entities.Creatures.Creature)model;
        return new(creature.IsPlayer, creature.Monster is MegaCrit.Sts2.Core.Models.Monsters.Osty,
            creature.IsPlayer ? RunContext.PlayerSlot(creature.Player) : creature.PetOwner != null ? RunContext.PlayerSlot(creature.PetOwner) : 4, creature.CombatState);
    }
    internal override PowerObservation ObservePower(object model, object owner = null)
    {
        var power = (PowerModel)model;
        var creature = owner as MegaCrit.Sts2.Core.Entities.Creatures.Creature ?? power.Owner;
        bool attached = creature != null && creature.Powers.Any(p => ReferenceEquals(p, power));
        int kind = creature == null ? 3 : creature.IsPlayer ? 0 : creature.IsPet ? 2 : creature.IsMonster ? 1 : 3;
        int slot = creature?.IsPlayer == true ? RunContext.PlayerSlot(creature.Player) : creature?.PetOwner != null ? RunContext.PlayerSlot(creature.PetOwner) : 4;
        return new(power, creature, power.Id?.Entry ?? "", kind, slot, power.Amount, attached);
    }
    internal override bool TemporaryPower(object power) => power is TemporaryStrengthPower or TemporaryFocusPower or TemporaryDexterityPower;
}
