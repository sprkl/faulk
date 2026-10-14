//! Declaration macros: a command is one block in its domain file, from which the
//! serde names, the schema metadata, `name()`, `meta()` and the output variant are
//! derived. Nothing else is written by hand per command (spec § 5.2: no `match` to
//! extend in three places).

/// Parses a `kind` literal at compile time; a typo is a build error.
pub const fn parse_kind(s: &str) -> crate::v2::contract::Kind {
    use crate::v2::contract::Kind;
    match s.as_bytes() {
        b"edit" => Kind::Edit,
        b"live" => Kind::Live,
        b"query" => Kind::Query,
        _ => panic!("kind is `edit`, `live` or `query`"),
    }
}

/// Parses an `undo` literal at compile time; a typo is a build error.
pub const fn parse_undo(s: &str) -> crate::v2::contract::UndoKind {
    use crate::v2::contract::UndoKind;
    match s.as_bytes() {
        b"inverse" => UndoKind::Inverse,
        b"none" => UndoKind::None,
        _ => panic!("undo is `inverse` or `none`"),
    }
}

/// Declares a domain's `Command` and `Output` enums.
///
/// ```ignore
/// domain! {
///     domain = "track", schema = ("TrackCommand", "TrackOutput");
///     /// Create a track ... (the description agents read)
///     "track.create" (kind = "edit", undo = "inverse", destructive = false)
///     Create { kind: TrackKind } => { track: TrackId };
/// }
/// ```
///
/// Every command has an `Output` variant of the same name (`=> {}` for an empty one),
/// is tagged by `type` on both sides, refuses unknown fields, and carries `x-kind`,
/// `x-undo` and `x-destructive` in its schema. A command without fields is still a
/// struct variant (`Play {}`): serde ignores unknown fields on an internally tagged
/// unit variant, a struct variant refuses them.
macro_rules! domain {
    (
        domain = $domain:literal, schema = ($cmd_schema:literal, $out_schema:literal);
        $(
            $(#[$attr:meta])*
            $name:literal (kind = $kind:literal, undo = $undo:literal, destructive = $destructive:literal)
            $Variant:ident $({ $( $(#[$fattr:meta])* $field:ident : $ty:ty ),* $(,)? })?
            => { $( $(#[$oattr:meta])* $ofield:ident : $oty:ty ),* $(,)? };
        )*
    ) => {
        /// The commands of this domain, tagged by `type`.
        #[derive(Clone, Debug, PartialEq, ::serde::Serialize, ::serde::Deserialize, ::schemars::JsonSchema)]
        #[serde(tag = "type", deny_unknown_fields)]
        #[schemars(rename = $cmd_schema)]
        pub enum Command {
            $(
                $(#[$attr])*
                #[serde(rename = $name)]
                #[schemars(extend("x-kind" = $kind, "x-undo" = $undo, "x-destructive" = $destructive))]
                $Variant { $( $( $(#[$fattr])* $field: $ty ),* )? },
            )*
        }

        /// The outputs of this domain's commands, tagged by the command's `type`.
        #[derive(Clone, Debug, PartialEq, ::serde::Serialize, ::serde::Deserialize, ::schemars::JsonSchema)]
        #[serde(tag = "type", deny_unknown_fields)]
        #[schemars(rename = $out_schema)]
        pub enum Output {
            $(
                #[serde(rename = $name)]
                $Variant { $( $(#[$oattr])* $ofield: $oty ),* },
            )*
        }

        impl Command {
            /// The domain, the part of every name before the dot.
            pub const DOMAIN: &'static str = $domain;

            /// The metadata of every command of the domain, in declaration order.
            pub const METAS: &'static [$crate::v2::contract::Meta] = &[
                $(
                    $crate::v2::contract::Meta {
                        name: $name,
                        kind: $crate::v2::macros::parse_kind($kind),
                        undo: $crate::v2::macros::parse_undo($undo),
                        destructive: $destructive,
                    },
                )*
            ];

            /// The command's name, as in its `type` field.
            pub fn name(&self) -> &'static str {
                self.meta().name
            }

            pub fn meta(&self) -> $crate::v2::contract::Meta {
                match *self {
                    $(
                        Self::$Variant { .. } => $crate::v2::contract::Meta {
                            name: $name,
                            kind: $crate::v2::macros::parse_kind($kind),
                            undo: $crate::v2::macros::parse_undo($undo),
                            destructive: $destructive,
                        },
                    )*
                }
            }
        }

        impl Output {
            /// The name of the command this output answers.
            pub fn name(&self) -> &'static str {
                match *self {
                    $( Self::$Variant { .. } => $name, )*
                }
            }
        }
    };
}

/// Declares the root `Command` and `Output`: one line per domain.
macro_rules! root {
    ( $( $Variant:ident => $module:ident ),* $(,)? ) => {
        /// A Faulk command, `{"type": "domain.verb", ...}`. One variant per domain; the
        /// domain's enum holds its commands.
        #[derive(Clone, Debug, PartialEq, ::serde::Serialize, ::schemars::JsonSchema)]
        #[serde(untagged)]
        pub enum Command {
            $( $Variant($module::Command), )*
        }

        /// The output of a command, `{"type": "domain.verb", ...}` with the same name.
        #[derive(Clone, Debug, PartialEq, ::serde::Serialize, ::serde::Deserialize, ::schemars::JsonSchema)]
        #[serde(untagged)]
        pub enum Output {
            $( $Variant($module::Output), )*
        }

        impl Command {
            /// The MVP domains, in the spec's order.
            pub const DOMAINS: &'static [&'static str] = &[ $( $module::Command::DOMAIN, )* ];

            /// The metadata of every command of every domain.
            pub fn metas() -> Vec<$crate::v2::contract::Meta> {
                let mut all = Vec::new();
                $( all.extend_from_slice($module::Command::METAS); )*
                all
            }

            pub fn name(&self) -> &'static str {
                match self {
                    $( Self::$Variant(c) => c.name(), )*
                }
            }

            pub fn meta(&self) -> $crate::v2::contract::Meta {
                match self {
                    $( Self::$Variant(c) => c.meta(), )*
                }
            }

            /// Applies the command to the state. The frozen signature of the contract.
            pub fn apply(
                &self,
                state: &mut $crate::v2::contract::State,
                ctx: &mut $crate::v2::contract::Context,
            ) -> Result<$crate::v2::Outcome, $crate::v2::contract::CommandError> {
                match self {
                    $(
                        Self::$Variant(c) => {
                            let out = $module::apply(c, state, ctx)?;
                            Ok($crate::v2::contract::Outcome {
                                output: Output::$Variant(out.output),
                                undo: match out.undo {
                                    $crate::v2::contract::Undo::Inverse(inv) => {
                                        $crate::v2::contract::Undo::Inverse(Box::new(Self::$Variant(*inv)))
                                    }
                                    $crate::v2::contract::Undo::None => $crate::v2::contract::Undo::None,
                                },
                                events: out.events,
                                control: out.control,
                            })
                        }
                    )*
                }
            }
        }

        impl Output {
            pub fn name(&self) -> &'static str {
                match self {
                    $( Self::$Variant(o) => o.name(), )*
                }
            }
        }

        impl<'de> ::serde::Deserialize<'de> for Command {
            /// Dispatches on the domain prefix of `type`, so an unknown domain and an
            /// unknown verb are named in the error (an untagged enum would only say
            /// that nothing matched).
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                use ::serde::de::Error as _;
                let value = ::serde_json::Value::deserialize(deserializer)?;
                let Some(ty) = value.get("type").and_then(|t| t.as_str()) else {
                    return Err(D::Error::custom("a command has a string `type` field, `domain.verb`"));
                };
                let Some((domain, _)) = ty.split_once('.') else {
                    return Err(D::Error::custom(format!("`{ty}` is not a `domain.verb` command name")));
                };
                match domain {
                    $(
                        $module::Command::DOMAIN => ::serde_json::from_value::<$module::Command>(value)
                            .map(Self::$Variant)
                            .map_err(D::Error::custom),
                    )*
                    other => Err(D::Error::custom(format!(
                        "unknown command domain `{other}` in `{ty}`; the domains are {}",
                        Self::DOMAINS.join(", ")
                    ))),
                }
            }
        }
    };
}

pub(crate) use domain;
pub(crate) use root;
