//! Internal macros shared by the domain modules.

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(::uuid::Uuid);

        impl $name {
            /// Wraps an existing UUID (e.g. read back from storage).
            #[must_use]
            pub const fn new(id: ::uuid::Uuid) -> Self {
                Self(id)
            }

            /// Generates a new time-ordered (UUIDv7) identifier.
            #[must_use]
            pub fn generate() -> Self {
                Self(::uuid::Uuid::now_v7())
            }

            /// The underlying UUID.
            #[must_use]
            pub const fn as_uuid(self) -> ::uuid::Uuid {
                self.0
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::Display::fmt(&self.0, f)
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::identity::IdentityError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                ::uuid::Uuid::parse_str(s)
                    .map(Self)
                    .map_err(|_| $crate::identity::IdentityError::MalformedId)
            }
        }
    };
}
