use serde::{Deserialize, Serialize};
use thiserror::Error;

macro_rules! graph_id {
    ($name:ident, $label:literal) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(try_from = "u32", into = "u32")]
        pub struct $name(u32);

        impl $name {
            pub(crate) fn from_index(index: usize) -> Result<Self, GraphIdError> {
                let value = index
                    .checked_add(1)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or(GraphIdError::BudgetExceeded { kind: $label })?;
                Ok(Self(value))
            }

            pub const fn get(self) -> u32 {
                self.0
            }

            pub(crate) fn index(self) -> usize {
                (self.0 - 1) as usize
            }
        }

        impl TryFrom<u32> for $name {
            type Error = GraphIdError;

            fn try_from(value: u32) -> Result<Self, Self::Error> {
                if value == 0 {
                    Err(GraphIdError::Zero { kind: $label })
                } else {
                    Ok(Self(value))
                }
            }
        }

        impl From<$name> for u32 {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

graph_id!(ResourceId, "resource");
graph_id!(PassId, "pass");
graph_id!(CompositeVersionId, "composite version");

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum GraphIdError {
    #[error("graph {kind} id zero is reserved")]
    Zero { kind: &'static str },
    #[error("graph {kind} id budget exceeded")]
    BudgetExceeded { kind: &'static str },
}
