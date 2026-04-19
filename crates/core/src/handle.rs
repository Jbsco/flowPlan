//! Generational handles for the asset table.
//!
//! `Handle<T>` is a `u32` slot index plus a `u32` generation, with a `PhantomData<T>` marker
//! to prevent cross-type confusion. Handles remain stable across asset hot-reload: the slot
//! is reused but the generation increments, invalidating any stale `Handle<T>` bound to the
//! previous generation.
//!
//! Handles are `Copy`, `Eq`, `Hash`, and bytemuck-POD (the T type parameter does not touch
//! representation), suitable for storing in fixed-capacity pools and for sharing across
//! threads.

use core::fmt;
use core::marker::PhantomData;

/// An opaque, generational handle to a resource of type `T` in a handle table.
///
/// Construct via the issuing table's API; compare with `==`; use as a map key via `Hash`.
pub struct Handle<T> {
    slot: u32,
    generation: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    /// Construct a handle from raw components. Intended for use by handle tables only.
    #[inline]
    #[must_use]
    pub const fn new(slot: u32, generation: u32) -> Self {
        Self {
            slot,
            generation,
            _marker: PhantomData,
        }
    }

    /// A handle that points to nothing. Never matches a live slot.
    pub const INVALID: Self = Self {
        slot: u32::MAX,
        generation: u32::MAX,
        _marker: PhantomData,
    };

    /// Slot index into the asset table's backing vector.
    #[inline]
    #[must_use]
    pub const fn slot(self) -> u32 {
        self.slot
    }

    /// Generation for the slot. Matched against the table's stored generation to detect
    /// stale handles.
    #[inline]
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// Returns true if this handle is the sentinel `INVALID` value.
    #[inline]
    #[must_use]
    pub const fn is_invalid(self) -> bool {
        self.slot == u32::MAX && self.generation == u32::MAX
    }
}

// Manual trait derivations -- deriving them would require T: Copy/Clone, which is an
// unnecessary constraint on the phantom-marker type.
impl<T> Clone for Handle<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.slot == other.slot && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> core::hash::Hash for Handle<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.slot.hash(state);
        self.generation.hash(state);
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let type_name = core::any::type_name::<T>();
        f.debug_struct("Handle")
            .field("type", &type_name)
            .field("slot", &self.slot)
            .field("generation", &self.generation)
            .finish()
    }
}

impl<T> Default for Handle<T> {
    #[inline]
    fn default() -> Self {
        Self::INVALID
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mesh;
    struct Texture;

    #[test]
    fn invalid_is_distinct() {
        let h: Handle<Mesh> = Handle::INVALID;
        let g: Handle<Mesh> = Handle::new(0, 0);
        assert_ne!(h, g);
        assert!(h.is_invalid());
        assert!(!g.is_invalid());
    }

    #[test]
    fn same_slot_different_generation_is_different() {
        let a: Handle<Mesh> = Handle::new(5, 1);
        let b: Handle<Mesh> = Handle::new(5, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn same_slot_same_generation_is_equal() {
        let a: Handle<Mesh> = Handle::new(5, 1);
        let b: Handle<Mesh> = Handle::new(5, 1);
        assert_eq!(a, b);
    }

    #[test]
    fn handle_is_copy() {
        let a: Handle<Mesh> = Handle::new(7, 3);
        let b = a;
        // Copy again; proves `a` was not moved.
        let c = a;
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn default_is_invalid() {
        let h: Handle<Texture> = Handle::default();
        assert!(h.is_invalid());
    }

    #[test]
    fn slot_and_generation_accessors() {
        let h: Handle<Mesh> = Handle::new(42, 99);
        assert_eq!(h.slot(), 42);
        assert_eq!(h.generation(), 99);
    }

    #[test]
    fn debug_format_includes_type_name() {
        let h: Handle<Texture> = Handle::new(1, 2);
        let s = format!("{h:?}");
        assert!(s.contains("Texture"), "expected Texture in {s}");
        assert!(s.contains("slot: 1"));
        assert!(s.contains("generation: 2"));
    }

    // Cross-type confusion is a compile-time error by design. This is a documentation-only
    // compile_fail doctest equivalent:
    // let m: Handle<Mesh> = Handle::new(0, 0);
    // let t: Handle<Texture> = m; // ERROR: mismatched types

    #[test]
    fn hash_and_eq_compatible() {
        use std::collections::HashSet;
        let mut set: HashSet<Handle<Mesh>> = HashSet::new();
        set.insert(Handle::new(1, 1));
        set.insert(Handle::new(1, 1));
        set.insert(Handle::new(1, 2));
        assert_eq!(set.len(), 2);
    }
}
