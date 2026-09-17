//! One declaration of each enum drives serde, wire strings, and parsing.

/// Declare an enum together with the one wire string for each variant.
///
/// The string is written once and drives serde, `as_str`, and the `parse_*`
/// function, so a new variant cannot encode one way in the JSON API and another
/// in its database column. Those three used to be separate hand-written
/// surfaces with nothing forcing them to agree.
macro_rules! wire_enum {
    (
        $(#[doc = $doc:literal])*
        $name:ident { $($variant:ident => $wire:literal),+ $(,)? }
        parse: $parse:ident
    ) => {
        $(#[doc = $doc])*
        #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
        pub enum $name {
            $(
                #[serde(rename = $wire)]
                $variant,
            )+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire,)+
                }
            }
        }

        pub fn $parse(value: &str) -> Option<$name> {
            match value {
                $($wire => Some($name::$variant),)+
                _ => None,
            }
        }
    };
    // Forward-tolerant form: an unrecognised wire string parses as the named
    // fallback variant instead of failing, so an N-1 reader survives the next
    // added variant. Only for enums with a variant whose documented meaning
    // is already "not known" — never for enums where a wrong guess would be
    // acted on.
    (
        $(#[doc = $doc:literal])*
        $name:ident { $($variant:ident => $wire:literal),+ $(,)? }
        parse: $parse:ident
        fallback: $fallback:ident
    ) => {
        $(#[doc = $doc])*
        #[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
        pub enum $name {
            $(
                #[serde(rename = $wire)]
                $variant,
            )+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire,)+
                }
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = <String as serde::Deserialize>::deserialize(deserializer)?;
                Ok($parse(&value).unwrap_or(Self::$fallback))
            }
        }

        /// Never `None`: an unrecognised string is the fallback variant. The
        /// `Option` return keeps the shape of every other `parse_*`.
        pub fn $parse(value: &str) -> Option<$name> {
            match value {
                $($wire => Some($name::$variant),)+
                _ => Some($name::$fallback),
            }
        }
    };
}

pub(crate) use wire_enum;
