//! The schedule store (feature 018): schedules, their immutable revisions, the actions taken on
//! their nominal times, backfills and the owed cancellations of `CANCEL_OTHER`.
//!
//! PENDING (T018-014): the migration `20261230000000_schedules` and the storage classification
//! already name this file as the writer of those tables' columns. No statement is issued from here
//! yet; the store arrives with its tests.
