//! Contains types related to read and write access declarations. Used by the scheduler and
//! by [SubWorld](../world/struct.SubWorld.html) splits.

use smallvec::SmallVec;
use std::fmt::{Debug, Display};

/// Describes read and write access to resources.
#[derive(Clone)]
pub struct Permissions<T: PartialEq> {
    items: SmallVec<[T; 4]>,
    shared: usize, // index of first shared
    write: usize,  // index of first write exclusive
}

impl<T: PartialEq> Permissions<T> {
    /// Constructs a new empty permissions set.
    pub fn new() -> Self {
        Self {
            items: SmallVec::default(),
            shared: 0,
            write: 0,
        }
    }

    fn find(&self, item: &T) -> Option<usize> {
        self.items.iter().position(|x| x == item)
    }

    /// Adds a resource to the permissions set as both readable and writable.
    pub fn push(&mut self, item: T) {
        if let Some(index) = self.find(&item) {
            if index < self.shared {
                // if it is in read exclusive, move it up into shared
                self.items.swap(index, self.shared - 1);
                self.shared -= 1;
            } else if index > self.write {
                // if it is in write exclusive, move it down into shared
                self.items.swap(index, self.write);
                self.write += 1;
            }
        } else {
            // add the item
            self.items.push(item);

            // swap it down into shared
            let index = self.items.len() - 1;
            self.items.swap(index, self.write);
            self.write += 1;
        }
    }

    /// Adds a resource to the permissions set as readable.
    pub fn push_read(&mut self, item: T) {
        if let Some(index) = self.find(&item) {
            // if the item had exclusive write, move it into shared
            if index >= self.write {
                // swap it down to the beginning of the exclusive write segment,
                // then move the boundry over it
                self.items.swap(index, self.write);
                self.write += 1;
            }
        } else {
            // add the item to the end of the vec
            self.items.push(item);
            let index = self.items.len() - 1;

            // move it down into shared
            self.items.swap(index, self.write);

            // move it down into read exclusive
            self.items.swap(self.write, self.shared);

            // move the boundaries
            self.write += 1;
            self.shared += 1;
        }
    }

    /// Adds a resource to the permissions set as writable.
    pub fn push_write(&mut self, item: T) {
        if let Some(index) = self.find(&item) {
            if index < self.shared {
                // move it into shared
                self.items.swap(index, self.shared - 1);
                self.shared -= 1;
            }
        } else {
            // add the item to the end of the vec
            self.items.push(item);
        }
    }

    /// Removes a resource from the permissions set.
    pub fn remove(&mut self, item: &T) {
        if let Some(mut index) = self.find(item) {
            if index < self.shared {
                // push value up into shared
                self.items.swap(index, self.shared - 1);
                self.shared -= 1;
                index = self.shared;
            }

            if index < self.write {
                // push value up into write
                self.items.swap(index, self.write - 1);
                self.write -= 1;
                index = self.write;
            }

            self.items.swap_remove(index);
        }
    }

    /// Removes read access to a resource from the permissions set.
    pub fn remove_read(&mut self, item: &T) {
        if let Some(index) = self.find(item) {
            if index < self.shared {
                // move into shared
                self.items.swap(index, self.shared - 1);
                self.shared -= 1;

                // move into write
                self.items.swap(self.shared, self.write - 1);
                self.write -= 1;

                // remove
                self.items.swap_remove(self.write);
            } else if index < self.write {
                // move into write-only
                self.items.swap(index, self.write - 1);
                self.write -= 1;
            }
        }
    }

    /// Removes write access to a resource from the permissions set.
    pub fn remove_write(&mut self, item: &T) {
        if let Some(index) = self.find(item) {
            if index >= self.write {
                // remove
                self.items.swap_remove(index);
            } else if index >= self.shared {
                // move into read-only
                self.items.swap(index, self.shared);
                self.shared += 1;
            }
        }
    }

    /// Adds all of the permissions contained in the given set to this permission set.
    pub fn add(&mut self, mut other: Self) {
        for read in other.items.drain(..other.shared) {
            self.push_read(read);
        }

        for shared in other.items.drain(..(other.write - other.shared)) {
            self.push(shared);
        }

        for write in other.items.drain(..) {
            self.push_write(write);
        }
    }

    /// Subtracts all of the permissions contained in the given set from this permission set.
    pub fn subtract(&mut self, other: &Self) {
        for read in other.reads_only() {
            self.remove_read(read);
        }

        for shared in other.readwrite() {
            self.remove(shared);
        }

        for write in other.writes_only() {
            self.remove_write(write);
        }
    }

    /// Gets a slice of resources which are afforded read access.
    pub fn reads(&self) -> &[T] {
        &self.items[..self.write]
    }

    /// Gets a slice of resources which are afforded write access.
    pub fn writes(&self) -> &[T] {
        &self.items[self.shared..]
    }

    /// Gets a slice of resources which are afforded read access but not write access.
    pub fn reads_only(&self) -> &[T] {
        &self.items[..self.shared]
    }

    /// Gets a slice of resources which are afforded write access but not read access.
    pub fn writes_only(&self) -> &[T] {
        &self.items[self.write..]
    }

    /// Gets a slice of resources which are afforded both read and write access.
    pub fn readwrite(&self) -> &[T] {
        &self.items[self.shared..self.write]
    }

    /// Returns `true` if all of the permissions in the given set are contained in this set.
    pub fn is_superset(&self, other: &Self) -> bool {
        for read in other.reads_only() {
            // exit if reads are in exclusive write range, or are not found
            if self.find(read).map(|i| i >= self.write).unwrap_or(true) {
                return false;
            }
        }

        for shared in other.readwrite() {
            // exit if shareds are in exclusive read or write range, or are not found
            if self
                .find(shared)
                .map(|i| i < self.shared || i >= self.write)
                .unwrap_or(true)
            {
                return false;
            }
        }

        for write in other.writes_only() {
            // exit if writes are in exclusive read range, or are not found
            if self.find(write).map(|i| i < self.shared).unwrap_or(true) {
                return false;
            }
        }

        true
    }

    /// Returns `true` if none of the permissions in the given set are contained in this set.
    pub fn is_disjoint(&self, other: &Self) -> bool {
        for read in other.reads_only() {
            // exit if reads are in read-only or shared range
            if self.find(read).map(|i| i < self.write).unwrap_or(false) {
                return false;
            }
        }

        for shared in other.readwrite() {
            // exit if shareds are found
            if self.find(shared).is_some() {
                return false;
            }
        }

        for write in other.writes_only() {
            // exit if writes are in write-only or shared range
            if self.find(write).map(|i| i >= self.shared).unwrap_or(false) {
                return false;
            }
        }

        true
    }
}

impl<T: PartialEq> Default for Permissions<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: PartialEq + Debug> Debug for Permissions<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn list<V: Debug>(items: &[V]) -> String {
            use itertools::Itertools;
            items
                .iter()
                .map(|x| format!("{:?}", x))
                .fold1(|x, y| format!("{}, {}", x, y))
                .unwrap_or_else(|| "".to_owned())
        }

        write!(
            f,
            "Permissions {{ reads: [{}], writes: [{}] }}",
            list::<T>(&self.reads()),
            list::<T>(&self.writes())
        )
    }
}

impl<T: PartialEq + Display> Display for Permissions<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn list<V: Display>(items: &[V]) -> String {
            use itertools::Itertools;
            items
                .iter()
                .map(|x| format!("{}", x))
                .fold1(|x, y| format!("{}, {}", x, y))
                .unwrap_or_else(|| "".to_owned())
        }

        write!(
            f,
            "reads: [{}], writes: [{}]",
            list::<T>(&self.reads()),
            list::<T>(&self.writes())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Permissions;

    #[test]
    fn push_read() {
        let mut permissions = Permissions::new();
        permissions.push_read(1usize);

        let empty: &[usize] = &[];
        assert_eq!(permissions.reads_only(), &[1usize]);
        assert_eq!(permissions.readwrite(), empty);
        assert_eq!(permissions.writes_only(), empty);
    }

    #[test]
    fn push_write() {
        let mut permissions = Permissions::new();
        permissions.push_write(1usize);

        let empty: &[usize] = &[];
        assert_eq!(permissions.reads_only(), empty);
        assert_eq!(permissions.readwrite(), empty);
        assert_eq!(permissions.writes_only(), &[1usize]);
    }

    #[test]
    fn push_both() {
        let mut permissions = Permissions::new();
        permissions.push(1usize);

        let empty: &[usize] = &[];
        assert_eq!(permissions.reads_only(), empty);
        assert_eq!(permissions.readwrite(), &[1usize]);
        assert_eq!(permissions.writes_only(), empty);
    }

    #[test]
    fn promote_read_to_readwrite() {
        let mut permissions = Permissions::new();
        permissions.push_read(1usize);
        permissions.push_write(1usize);

        let empty: &[usize] = &[];
        assert_eq!(permissions.reads_only(), empty);
        assert_eq!(permissions.readwrite(), &[1usize]);
        assert_eq!(permissions.writes_only(), empty);
    }

    #[test]
    fn promote_write_to_readwrite() {
        let mut permissions = Permissions::new();
        permissions.push_write(1usize);
        permissions.push_read(1usize);

        let empty: &[usize] = &[];
        assert_eq!(permissions.reads_only(), empty);
        assert_eq!(permissions.readwrite(), &[1usize]);
        assert_eq!(permissions.writes_only(), empty);
    }

    #[test]
    fn remove_write() {
        let mut permissions = Permissions::new();
        permissions.push_write(1usize);
        permissions.remove_write(&1usize);

        let empty: &[usize] = &[];
        assert_eq!(permissions.reads_only(), empty);
        assert_eq!(permissions.readwrite(), empty);
        assert_eq!(permissions.writes_only(), empty);
    }

    #[test]
    fn remove_read() {
        let mut permissions = Permissions::new();
        permissions.push_read(1usize);
        permissions.remove_read(&1usize);

        let empty: &[usize] = &[];
        assert_eq!(permissions.reads_only(), empty);
        assert_eq!(permissions.readwrite(), empty);
        assert_eq!(permissions.writes_only(), empty);
    }

    #[test]
    fn demote_readwrite_to_read() {
        let mut permissions = Permissions::new();
        permissions.push(1usize);
        permissions.remove_write(&1usize);

        let empty: &[usize] = &[];
        assert_eq!(permissions.reads_only(), &[1usize]);
        assert_eq!(permissions.readwrite(), empty);
        assert_eq!(permissions.writes_only(), empty);
    }

    #[test]
    fn demote_readwrite_to_write() {
        let mut permissions = Permissions::new();
        permissions.push(1usize);
        permissions.remove_read(&1usize);

        let empty: &[usize] = &[];
        assert_eq!(permissions.reads_only(), empty);
        assert_eq!(permissions.readwrite(), empty);
        assert_eq!(permissions.writes_only(), &[1usize]);
    }
}
