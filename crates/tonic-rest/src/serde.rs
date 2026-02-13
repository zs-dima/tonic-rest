//! Serde adapters for prost well-known types and proto3 enums.
//!
//! These adapters bridge `prost_types` types (which lack native `Serialize`/`Deserialize`)
//! to their canonical protobuf JSON representations:
//!
//! - **Timestamp** → RFC 3339 string (`"2025-01-15T09:30:00Z"`)
//! - **Duration**  → seconds string with `s` suffix (`"300s"`)
//! - **`FieldMask`** → comma-separated camelCase paths (`"name,email,role"`)
//!
//! ## Proto enums
//!
//! Proto3 enum fields are `i32` in prost. The [`define_enum_serde`] macro generates
//! `#[serde(with)]` modules that serialize as proto enum name strings
//! (e.g., `"USER_ROLE_ADMIN"`) following Google's protobuf JSON mapping.
//!
//! # Usage
//!
//! Wire these adapters via `#[serde(with = "...")]` on prost-generated fields,
//! or use `tonic_rest_build::configure_prost_serde` to auto-discover and
//! apply them at build time.
//!
//! ```ignore
//! // In build.rs (via tonic-rest-build):
//! tonic_rest_build::configure_prost_serde(&mut config, &fds, "crate::serde_wkt", &wkt_map, &enum_map);
//!
//! // In lib.rs:
//! pub mod serde_wkt {
//!     pub use tonic_rest::serde::{opt_timestamp, opt_duration, opt_field_mask};
//!     tonic_rest::define_enum_serde!(user_role, crate::core::UserRole);
//! }
//! ```

/// Serde adapter for `Option<prost_types::Timestamp>` ↔ RFC 3339 string.
///
/// Serializes `Some(Timestamp)` as an RFC 3339 string (e.g., `"2025-01-15T09:30:00Z"`),
/// and `None` as JSON `null`. Follows the canonical protobuf JSON mapping.
///
/// # Errors
///
/// Serialization fails if the timestamp has negative nanos or is out of range.
/// Deserialization fails if the string is not a valid RFC 3339 datetime.
pub mod opt_timestamp {
    use prost_types::Timestamp;
    use serde::{self, Deserialize, Deserializer, Serializer};

    /// Serialize an optional `Timestamp` as an RFC 3339 string.
    ///
    /// # Errors
    ///
    /// Returns `S::Error` if the timestamp is out of range or has negative nanos.
    pub fn serialize<S>(value: &Option<Timestamp>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(ts) => super::timestamp::serialize(ts, serializer),
            None => serializer.serialize_none(),
        }
    }

    /// Deserialize an optional `Timestamp` from an RFC 3339 string.
    ///
    /// # Errors
    ///
    /// Returns `D::Error` if the string is not a valid RFC 3339 datetime.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Timestamp>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt {
            Some(s) => super::timestamp::deserialize_str(&s)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

/// Serde adapter for `prost_types::Timestamp` ↔ RFC 3339 string.
///
/// Serializes a `Timestamp` as an RFC 3339 string (e.g., `"2025-01-15T09:30:00Z"`).
/// Follows the canonical protobuf JSON mapping.
///
/// Use [`opt_timestamp`] for `Option<Timestamp>` fields.
///
/// # Examples
///
/// ```
/// use serde::{Serialize, Deserialize};
/// use prost_types::Timestamp;
///
/// #[derive(Serialize, Deserialize)]
/// struct Event {
///     #[serde(with = "tonic_rest::serde::timestamp")]
///     created_at: Timestamp,
/// }
///
/// let event = Event { created_at: Timestamp { seconds: 1_736_934_600, nanos: 0 } };
/// let json = serde_json::to_string(&event).unwrap();
/// assert!(json.contains("2025-01-15"));
/// ```
/// # Errors
///
/// Serialization fails if the timestamp has negative nanos or is out of range.
/// Deserialization fails if the string is not a valid RFC 3339 datetime.
pub mod timestamp {
    use prost_types::Timestamp;
    use serde::{self, Deserializer, Serializer};

    /// Serialize a `Timestamp` as an RFC 3339 string.
    ///
    /// # Errors
    ///
    /// Returns `S::Error` if the timestamp is out of range or has negative nanos.
    pub fn serialize<S>(value: &Timestamp, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let nanos = u32::try_from(value.nanos)
            .map_err(|_| serde::ser::Error::custom("negative nanos in Timestamp"))?;
        let dt = chrono::DateTime::from_timestamp(value.seconds, nanos)
            .ok_or_else(|| serde::ser::Error::custom("timestamp out of range"))?;
        serializer.serialize_str(&dt.to_rfc3339())
    }

    /// Parse a `Timestamp` from an RFC 3339 string.
    pub(crate) fn deserialize_str(s: &str) -> Result<Timestamp, String> {
        let dt = chrono::DateTime::parse_from_rfc3339(s).map_err(|e| e.to_string())?;
        // Safety: `timestamp_subsec_nanos()` returns 0..=999_999_999 which
        // always fits in `i32` (max 2_147_483_647).
        #[allow(clippy::cast_possible_wrap)]
        Ok(Timestamp {
            seconds: dt.timestamp(),
            nanos: dt.timestamp_subsec_nanos() as i32,
        })
    }

    /// Deserialize a `Timestamp` from an RFC 3339 string.
    ///
    /// # Errors
    ///
    /// Returns `D::Error` if the string is not a valid RFC 3339 datetime.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Timestamp, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::Deserialize;
        let s = String::deserialize(deserializer)?;
        deserialize_str(&s).map_err(serde::de::Error::custom)
    }
}

/// Serde adapter for `Option<prost_types::Duration>` ↔ seconds string with `s` suffix.
///
/// Follows the protobuf JSON mapping: `"300s"`, `"1.500s"`, `"0s"`.
/// Per proto3 spec, `seconds` and `nanos` must have the same sign.
///
/// # Errors
///
/// Deserialization fails if the string cannot be parsed as a duration
/// (e.g., missing `s` suffix, non-numeric parts, fractional part exceeding
/// nanosecond precision).
pub mod opt_duration {
    use prost_types::Duration;
    use serde::{self, Deserialize, Deserializer, Serializer};

    /// Serialize an optional `Duration` as a seconds string with `s` suffix.
    ///
    /// # Errors
    ///
    /// Returns `S::Error` if serialization fails.
    pub fn serialize<S>(value: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(d) => super::duration::serialize(d, serializer),
            None => serializer.serialize_none(),
        }
    }

    /// Deserialize an optional `Duration` from a seconds string with `s` suffix.
    ///
    /// # Errors
    ///
    /// Returns `D::Error` if the string is not a valid duration.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt {
            Some(s) => super::duration::deserialize_str(&s)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

/// Serde adapter for `prost_types::Duration` ↔ seconds string with `s` suffix.
///
/// Follows the protobuf JSON mapping: `"300s"`, `"1.500s"`, `"0s"`.
/// Per proto3 spec, `seconds` and `nanos` must have the same sign.
///
/// # Panics
///
/// Does not panic, but produces incorrect output for non-canonical `Duration`
/// values where `seconds` and `nanos` have different signs (e.g.,
/// `Duration { seconds: 5, nanos: -100 }`). The proto3 spec guarantees
/// same-sign values; this adapter relies on that invariant.
///
/// Use [`opt_duration`] for `Option<Duration>` fields.
///
/// # Examples
///
/// ```
/// use serde::{Serialize, Deserialize};
/// use prost_types::Duration;
///
/// #[derive(Serialize, Deserialize)]
/// struct Config {
///     #[serde(with = "tonic_rest::serde::duration")]
///     timeout: Duration,
/// }
///
/// let config = Config { timeout: Duration { seconds: 300, nanos: 0 } };
/// let json = serde_json::to_string(&config).unwrap();
/// assert!(json.contains("300s"));
/// ```
/// # Errors
///
/// Deserialization fails if the string cannot be parsed as a duration
/// (e.g., missing `s` suffix, non-numeric parts, fractional part exceeding
/// nanosecond precision).
pub mod duration {
    use prost_types::Duration;
    use serde::{self, Deserialize, Deserializer, Serializer};

    /// Serialize a `Duration` as a seconds string with `s` suffix.
    ///
    /// # Errors
    ///
    /// Returns `S::Error` if serialization fails.
    pub fn serialize<S>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let negative = value.seconds < 0 || value.nanos < 0;
        let abs_secs = value.seconds.unsigned_abs();
        let abs_nanos = value.nanos.unsigned_abs();

        if abs_nanos == 0 {
            let sign = if negative { "-" } else { "" };
            serializer.serialize_str(&format!("{sign}{abs_secs}s"))
        } else {
            let sign = if negative { "-" } else { "" };
            let frac = format!("{abs_nanos:09}");
            let trimmed = frac.trim_end_matches('0');
            serializer.serialize_str(&format!("{sign}{abs_secs}.{trimmed}s"))
        }
    }

    /// Parse a `Duration` from a seconds string with `s` suffix.
    ///
    /// **Leniency note:** The `s` suffix is optional during deserialization.
    /// Input `"300"` is accepted as 300 seconds, even though the proto JSON
    /// spec requires `"300s"`. This improves interoperability with clients
    /// that omit the suffix.
    pub(crate) fn deserialize_str(s: &str) -> Result<Duration, String> {
        let s = s.strip_suffix('s').unwrap_or(s);
        let negative = s.starts_with('-');
        let s = s.strip_prefix('-').unwrap_or(s);

        let (secs, nanos) = if let Some((whole, frac)) = s.split_once('.') {
            let secs: i64 = whole
                .parse()
                .map_err(|e: std::num::ParseIntError| e.to_string())?;
            if frac.len() > 9 {
                return Err(
                    "duration fractional part exceeds 9 digits (nanosecond precision)".to_string(),
                );
            }
            let padded = format!("{frac:0<9}");
            let nanos: i32 = padded
                .parse()
                .map_err(|e: std::num::ParseIntError| e.to_string())?;
            (secs, nanos)
        } else {
            let secs: i64 = s
                .parse()
                .map_err(|e: std::num::ParseIntError| e.to_string())?;
            (secs, 0)
        };

        if negative {
            Ok(Duration {
                seconds: -secs,
                nanos: -nanos,
            })
        } else {
            Ok(Duration {
                seconds: secs,
                nanos,
            })
        }
    }

    /// Deserialize a `Duration` from a seconds string with `s` suffix.
    ///
    /// # Errors
    ///
    /// Returns `D::Error` if the string is not a valid duration.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        deserialize_str(&s).map_err(serde::de::Error::custom)
    }
}

/// Serde adapter for `Option<prost_types::FieldMask>` ↔ comma-separated paths string.
///
/// Proto JSON mapping uses camelCase paths: `"displayName,email"`.
/// Paths are normalized to `snake_case` on deserialization to match prost field names.
///
/// # Errors
///
/// Deserialization fails if the input is not a valid string.
/// Serialization is infallible.
pub mod opt_field_mask {
    use prost_types::FieldMask;
    use serde::{self, Deserialize, Deserializer, Serializer};

    /// Serialize an optional `FieldMask` as a comma-separated camelCase paths string.
    ///
    /// # Errors
    ///
    /// Returns `S::Error` if serialization fails.
    pub fn serialize<S>(value: &Option<FieldMask>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(fm) => super::field_mask::serialize(fm, serializer),
            None => serializer.serialize_none(),
        }
    }

    /// Deserialize an optional `FieldMask` from a comma-separated camelCase paths string.
    ///
    /// # Errors
    ///
    /// Returns `D::Error` if the input is not a valid string.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<FieldMask>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt {
            Some(s) if !s.is_empty() => Ok(Some(super::field_mask::from_str(&s))),
            _ => Ok(None),
        }
    }
}

/// Serde adapter for `prost_types::FieldMask` ↔ comma-separated paths string.
///
/// Proto JSON mapping uses camelCase paths: `"displayName,email"`.
/// Paths are normalized to `snake_case` on deserialization to match prost field names.
///
/// Use [`opt_field_mask`] for `Option<FieldMask>` fields.
///
/// # Examples
///
/// ```
/// use serde::{Serialize, Deserialize};
/// use prost_types::FieldMask;
///
/// #[derive(Serialize, Deserialize)]
/// struct UpdateRequest {
///     #[serde(with = "tonic_rest::serde::field_mask")]
///     update_mask: FieldMask,
/// }
///
/// let req = UpdateRequest {
///     update_mask: FieldMask { paths: vec!["display_name".to_string(), "email".to_string()] },
/// };
/// let json = serde_json::to_string(&req).unwrap();
/// assert!(json.contains("displayName,email"));
/// ```
/// # Errors
///
/// Deserialization fails if the input is not a valid string.
/// Serialization is infallible.
pub mod field_mask {
    use prost_types::FieldMask;
    use serde::{self, Deserialize, Deserializer, Serializer};

    /// Serialize a `FieldMask` as a comma-separated camelCase paths string.
    ///
    /// # Errors
    ///
    /// Returns `S::Error` if serialization fails.
    pub fn serialize<S>(value: &FieldMask, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let camel: Vec<String> = value.paths.iter().map(|p| snake_to_camel(p)).collect();
        serializer.serialize_str(&camel.join(","))
    }

    /// Parse a `FieldMask` from a comma-separated camelCase paths string.
    pub(crate) fn from_str(s: &str) -> FieldMask {
        FieldMask {
            paths: s.split(',').map(|p| camel_to_snake(p.trim())).collect(),
        }
    }

    /// Deserialize a `FieldMask` from a comma-separated camelCase paths string.
    ///
    /// # Errors
    ///
    /// Returns `D::Error` if the input is not a valid string.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<FieldMask, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(from_str(&s))
    }

    /// Convert `snake_case` → `camelCase` for proto JSON mapping.
    fn snake_to_camel(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        let mut upper_next = false;
        for ch in s.chars() {
            if ch == '_' {
                upper_next = true;
            } else if upper_next {
                result.extend(ch.to_uppercase());
                upper_next = false;
            } else {
                result.push(ch);
            }
        }
        result
    }

    /// Convert `camelCase` → `snake_case`, handling consecutive uppercase (acronyms).
    fn camel_to_snake(s: &str) -> String {
        let mut result = String::with_capacity(s.len() + 4);
        let chars: Vec<char> = s.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            if ch.is_uppercase() {
                if i > 0
                    && (chars[i - 1].is_lowercase()
                        || (i + 1 < chars.len() && chars[i + 1].is_lowercase()))
                {
                    result.push('_');
                }
                result.extend(ch.to_lowercase());
            } else {
                result.push(ch);
            }
        }
        result
    }
}

/// Generate `#[serde(with)]` modules for proto3 enum fields (`i32` in prost).
///
/// Serializes as the proto enum name string (e.g., `"USER_ROLE_ADMIN"`) following
/// Google's protobuf JSON mapping.
///
/// With an optional prefix, strips the prefix and lowercases for REST-friendly output:
/// `define_enum_serde!(health_status, HealthStatus, "HEALTH_STATUS_")` →
/// `"healthy"` / `"unhealthy"` instead of `"HEALTH_STATUS_HEALTHY"`.
///
/// For each invocation, three sub-modules are created inside `{name}`:
/// - `{name}`            — for `i32` fields (`#[serde(with = "serde_wkt::user_role")]`)
/// - `{name}::optional`  — for `Option<i32>` fields
/// - `{name}::repeated`  — for `Vec<i32>` fields
///
/// # Examples
///
/// Full enum name on the wire:
///
/// ```ignore
/// // Given a prost-generated enum:
/// // #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
/// // #[repr(i32)]
/// // pub enum UserRole { Unspecified = 0, Admin = 1, User = 2 }
///
/// mod serde_wkt {
///     tonic_rest::define_enum_serde!(user_role, crate::UserRole);
/// }
///
/// #[derive(serde::Serialize, serde::Deserialize)]
/// struct MyMessage {
///     #[serde(with = "serde_wkt::user_role")]
///     role: i32,                          // serializes as "ADMIN"
///     #[serde(with = "serde_wkt::user_role::optional")]
///     backup_role: Option<i32>,           // serializes as "USER" or null
///     #[serde(with = "serde_wkt::user_role::repeated")]
///     allowed_roles: Vec<i32>,            // serializes as ["ADMIN", "USER"]
/// }
/// ```
///
/// With prefix stripping for REST-friendly output:
///
/// ```ignore
/// mod serde_wkt {
///     // Strips "HEALTH_STATUS_" prefix and lowercases: "healthy", "unhealthy"
///     tonic_rest::define_enum_serde!(health_status, crate::HealthStatus, "HEALTH_STATUS_");
/// }
///
/// #[derive(serde::Serialize, serde::Deserialize)]
/// struct HealthCheck {
///     #[serde(with = "serde_wkt::health_status")]
///     status: i32,  // serializes as "healthy" instead of "HEALTH_STATUS_HEALTHY"
/// }
/// ```
///
/// Deserialization accepts both the wire format and the original proto name,
/// as well as raw integers for forward compatibility.
#[macro_export]
macro_rules! define_enum_serde {
    ($name:ident, $enum_type:ty) => {
        $crate::define_enum_serde!(@impl $name, $enum_type, |s: &str| s.to_string(), |s: &str| s.to_string());
    };
    ($name:ident, $enum_type:ty, $prefix:literal) => {
        $crate::define_enum_serde!(@impl $name, $enum_type,
            |s: &str| s.strip_prefix($prefix).unwrap_or(s).to_lowercase(),
            |s: &str| {
                let upper = s.to_uppercase();
                format!("{}{}", $prefix, upper)
            });
    };
    (@impl $name:ident, $enum_type:ty, $to_wire:expr, $from_wire:expr) => {
        #[allow(clippy::missing_errors_doc)]
        pub mod $name {
            use serde::{Deserializer, Serializer};

            /// Serialize `i32` → wire string.
            pub fn serialize<S: Serializer>(value: &i32, serializer: S) -> Result<S::Ok, S::Error> {
                let to_wire: fn(&str) -> String = $to_wire;
                match <$enum_type>::try_from(*value) {
                    Ok(e) => serializer.serialize_str(&to_wire(e.as_str_name())),
                    Err(_) => serializer.serialize_i32(*value),
                }
            }

            /// Deserialize from wire string or integer.
            pub fn deserialize<'de, D: Deserializer<'de>>(
                deserializer: D,
            ) -> Result<i32, D::Error> {
                use serde::de;

                struct EnumVisitor;

                impl de::Visitor<'_> for EnumVisitor {
                    type Value = i32;

                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        write!(f, "a proto enum name string or integer")
                    }

                    fn visit_str<E: de::Error>(self, v: &str) -> Result<i32, E> {
                        // Try exact proto name first
                        if let Some(e) = <$enum_type>::from_str_name(v) {
                            return Ok(e as i32);
                        }
                        // Try converting from wire format
                        let from_wire: fn(&str) -> String = $from_wire;
                        let canonical = from_wire(v);
                        <$enum_type>::from_str_name(&canonical)
                            .map(|e| e as i32)
                            .ok_or_else(|| {
                                E::custom(
                                    concat!("unknown ", stringify!($enum_type), " value: ")
                                        .to_string()
                                        + v,
                                )
                            })
                    }

                    fn visit_i64<E: de::Error>(self, v: i64) -> Result<i32, E> {
                        i32::try_from(v).map_err(E::custom)
                    }

                    fn visit_u64<E: de::Error>(self, v: u64) -> Result<i32, E> {
                        i32::try_from(v).map_err(E::custom)
                    }
                }

                deserializer.deserialize_any(EnumVisitor)
            }

            /// Serde adapter for `Option<i32>` proto enum fields.
            #[allow(clippy::missing_errors_doc)]
            pub mod optional {
                use serde::{Deserializer, Serializer};

                /// Serialize an optional proto enum `i32` as a wire string.
                #[allow(clippy::ref_option)] // serde `with` protocol requires `&Option<T>`
                pub fn serialize<S: Serializer>(
                    value: &Option<i32>,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    match value {
                        Some(v) => super::serialize(v, serializer),
                        None => serializer.serialize_none(),
                    }
                }

                /// Deserialize an optional proto enum from a wire string or integer.
                pub fn deserialize<'de, D: Deserializer<'de>>(
                    deserializer: D,
                ) -> Result<Option<i32>, D::Error> {
                    use serde::de;

                    struct OptionalEnumVisitor;

                    impl<'de> de::Visitor<'de> for OptionalEnumVisitor {
                        type Value = Option<i32>;

                        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                            write!(f, "a proto enum name string, integer, or null")
                        }

                        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                            Ok(None)
                        }

                        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                            Ok(None)
                        }

                        fn visit_some<D2: de::Deserializer<'de>>(
                            self,
                            deserializer: D2,
                        ) -> Result<Self::Value, D2::Error> {
                            super::deserialize(deserializer).map(Some)
                        }
                    }

                    deserializer.deserialize_option(OptionalEnumVisitor)
                }
            }

            /// Serde adapter for `Vec<i32>` repeated proto enum fields.
            #[allow(clippy::missing_errors_doc)]
            pub mod repeated {
                use serde::{Deserializer, Serializer};

                /// Serialize repeated proto enum `i32` values as wire strings.
                ///
                /// Known values serialize as their proto name string. Unknown values
                /// fall back to the raw `i32`, consistent with singular enum serialization.
                pub fn serialize<S: Serializer>(
                    values: &[i32],
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    use serde::ser::SerializeSeq;

                    let to_wire: fn(&str) -> String = $to_wire;
                    let mut seq = serializer.serialize_seq(Some(values.len()))?;
                    for v in values {
                        match <$enum_type>::try_from(*v) {
                            Ok(e) => seq.serialize_element(&to_wire(e.as_str_name()))?,
                            Err(_) => seq.serialize_element(v)?,
                        }
                    }
                    seq.end()
                }

                /// Deserialize repeated proto enum values from wire strings or integers.
                pub fn deserialize<'de, D: Deserializer<'de>>(
                    deserializer: D,
                ) -> Result<Vec<i32>, D::Error> {
                    use serde::de;

                    struct EnumSeqVisitor;

                    impl<'de> de::Visitor<'de> for EnumSeqVisitor {
                        type Value = Vec<i32>;

                        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                            write!(f, "a sequence of proto enum name strings or integers")
                        }

                        fn visit_seq<A: de::SeqAccess<'de>>(
                            self,
                            mut seq: A,
                        ) -> Result<Self::Value, A::Error> {
                            let mut values =
                                Vec::with_capacity(seq.size_hint().unwrap_or(0));
                            while let Some(val) = seq.next_element_seed(EnumSeed)? {
                                values.push(val);
                            }
                            Ok(values)
                        }
                    }

                    struct EnumSeed;

                    impl<'de> de::DeserializeSeed<'de> for EnumSeed {
                        type Value = i32;

                        fn deserialize<D2: de::Deserializer<'de>>(
                            self,
                            deserializer: D2,
                        ) -> Result<Self::Value, D2::Error> {
                            super::deserialize(deserializer)
                        }
                    }

                    deserializer.deserialize_seq(EnumSeqVisitor)
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use prost_types::{Duration, FieldMask, Timestamp};
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(i32)]
    enum TestStatus {
        Unspecified = 0,
        Active = 1,
        Inactive = 2,
    }

    impl TestStatus {
        #[allow(clippy::trivially_copy_pass_by_ref)] // matches prost-generated API
        fn as_str_name(&self) -> &'static str {
            match self {
                Self::Unspecified => "TEST_STATUS_UNSPECIFIED",
                Self::Active => "ACTIVE",
                Self::Inactive => "INACTIVE",
            }
        }

        fn from_str_name(s: &str) -> Option<Self> {
            match s {
                "TEST_STATUS_UNSPECIFIED" => Some(Self::Unspecified),
                "ACTIVE" => Some(Self::Active),
                "INACTIVE" => Some(Self::Inactive),
                _ => None,
            }
        }
    }

    impl TryFrom<i32> for TestStatus {
        type Error = &'static str;
        fn try_from(value: i32) -> Result<Self, Self::Error> {
            match value {
                0 => Ok(Self::Unspecified),
                1 => Ok(Self::Active),
                2 => Ok(Self::Inactive),
                _ => Err("unknown"),
            }
        }
    }

    define_enum_serde!(test_status, crate::serde::tests::TestStatus);

    // --- Prefix-stripping variant ---

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(i32)]
    enum HealthStatus {
        Unspecified = 0,
        Healthy = 1,
        Unhealthy = 2,
    }

    impl HealthStatus {
        #[allow(clippy::trivially_copy_pass_by_ref)]
        fn as_str_name(&self) -> &'static str {
            match self {
                Self::Unspecified => "HEALTH_STATUS_UNSPECIFIED",
                Self::Healthy => "HEALTH_STATUS_HEALTHY",
                Self::Unhealthy => "HEALTH_STATUS_UNHEALTHY",
            }
        }

        fn from_str_name(s: &str) -> Option<Self> {
            match s {
                "HEALTH_STATUS_UNSPECIFIED" => Some(Self::Unspecified),
                "HEALTH_STATUS_HEALTHY" => Some(Self::Healthy),
                "HEALTH_STATUS_UNHEALTHY" => Some(Self::Unhealthy),
                _ => None,
            }
        }
    }

    impl TryFrom<i32> for HealthStatus {
        type Error = &'static str;
        fn try_from(value: i32) -> Result<Self, Self::Error> {
            match value {
                0 => Ok(Self::Unspecified),
                1 => Ok(Self::Healthy),
                2 => Ok(Self::Unhealthy),
                _ => Err("unknown"),
            }
        }
    }

    define_enum_serde!(
        health_status,
        crate::serde::tests::HealthStatus,
        "HEALTH_STATUS_"
    );

    #[derive(Serialize, Deserialize, Debug)]
    struct TsRequired {
        #[serde(with = "super::timestamp")]
        ts: Timestamp,
    }

    #[test]
    fn timestamp_required_round_trip() {
        let w = TsRequired {
            ts: Timestamp {
                seconds: 1_736_934_600,
                nanos: 0,
            },
        };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("2025-01-15"));
        let back: TsRequired = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ts.seconds, 1_736_934_600);
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct TsWrapper {
        #[serde(with = "super::opt_timestamp", default)]
        ts: Option<Timestamp>,
    }

    #[test]
    fn timestamp_round_trip() {
        let ts = Timestamp {
            seconds: 1_736_934_600,
            nanos: 0,
        };
        let w = TsWrapper { ts: Some(ts) };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("2025-01-15"));
        let back: TsWrapper = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ts.unwrap().seconds, 1_736_934_600);
    }

    #[test]
    fn timestamp_none() {
        let w = TsWrapper { ts: None };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("null"));
        let back: TsWrapper = serde_json::from_str(&json).unwrap();
        assert!(back.ts.is_none());
    }

    #[test]
    fn timestamp_with_subsecond_nanos_round_trip() {
        let ts = Timestamp {
            seconds: 1_736_934_600,
            nanos: 123_456_789,
        };
        let w = TsWrapper { ts: Some(ts) };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("2025-01-15"), "date present: {json}");
        assert!(json.contains(".123456789"), "nanos present: {json}");
        let back: TsWrapper = serde_json::from_str(&json).unwrap();
        let back_ts = back.ts.unwrap();
        assert_eq!(back_ts.seconds, 1_736_934_600);
        assert_eq!(back_ts.nanos, 123_456_789);
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct DurRequired {
        #[serde(with = "super::duration")]
        dur: Duration,
    }

    #[test]
    fn duration_required_round_trip() {
        let w = DurRequired {
            dur: Duration {
                seconds: 300,
                nanos: 0,
            },
        };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("300s"));
        let back: DurRequired = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dur.seconds, 300);
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct DurWrapper {
        #[serde(with = "super::opt_duration", default)]
        dur: Option<Duration>,
    }

    #[test]
    fn duration_round_trip() {
        let d = Duration {
            seconds: 300,
            nanos: 0,
        };
        let w = DurWrapper { dur: Some(d) };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("300s"));
        let back: DurWrapper = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dur.unwrap().seconds, 300);
    }

    #[test]
    fn duration_with_nanos() {
        let d = Duration {
            seconds: 1,
            nanos: 500_000_000,
        };
        let w = DurWrapper { dur: Some(d) };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("1.5s"));
    }

    #[test]
    fn negative_duration_round_trip() {
        let d = Duration {
            seconds: -300,
            nanos: 0,
        };
        let w = DurWrapper { dur: Some(d) };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("-300s"), "negative: {json}");
        let back: DurWrapper = serde_json::from_str(&json).unwrap();
        let back_dur = back.dur.unwrap();
        assert_eq!(back_dur.seconds, -300);
        assert_eq!(back_dur.nanos, 0);
    }

    #[test]
    fn negative_duration_with_nanos_round_trip() {
        let d = Duration {
            seconds: -1,
            nanos: -500_000_000,
        };
        let w = DurWrapper { dur: Some(d) };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("-1.5s"), "negative with nanos: {json}");
        let back: DurWrapper = serde_json::from_str(&json).unwrap();
        let back_dur = back.dur.unwrap();
        assert_eq!(back_dur.seconds, -1);
        assert_eq!(back_dur.nanos, -500_000_000);
    }

    #[test]
    fn duration_none() {
        let w = DurWrapper { dur: None };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("null"));
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct FmRequired {
        #[serde(with = "super::field_mask")]
        mask: FieldMask,
    }

    #[test]
    fn field_mask_required_round_trip() {
        let w = FmRequired {
            mask: FieldMask {
                paths: vec!["display_name".to_string(), "email".to_string()],
            },
        };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("displayName,email"));
        let back: FmRequired = serde_json::from_str(&json).unwrap();
        assert_eq!(back.mask.paths, vec!["display_name", "email"]);
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct FmWrapper {
        #[serde(with = "super::opt_field_mask", default)]
        mask: Option<FieldMask>,
    }

    #[test]
    fn field_mask_round_trip() {
        let fm = FieldMask {
            paths: vec!["display_name".to_string(), "email".to_string()],
        };
        let w = FmWrapper { mask: Some(fm) };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("displayName,email"));
        let back: FmWrapper = serde_json::from_str(&json).unwrap();
        let paths = back.mask.unwrap().paths;
        assert_eq!(paths, vec!["display_name", "email"]);
    }

    #[test]
    fn field_mask_none() {
        let w = FmWrapper { mask: None };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("null"));
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct EnumWrapper {
        #[serde(with = "test_status")]
        status: i32,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct OptEnumWrapper {
        #[serde(with = "test_status::optional", default)]
        status: Option<i32>,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct RepEnumWrapper {
        #[serde(with = "test_status::repeated")]
        statuses: Vec<i32>,
    }

    #[test]
    fn enum_serialize_by_name() {
        let w = EnumWrapper { status: 1 };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("ACTIVE"));
    }

    #[test]
    fn enum_deserialize_from_name() {
        let w: EnumWrapper = serde_json::from_str(r#"{"status":"ACTIVE"}"#).unwrap();
        assert_eq!(w.status, 1);
    }

    #[test]
    fn enum_deserialize_from_int() {
        let w: EnumWrapper = serde_json::from_str(r#"{"status":2}"#).unwrap();
        assert_eq!(w.status, 2);
    }

    #[test]
    fn optional_enum_none() {
        let w: OptEnumWrapper = serde_json::from_str(r#"{"status":null}"#).unwrap();
        assert!(w.status.is_none());
    }

    #[test]
    fn optional_enum_some() {
        let w: OptEnumWrapper = serde_json::from_str(r#"{"status":"ACTIVE"}"#).unwrap();
        assert_eq!(w.status, Some(1));
    }

    #[test]
    fn repeated_enum_round_trip() {
        let w = RepEnumWrapper {
            statuses: vec![1, 2],
        };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("ACTIVE"));
        assert!(json.contains("INACTIVE"));
        let back: RepEnumWrapper = serde_json::from_str(&json).unwrap();
        assert_eq!(back.statuses, vec![1, 2]);
    }

    /// Unknown i32 values in repeated enums serialize as raw integers,
    /// consistent with singular enum serialization, rather than collapsing
    /// to an `"UNKNOWN"` string.
    #[test]
    fn repeated_enum_unknown_values_serialize_as_integers() {
        let w = RepEnumWrapper {
            statuses: vec![1, 999, 2],
        };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("\"ACTIVE\""), "known value: {json}");
        assert!(json.contains("\"INACTIVE\""), "known value: {json}");
        assert!(json.contains("999"), "unknown value as int: {json}");
        assert!(
            !json.contains("\"999\""),
            "unknown value should NOT be a quoted string: {json}",
        );

        let back: RepEnumWrapper = serde_json::from_str(&json).unwrap();
        assert_eq!(back.statuses, vec![1, 999, 2]);
    }

    // --- Prefix-stripping enum tests ---

    #[derive(Serialize, Deserialize, Debug)]
    struct HealthWrapper {
        #[serde(with = "health_status")]
        status: i32,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct OptHealthWrapper {
        #[serde(with = "health_status::optional", default)]
        status: Option<i32>,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct RepHealthWrapper {
        #[serde(with = "health_status::repeated")]
        statuses: Vec<i32>,
    }

    #[test]
    fn prefix_stripped_enum_serialize() {
        let w = HealthWrapper { status: 1 };
        let json = serde_json::to_string(&w).unwrap();
        // Should be lowercase "healthy", not full "HEALTH_STATUS_HEALTHY"
        assert!(
            json.contains("\"healthy\""),
            "should be prefix-stripped lowercase: {json}",
        );
        assert!(
            !json.contains("HEALTH_STATUS"),
            "should NOT contain original prefix: {json}",
        );
    }

    #[test]
    fn prefix_stripped_enum_deserialize_from_lowercase() {
        let w: HealthWrapper = serde_json::from_str(r#"{"status":"healthy"}"#).unwrap();
        assert_eq!(w.status, 1);
    }

    #[test]
    fn prefix_stripped_enum_deserialize_from_original() {
        // Should also accept the full proto name
        let w: HealthWrapper =
            serde_json::from_str(r#"{"status":"HEALTH_STATUS_HEALTHY"}"#).unwrap();
        assert_eq!(w.status, 1);
    }

    #[test]
    fn prefix_stripped_enum_deserialize_from_int() {
        let w: HealthWrapper = serde_json::from_str(r#"{"status":2}"#).unwrap();
        assert_eq!(w.status, 2);
    }

    #[test]
    fn prefix_stripped_enum_round_trip() {
        let w = HealthWrapper { status: 1 };
        let json = serde_json::to_string(&w).unwrap();
        let back: HealthWrapper = serde_json::from_str(&json).unwrap();
        assert_eq!(back.status, 1);
    }

    #[test]
    fn prefix_stripped_optional_none() {
        let w: OptHealthWrapper = serde_json::from_str(r#"{"status":null}"#).unwrap();
        assert!(w.status.is_none());
    }

    #[test]
    fn prefix_stripped_optional_some() {
        let w: OptHealthWrapper = serde_json::from_str(r#"{"status":"unhealthy"}"#).unwrap();
        assert_eq!(w.status, Some(2));
    }

    #[test]
    fn prefix_stripped_repeated_round_trip() {
        let w = RepHealthWrapper {
            statuses: vec![1, 2],
        };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("\"healthy\""), "first value: {json}");
        assert!(json.contains("\"unhealthy\""), "second value: {json}");
        let back: RepHealthWrapper = serde_json::from_str(&json).unwrap();
        assert_eq!(back.statuses, vec![1, 2]);
    }
}
