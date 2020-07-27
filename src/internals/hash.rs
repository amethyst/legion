use std::hash::Hasher;

/// A hasher optimized for hashing component type IDs.
#[derive(Default)]
pub struct ComponentTypeIdHasher(u64);

impl Hasher for ComponentTypeIdHasher {
    fn finish(&self) -> u64 { self.0 }

    fn write(&mut self, bytes: &[u8]) {
        use core::convert::TryInto;
        self.0 = u64::from_ne_bytes(bytes.try_into().unwrap());
    }
}

/// A hasher optimized for hashing types that are represented as a u64.
#[derive(Default)]
pub struct U64Hasher(u64);

impl Hasher for U64Hasher {
    fn finish(&self) -> u64 { self.0 }

    fn write(&mut self, bytes: &[u8]) {
        use core::convert::TryInto;
        let seed = u64::from_ne_bytes(bytes.try_into().unwrap());
        let max_prime = 11_400_714_819_323_198_549u64;
        self.0 = max_prime.wrapping_mul(seed);
    }
}

#[test]
fn hasher() {
    fn verify<T: 'static + ?Sized>() {
        use core::any::TypeId;
        use core::hash::Hash;

        let mut hasher = ComponentTypeIdHasher::default();
        let type_id = TypeId::of::<T>();
        type_id.hash(&mut hasher);
        assert_eq!(hasher.finish(), unsafe {
            core::mem::transmute::<TypeId, u64>(type_id)
        });
    }

    verify::<usize>();
    verify::<()>();
    verify::<str>();
    verify::<&'static str>();
    verify::<[u8; 20]>();
}
