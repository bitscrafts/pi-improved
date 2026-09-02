//! Id newtypes over `String`.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident) => {
        #[doc = concat!("Unique identifier newtype (", stringify!($name), ").")]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Creates a new id from any string-like value.
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Borrows the underlying string.
            #[must_use]
            pub const fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(String::from(value))
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

string_id!(
    /// Identifies a pipeline run.
    RunId
);
string_id!(
    /// Identifies a node inside a graph.
    NodeId
);
string_id!(
    /// Names an output port of a node.
    PortName
);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{NodeId, PortName, RunId};

    #[test]
    fn ids_display_and_from_str() {
        assert_eq!(RunId::from("r1").to_string(), "r1");
        assert_eq!(NodeId::from("n1").to_string(), "n1");
        assert_eq!(PortName::from("ok").to_string(), "ok");
    }
}