/// A small runtime type descriptor.
///
/// This deliberately does not use Rust's `TypeId`: `TypeId` is only meaningful
/// inside one Rust program, while this graph is being shaped so Swift or another
/// rule provider can participate. In a real bridge this could become a stable
/// ABI descriptor, metadata pointer, mangled type name, or host-owned type key.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TypeDescriptor {
    name: &'static str,
}

impl TypeDescriptor {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }
}

/// Type-erased value storage for this first runtime pass.
///
/// The graph should not need to know whether a value came from Rust, Swift, or
/// some other host. So it stores a type descriptor plus bytes. This is still a
/// simplified model: a production graph would likely attach clone/drop/compare
/// callbacks to the type descriptor so non-trivial values can be owned safely and
/// compared efficiently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueStorage {
    value_type: TypeDescriptor,
    bytes: Vec<u8>,
    comparison: ValueComparison,
}

/// How the graph decides whether a recomputed value meaningfully changed.
///
/// `Bytewise` is enough for this first implementation because our current test
/// values are simple byte-backed scalars and static strings. A real Swift or
/// cross-language host would likely replace or extend this with a host-supplied
/// comparison callback on the type descriptor, so values can use Swift `Equatable`,
/// identity checks, bitwise comparison, or "always changed" semantics as needed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueComparison {
    Bytewise,
    AlwaysChanged,
}

impl ValueStorage {
    pub fn from_bytes(value_type: TypeDescriptor, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            value_type,
            bytes: bytes.into(),
            comparison: ValueComparison::Bytewise,
        }
    }

    pub fn with_comparison(mut self, comparison: ValueComparison) -> Self {
        self.comparison = comparison;
        self
    }

    pub fn from_bool(value: bool) -> Self {
        Self::from_bytes(TypeDescriptor::new("bool"), [u8::from(value)])
    }

    pub fn from_i64(value: i64) -> Self {
        Self::from_bytes(TypeDescriptor::new("i64"), value.to_ne_bytes())
    }

    pub fn from_static_str(value: &'static str) -> Self {
        Self::from_bytes(TypeDescriptor::new("&'static str"), value.as_bytes())
    }

    pub const fn value_type(&self) -> TypeDescriptor {
        self.value_type
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn comparison(&self) -> ValueComparison {
        self.comparison
    }

    pub fn meaningfully_changed_from(&self, old: Option<&ValueStorage>) -> bool {
        let Some(old) = old else {
            return true;
        };

        if self.value_type != old.value_type {
            return true;
        }

        match (old.comparison, self.comparison) {
            (ValueComparison::AlwaysChanged, _) | (_, ValueComparison::AlwaysChanged) => true,
            (ValueComparison::Bytewise, ValueComparison::Bytewise) => self.bytes != old.bytes,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        (self.value_type == TypeDescriptor::new("bool") && self.bytes.len() == 1)
            .then(|| self.bytes[0] != 0)
    }

    pub fn as_i64(&self) -> Option<i64> {
        if self.value_type != TypeDescriptor::new("i64") || self.bytes.len() != 8 {
            return None;
        }

        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.bytes);
        Some(i64::from_ne_bytes(bytes))
    }

    pub fn as_static_str(&self) -> Option<&str> {
        (self.value_type == TypeDescriptor::new("&'static str"))
            .then(|| std::str::from_utf8(&self.bytes).ok())
            .flatten()
    }
}

/// Rust-side values that can move through the typed Attribute API.
///
/// The graph still stores bytes internally so foreign hosts can plug in later,
/// but normal Rust callers should read and write `T` instead of manually building
/// `ValueStorage`.
pub trait AttributeValue: Clone + Sized + 'static {
    fn type_descriptor() -> TypeDescriptor;
    fn into_storage(self) -> ValueStorage;
    fn from_storage(storage: &ValueStorage) -> Option<Self>;
}

impl AttributeValue for bool {
    fn type_descriptor() -> TypeDescriptor {
        TypeDescriptor::new("bool")
    }

    fn into_storage(self) -> ValueStorage {
        ValueStorage::from_bool(self)
    }

    fn from_storage(storage: &ValueStorage) -> Option<Self> {
        storage.as_bool()
    }
}

impl AttributeValue for i64 {
    fn type_descriptor() -> TypeDescriptor {
        TypeDescriptor::new("i64")
    }

    fn into_storage(self) -> ValueStorage {
        ValueStorage::from_i64(self)
    }

    fn from_storage(storage: &ValueStorage) -> Option<Self> {
        storage.as_i64()
    }
}

impl AttributeValue for String {
    fn type_descriptor() -> TypeDescriptor {
        TypeDescriptor::new("String")
    }

    fn into_storage(self) -> ValueStorage {
        ValueStorage::from_bytes(Self::type_descriptor(), self.into_bytes())
    }

    fn from_storage(storage: &ValueStorage) -> Option<Self> {
        if storage.value_type() != Self::type_descriptor() {
            return None;
        }

        String::from_utf8(storage.bytes().to_vec()).ok()
    }
}
