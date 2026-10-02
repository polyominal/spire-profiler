//! Journal syntax is parsed once. Missing checkpoints and unsupported numeric
//! domains remain evidence limitations; malformed field representations fail
//! before a report is published. Raw JSON survives only as display text.

use serde::Deserialize;

use super::*;

pub(super) struct Event {
    pub(super) seq: u64,
    pub(super) turn: u32,
    pub(super) data: Data,
    pub(super) native: Option<Result<Native, String>>,
    facts: String,
}

pub(super) enum Data {
    Start(Header),
    End(Footer),
    Frame(SourceFrame),
    Attempt(Attempt),
    Change(Mutation),
    CommandEnd(CommandEnd),
    Envenom(Envenom),
    EnvenomEnd(EnvenomEnd),
    Tick(Tick),
    PoisonDamage(Damage),
    Damage(Damage),
    DamageBegin(DamageBegin),
    PoisonAttempt(PoisonAttempt),
    PoisonChange(Mutation),
    Other(String),
}

#[derive(Deserialize)]
pub(super) struct Header {
    pub(super) audit_version: u64,
    pub(super) policy_version: u64,
    pub(super) epoch: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct Footer {
    pub(super) complete: Option<bool>,
}

#[derive(Deserialize)]
pub(super) struct Creature {
    pub(super) instance: Option<u64>,
    pub(super) native_instance: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct RawSource {
    pub(super) identity: u64,
    pub(super) model: String,
    pub(super) role: String,
    pub(super) origin: String,
    pub(super) player: u64,
}

#[derive(Deserialize)]
pub(super) struct SourceFrame {
    pub(super) source: RawSource,
    pub(super) power_identity: u64,
    pub(super) target: Option<Creature>,
    pub(super) amount: Amount,
    pub(super) context: String,
}

#[derive(Deserialize)]
pub(super) struct Attempt {
    pub(super) command: String,
    pub(super) power: String,
    pub(super) power_identity: u64,
    pub(super) target: Option<Creature>,
    pub(super) source: RawSource,
    pub(super) source_frame: u64,
    pub(super) cause: u64,
    pub(super) amount: Amount,
}

#[derive(Deserialize)]
pub(super) struct PoisonAttempt {
    pub(super) power_identity: Option<u64>,
    pub(super) target: Option<Creature>,
}

#[derive(Deserialize)]
pub(super) struct Mutation {
    pub(super) power: Option<String>,
    pub(super) power_identity: Option<u64>,
    pub(super) power_instance: Option<u64>,
    pub(super) target: Option<Creature>,
    pub(super) action: Option<u64>,
    pub(super) source_frame: Option<u64>,
    pub(super) before: Amount,
    pub(super) after: Amount,
    pub(super) before_attached: Option<bool>,
    pub(super) after_attached: bool,
}

#[derive(Deserialize)]
pub(super) struct CommandEnd {
    pub(super) action: u64,
    pub(super) command: String,
    pub(super) outcome: String,
    pub(super) requested_power_identity: u64,
    pub(super) power_identity: u64,
    pub(super) target: Option<Creature>,
}

#[derive(Deserialize)]
pub(super) struct Envenom {
    pub(super) power_identity: u64,
    pub(super) source_frame: u64,
    pub(super) owner: Option<Creature>,
    pub(super) dealer: Option<Creature>,
    pub(super) target: Option<Creature>,
    pub(super) result_identity: u64,
    pub(super) unblocked: Amount,
    pub(super) props: u64,
    pub(super) amount: Amount,
}

#[derive(Deserialize)]
pub(super) struct EnvenomEnd {
    pub(super) trigger: u64,
    pub(super) outcome: String,
}

#[derive(Deserialize)]
pub(super) struct Tick {
    pub(super) power_identity: Option<u64>,
    pub(super) power_instance: u64,
    pub(super) source_frame: Option<u64>,
    pub(super) target: Option<Creature>,
    pub(super) dealer: Option<Creature>,
    pub(super) props: Option<u64>,
    pub(super) requested: Option<Amount>,
    pub(super) poison_before: Option<Amount>,
    pub(super) hp_before: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct Damage {
    pub(super) tick_seq: u64,
    pub(super) result_identity: Option<u64>,
    pub(super) target: Option<Creature>,
    pub(super) unblocked: Amount,
    pub(super) blocked: Amount,
    pub(super) hp_after: Option<u64>,
    pub(super) group_index: Option<u64>,
    pub(super) group_count: Option<u64>,
    pub(super) receiver_side: Option<String>,
    pub(super) receiver_kind: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct DamageBegin {
    pub(super) dealer: Option<Creature>,
    pub(super) props: u64,
}

/// Game decimal requests may be fractional or negative. Parse their syntax once;
/// the supported nonnegative integral domain is selected only where needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Amount(Option<u64>);

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let text = match value {
            Value::String(text) => text,
            Value::Number(number) => number.to_string(),
            _ => return Err(serde::de::Error::custom("expected a numeric amount")),
        };
        let digits = text.strip_prefix('-').unwrap_or(&text);
        let mut parts = digits.split('.');
        let whole = parts.next().unwrap_or_default();
        let fraction = parts.next();
        if whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || fraction.is_some_and(|part| {
                part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit())
            })
            || parts.next().is_some()
        {
            return Err(serde::de::Error::custom("invalid decimal amount"));
        }
        let integral = !text.starts_with('-')
            && fraction.is_none_or(|part| part.bytes().all(|byte| byte == b'0'));
        Ok(Self(if integral {
            whole
                .parse::<u64>()
                .ok()
                .filter(|amount| *amount <= i32::MAX as u64)
        } else {
            None
        }))
    }
}

impl Amount {
    pub(super) fn nonnegative(self) -> Result<u64> {
        self.0.ok_or_else(|| {
            anyhow!("Amount is outside the supported nonnegative integral game domain")
        })
    }
}

impl Event {
    pub(super) fn read(seq: u64, turn: u32, name: &str, raw: Value) -> Result<Self> {
        let object = raw
            .as_object()
            .ok_or_else(|| anyhow!("event data must be an object"))?;
        let facts = object
            .iter()
            .filter(|(key, _)| key.as_str() != "native")
            .map(|(key, value)| format!("{}={}", markdown(key), markdown(&value.to_string())))
            .collect::<Vec<_>>()
            .join("; ");
        let native = raw
            .get("native")
            .filter(|native| !native.is_null())
            .map(|native| -> Result<_> {
                Trace::checkpoint_version(native)?;
                Ok(Native::read(native).map_err(|error| error.to_string()))
            })
            .transpose()?;
        let data = match name {
            "combat_start" => Data::Start(serde_json::from_value(raw)?),
            "combat_end" => Data::End(serde_json::from_value(raw)?),
            "source_frame" => Data::Frame(serde_json::from_value(raw)?),
            "power_attempt" => Data::Attempt(serde_json::from_value(raw)?),
            "power_change" => Data::Change(serde_json::from_value(raw)?),
            "command_end" => Data::CommandEnd(serde_json::from_value(raw)?),
            "envenom_trigger" => Data::Envenom(serde_json::from_value(raw)?),
            "envenom_end" => Data::EnvenomEnd(serde_json::from_value(raw)?),
            "poison_tick" => Data::Tick(serde_json::from_value(raw)?),
            "poison_damage" => Data::PoisonDamage(serde_json::from_value(raw)?),
            "damage_result" => Data::Damage(serde_json::from_value(raw)?),
            "damage_begin" => Data::DamageBegin(serde_json::from_value(raw)?),
            "poison_attempt" => Data::PoisonAttempt(serde_json::from_value(raw)?),
            "poison_change" => Data::PoisonChange(serde_json::from_value(raw)?),
            _ => Data::Other(name.to_owned()),
        };
        if let Data::Start(header) = &data {
            ensure!(
                matches!(header.audit_version, 1 | 2),
                "unsupported audit version"
            );
            ensure!(
                matches!(header.policy_version, 3 | 4),
                "unsupported attribution policy version"
            );
        }
        Ok(Self {
            seq,
            turn,
            data,
            native,
            facts,
        })
    }

    pub(super) fn name(&self) -> &str {
        match &self.data {
            Data::Start(_) => "combat_start",
            Data::End(_) => "combat_end",
            Data::Frame(_) => "source_frame",
            Data::Attempt(_) => "power_attempt",
            Data::Change(_) => "power_change",
            Data::CommandEnd(_) => "command_end",
            Data::Envenom(_) => "envenom_trigger",
            Data::EnvenomEnd(_) => "envenom_end",
            Data::Tick(_) => "poison_tick",
            Data::PoisonDamage(_) => "poison_damage",
            Data::Damage(_) => "damage_result",
            Data::DamageBegin(_) => "damage_begin",
            Data::PoisonAttempt(_) => "poison_attempt",
            Data::PoisonChange(_) => "poison_change",
            Data::Other(name) => name,
        }
    }

    pub(super) fn epoch(&self) -> Option<u64> {
        if let Data::Start(header) = &self.data {
            header.epoch
        } else {
            None
        }
    }

    pub(super) fn facts(&self) -> &str {
        &self.facts
    }

    pub(super) fn checkpoint(&self) -> Result<&Native> {
        self.native
            .as_ref()
            .ok_or_else(|| anyhow!("Native checkpoint unavailable"))?
            .as_ref()
            .map_err(|error| anyhow!(error.clone()))
    }

    pub(super) fn damage_data(&self) -> Option<&Damage> {
        match &self.data {
            Data::Damage(data) | Data::PoisonDamage(data) => Some(data),
            _ => None,
        }
    }
}

impl Creature {
    pub(super) fn instance(creature: &Option<Self>) -> Option<u64> {
        creature
            .as_ref()
            .and_then(|creature| creature.instance)
            .filter(|id| *id != 0)
    }

    pub(super) fn native(creature: &Option<Self>) -> Option<u64> {
        creature
            .as_ref()
            .and_then(|creature| creature.native_instance)
            .filter(|id| *id != 0)
    }
}
