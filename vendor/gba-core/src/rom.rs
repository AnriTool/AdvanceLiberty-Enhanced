//! Shared cartridge bytes for isolated research forks. Mutations detach first,
//! so runtime patches and language arenas cannot affect an existing snapshot.
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Rom(Arc<Vec<u8>>);
impl Rom {
    pub fn shared(&self) -> Arc<Vec<u8>> {
        self.0.clone()
    }
    pub fn into_vec(self) -> Vec<u8> {
        Arc::try_unwrap(self.0).unwrap_or_else(|v| (*v).clone())
    }
}
impl From<Vec<u8>> for Rom {
    fn from(value: Vec<u8>) -> Self {
        Self(Arc::new(value))
    }
}
impl Deref for Rom {
    type Target = Vec<u8>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for Rom {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forks_share_reads_and_detach_for_patches_resize_and_clear() {
        let mut live = Rom::from(vec![1, 2, 3]);
        let mut fork = live.clone();
        assert!(Arc::ptr_eq(&live.shared(), &fork.shared()));
        live[0] = 7;
        fork.extend_from_slice(&[4, 5]);
        assert_eq!(&live[..], &[7, 2, 3]);
        assert_eq!(&fork[..], &[1, 2, 3, 4, 5]);
        let previous = live.clone();
        live.clear();
        assert_eq!(previous.into_vec(), [7, 2, 3]);
        assert!(live.is_empty());
    }
}
