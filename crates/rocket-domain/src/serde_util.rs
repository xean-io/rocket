//! Serde helpers that reproduce Go `encoding/json` behaviour.

use serde::{Deserialize, Deserializer, Serializer};
use time::OffsetDateTime;
use time::macros::datetime;

/// Go's zero `time.Time`, used when a non-pointer time is absent.
pub(crate) fn zero_time() -> OffsetDateTime {
    datetime!(0001-01-01 00:00:00 UTC)
}

/// Decode `null` as the type's default (Go ignores `null` for slices/maps).
pub(crate) fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// `time.Duration` is an int64 nanosecond count in JSON.
pub(crate) mod duration_ns {
    use super::*;
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
    }

    /// Negative values (valid in Go) clamp to zero: every consumer treats
    /// non-positive timeouts as "use the default".
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ns = Option::<i64>::deserialize(d)?.unwrap_or(0);
        Ok(Duration::from_nanos(u64::try_from(ns).unwrap_or(0)))
    }
}
