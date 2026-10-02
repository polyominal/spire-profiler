use super::SourceFailure;

macro_rules! wire_enum {
    ($name:ident, [$($derive:ident),*], {$($variant:ident = $value:literal),+ $(,)?}) => {
        #[repr(i32)]
        #[derive($($derive),*)]
        pub(super) enum $name { $($variant = $value),+ }
        impl $name {
            pub(super) fn decode(value: i32) -> Result<Self, SourceFailure> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(SourceFailure::Packet),
                }
            }
        }
        $(const _: () = assert!($name::$variant as i32 == $value);)+
    };
}

wire_enum!(ProducerRole, [Debug, PartialEq], { Unknown = 0, Card = 1, Power = 2, Relic = 3, Potion = 4, Orb = 5 });
wire_enum!(SourceCaptureKind, [PartialEq], { Unknown = 0, CardInstance = 1, PowerInstance = 2, OrbInstance = 3, DirectModel = 4, WeakHead = 5 });
wire_enum!(GenerationState, [Clone, Copy, PartialEq], { Ordinary = 0, GeneratedRecorded = 1, GeneratedUnavailable = 2, Unclassified = 3 });
wire_enum!(CreatureKind, [Clone, Copy, PartialEq], { Player = 0, Enemy = 1, OwnedPet = 2, Other = 3 });
wire_enum!(DamageSegment, [Clone, Copy, Debug, PartialEq], { Direct = 0, Attributed = 1, Modifier = 2 });
wire_enum!(ResultKind, [PartialEq], { Outgoing = 0, Incoming = 1, SelfDamage = 2, OstyDealt = 3, OstyAbsorbed = 4 });
