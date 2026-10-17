//! Bound live target sessions across attempts, including narrower policies reloaded at runtime.
use sagat_domain::{database::DatabaseFailure, identity::NamespaceId};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
type Key = (NamespaceId, uuid::Uuid);

#[derive(Default)]
pub(crate) struct Slots(Mutex<HashMap<Key, u32>>);
impl Slots {
    pub fn acquire(self: &Arc<Self>, key: Key, limit: u32) -> Result<Permit, DatabaseFailure> {
        let mut slots = self.0.lock().map_err(|_| DatabaseFailure::Capacity)?;
        let used = slots.get(&key).copied().unwrap_or(0);
        if used >= limit {
            return Err(DatabaseFailure::Capacity);
        }
        slots.insert(key, used + 1);
        Ok(Permit {
            slots: self.clone(),
            key,
        })
    }
}
pub(crate) struct Permit {
    slots: Arc<Slots>,
    key: Key,
}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.slots.0.lock()
            && let Some(used) = slots.get_mut(&self.key)
        {
            *used -= 1;
            if *used == 0 {
                slots.remove(&self.key);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_target_capacity_reloads_narrower_limits_and_releases_entries() {
        let slots = Arc::new(Slots::default());
        let key = (NamespaceId::new(uuid::Uuid::new_v4()), uuid::Uuid::new_v4());
        let first = slots.acquire(key, 2).unwrap_or_else(|_| unreachable!());
        let second = slots.acquire(key, 2).unwrap_or_else(|_| unreachable!());
        assert!(slots.acquire(key, 1).is_err());
        drop(first);
        assert!(slots.acquire(key, 1).is_err());
        drop(second);
        assert_eq!(slots.0.lock().map(|m| m.len()).ok(), Some(0));
        assert!(slots.acquire(key, 1).is_ok());
    }
}
