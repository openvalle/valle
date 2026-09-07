use serde::{Deserialize, Serialize};

macro_rules! typed_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u32);

        impl $name {
            pub const fn from_raw(raw: u32) -> Self {
                Self(raw)
            }

            pub const fn raw(self) -> u32 {
                self.0
            }

            pub(crate) const fn index(self) -> usize {
                self.0 as usize
            }

            pub(crate) fn from_index(index: usize) -> Self {
                debug_assert!(u32::try_from(index).is_ok());
                Self(index as u32)
            }
        }
    };
}

typed_id!(NodeId);
typed_id!(PathId);
typed_id!(PaintId);
