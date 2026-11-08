//! The lake profile of the runner (feature 035, T035-035, T035-044, T035-058). The attempt driver is built here in slices;
//! the pieces that do not need the connection to the rest of the runner come first, each with its tests.

pub mod blobs;
pub mod connection;
#[cfg(test)]
pub(crate) mod doubles;
pub mod driver;
#[cfg(test)]
pub(crate) mod fixtures;
pub mod prepare;
pub mod settings;
pub mod sweep;
