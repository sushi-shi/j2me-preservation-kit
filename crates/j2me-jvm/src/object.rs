//! Opaque Java object identities and host-owned object operations.
//!
//! Strict transliterations cannot represent a Java `Object` as a Rust enum of
//! copied values: Java observes allocation identity, aliases, nullable
//! reference arrays, casts, and dynamically dispatched `Object.equals` calls.
//! This module supplies the small common boundary needed to preserve those
//! observations without embedding a heap, garbage collector, or class loader
//! in `j2me-jvm`.
//!
//! The host owns every object and class table. A game stores only
//! [`JavaObjectRef`] (with `Option<JavaObjectRef>` representing Java `null`) and
//! invokes operations through [`JavaObjectRuntime`]. Hosts must keep a handle's
//! identity stable while it can still be observed and must execute callbacks in
//! source evaluation order. In particular, they must not implement
//! [`object_equals`] by comparing handles or Rust values: `equals` is a virtual
//! Java call and can be overridden, re-enter game code, or fail.

use std::num::NonZeroU32;

use crate::JavaError;

/// An opaque, non-null Java object identity owned by the host runtime.
///
/// Zero is deliberately excluded so [`Option<JavaObjectRef>`] has an
/// unambiguous representation for Java `null`. The numeric value has no Java
/// semantics and must not be used as a value hash or ordering key by game code.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct JavaObjectRef(NonZeroU32);

impl JavaObjectRef {
    /// Wraps a host arena identifier. Returns `None` for zero.
    #[inline]
    pub const fn from_raw(raw: u32) -> Option<Self> {
        match NonZeroU32::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    /// Exposes the identifier solely for host arena indexing and tracing.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0.get()
    }
}

/// An opaque host token for a resolved Java class or interface.
///
/// Resolution/loading belongs to the host. Passing a token into this layer
/// means that any resolution side effects have already happened at the exact
/// bytecode cut point chosen by the transliteration.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct JavaClassRef(NonZeroU32);

impl JavaClassRef {
    /// Wraps a host class-table identifier. Returns `None` for zero.
    #[inline]
    pub const fn from_raw(raw: u32) -> Option<Self> {
        match NonZeroU32::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    /// Exposes the identifier solely for host class-table lookup and tracing.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0.get()
    }
}

/// Host callbacks for identity-bearing `java.lang.Object` operations.
///
/// This is intentionally not a VM. Implementors provide the object arena,
/// class relationships, allocation, array-store checks, exact UTF-16 String
/// payloads, and Java virtual dispatch. Every method may return a held host or
/// game failure; callers must therefore preserve call order and must not cache
/// mutable game state across an invocation.
pub trait JavaObjectRuntime {
    /// The port's complete throwable/failure domain.
    ///
    /// `From<JavaError>` lets the generic wrappers create the JVM exceptions
    /// whose cut points they own while leaving callback-specific failures
    /// untouched.
    type Error: From<JavaError>;

    /// Implements `new Integer(value)`.
    ///
    /// Every successful invocation must allocate a fresh identity, even for two
    /// equal values. This is constructor behavior, not `Integer.valueOf`.
    fn allocate_integer(&mut self, value: i32) -> Result<JavaObjectRef, Self::Error>;

    /// Implements `anewarray` after its non-negative length check.
    ///
    /// `component_class` is the resolved component class from the constant
    /// pool. The result must be a fresh array whose elements are all Java
    /// `null`; later stores must enforce the runtime component type.
    fn allocate_reference_array(
        &mut self,
        component_class: JavaClassRef,
        length: i32,
    ) -> Result<JavaObjectRef, Self::Error>;

    /// Performs the host's type test for a known non-null object.
    fn is_instance_of_non_null(
        &mut self,
        object: JavaObjectRef,
        class: JavaClassRef,
    ) -> Result<bool, Self::Error>;

    /// Reads the length of a known non-null Java reference array.
    fn reference_array_length_non_null(&mut self, array: JavaObjectRef)
        -> Result<i32, Self::Error>;

    /// Implements `aaload` for a known non-null Java reference array.
    fn reference_array_get_non_null(
        &mut self,
        array: JavaObjectRef,
        index: i32,
    ) -> Result<Option<JavaObjectRef>, Self::Error>;

    /// Implements `aastore` for a known non-null Java reference array.
    ///
    /// The host owns the bounds and component-assignability checks and must not
    /// mutate the array when either check fails.
    fn reference_array_set_non_null(
        &mut self,
        array: JavaObjectRef,
        index: i32,
        value: Option<JavaObjectRef>,
    ) -> Result<(), Self::Error>;

    /// Invokes `Integer.toString()` on a known non-null Integer object.
    ///
    /// The returned handle is the actual Java String result; allocation and
    /// identity policy remain host-owned.
    fn integer_to_string_non_null(
        &mut self,
        integer: JavaObjectRef,
    ) -> Result<JavaObjectRef, Self::Error>;

    /// Borrows the exact UTF-16 code units of a known non-null Java String.
    ///
    /// No Unicode normalization or lossy UTF-8 conversion is permitted.
    fn string_utf16_non_null(&self, string: JavaObjectRef) -> Result<&[u16], Self::Error>;

    /// Performs virtual `receiver.equals(argument)` dispatch.
    ///
    /// `receiver` is non-null; `argument` may be Java `null`. The host must
    /// dispatch against the receiver's runtime class and preserve any re-entry
    /// or failure instead of substituting handle or Rust value equality.
    fn invoke_object_equals(
        &mut self,
        receiver: JavaObjectRef,
        argument: Option<JavaObjectRef>,
    ) -> Result<bool, Self::Error>;
}

/// Implements Java `new Integer(value)` through the host arena.
#[inline]
pub fn new_integer<R: JavaObjectRuntime + ?Sized>(
    runtime: &mut R,
    value: i32,
) -> Result<JavaObjectRef, R::Error> {
    runtime.allocate_integer(value)
}

/// Implements Java `anewarray`, including the pre-allocation negative check.
#[inline]
pub fn new_reference_array<R: JavaObjectRuntime + ?Sized>(
    runtime: &mut R,
    component_class: JavaClassRef,
    length: i32,
) -> Result<JavaObjectRef, R::Error> {
    if length < 0 {
        Err(JavaError::NegativeArraySize { length }.into())
    } else {
        runtime.allocate_reference_array(component_class, length)
    }
}

/// Implements Java `instanceof`; a null operand is false without host dispatch.
#[inline]
pub fn instance_of<R: JavaObjectRuntime + ?Sized>(
    runtime: &mut R,
    object: Option<JavaObjectRef>,
    class: JavaClassRef,
) -> Result<bool, R::Error> {
    match object {
        Some(object) => runtime.is_instance_of_non_null(object, class),
        None => Ok(false),
    }
}

/// Implements Java `checkcast`; null passes through and a successful cast keeps
/// the exact input identity.
#[inline]
pub fn check_cast<R: JavaObjectRuntime + ?Sized>(
    runtime: &mut R,
    object: Option<JavaObjectRef>,
    class: JavaClassRef,
) -> Result<Option<JavaObjectRef>, R::Error> {
    let Some(object) = object else {
        return Ok(None);
    };
    if runtime.is_instance_of_non_null(object, class)? {
        Ok(Some(object))
    } else {
        Err(JavaError::ClassCast.into())
    }
}

/// Reads a Java reference-array length, throwing before host dispatch on null.
#[inline]
pub fn reference_array_length<R: JavaObjectRuntime + ?Sized>(
    runtime: &mut R,
    array: Option<JavaObjectRef>,
) -> Result<i32, R::Error> {
    runtime.reference_array_length_non_null(require_non_null(array)?)
}

/// Implements `aaload`, throwing before host dispatch on a null array.
#[inline]
pub fn reference_array_get<R: JavaObjectRuntime + ?Sized>(
    runtime: &mut R,
    array: Option<JavaObjectRef>,
    index: i32,
) -> Result<Option<JavaObjectRef>, R::Error> {
    runtime.reference_array_get_non_null(require_non_null(array)?, index)
}

/// Implements `aastore`, throwing before host dispatch on a null array.
#[inline]
pub fn reference_array_set<R: JavaObjectRuntime + ?Sized>(
    runtime: &mut R,
    array: Option<JavaObjectRef>,
    index: i32,
    value: Option<JavaObjectRef>,
) -> Result<(), R::Error> {
    runtime.reference_array_set_non_null(require_non_null(array)?, index, value)
}

/// Invokes `Integer.toString()`, throwing before host dispatch on null.
#[inline]
pub fn integer_to_string<R: JavaObjectRuntime + ?Sized>(
    runtime: &mut R,
    integer: Option<JavaObjectRef>,
) -> Result<JavaObjectRef, R::Error> {
    runtime.integer_to_string_non_null(require_non_null(integer)?)
}

/// Borrows a Java String's UTF-16 units, throwing before host access on null.
#[inline]
pub fn string_utf16<R: JavaObjectRuntime + ?Sized>(
    runtime: &R,
    string: Option<JavaObjectRef>,
) -> Result<&[u16], R::Error> {
    runtime.string_utf16_non_null(require_non_null(string)?)
}

/// Invokes virtual Java `Object.equals`, throwing before dispatch on a null
/// receiver and passing a null argument through unchanged.
#[inline]
pub fn object_equals<R: JavaObjectRuntime + ?Sized>(
    runtime: &mut R,
    receiver: Option<JavaObjectRef>,
    argument: Option<JavaObjectRef>,
) -> Result<bool, R::Error> {
    runtime.invoke_object_equals(require_non_null(receiver)?, argument)
}

#[inline]
fn require_non_null<E: From<JavaError>>(object: Option<JavaObjectRef>) -> Result<JavaObjectRef, E> {
    object.ok_or_else(|| JavaError::NullPointer.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Eq, PartialEq)]
    struct HeldFailure(&'static str);

    #[derive(Debug, Eq, PartialEq)]
    enum TestError<'a> {
        Java(JavaError),
        Held(&'a HeldFailure),
    }

    impl From<JavaError> for TestError<'_> {
        fn from(value: JavaError) -> Self {
            Self::Java(value)
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Operation {
        AllocateInteger,
        TypeTest,
        ArrayGet,
        ArraySet,
        Equals,
    }

    #[derive(Debug)]
    enum TestObject {
        Integer(i32),
        String(Vec<u16>),
        Array {
            component: JavaClassRef,
            elements: Vec<Option<JavaObjectRef>>,
        },
        Other,
    }

    struct TestRuntime<'a> {
        objects: Vec<TestObject>,
        failure: Option<(Operation, &'a HeldFailure)>,
        type_tests: usize,
        array_calls: usize,
        equals_calls: usize,
        equals_override: Option<bool>,
    }

    impl<'a> TestRuntime<'a> {
        fn new() -> Self {
            Self {
                objects: Vec::new(),
                failure: None,
                type_tests: 0,
                array_calls: 0,
                equals_calls: 0,
                equals_override: None,
            }
        }

        fn alloc(&mut self, object: TestObject) -> JavaObjectRef {
            self.objects.push(object);
            JavaObjectRef::from_raw(self.objects.len() as u32).unwrap()
        }

        fn object(&self, object: JavaObjectRef) -> Result<&TestObject, TestError<'a>> {
            self.objects
                .get((object.raw() - 1) as usize)
                .ok_or_else(|| JavaError::IllegalState("unknown Java object handle").into())
        }

        fn fail(&self, operation: Operation) -> Result<(), TestError<'a>> {
            match self.failure {
                Some((configured, held)) if configured == operation => Err(TestError::Held(held)),
                _ => Ok(()),
            }
        }
    }

    impl<'a> JavaObjectRuntime for TestRuntime<'a> {
        type Error = TestError<'a>;

        fn allocate_integer(&mut self, value: i32) -> Result<JavaObjectRef, Self::Error> {
            self.fail(Operation::AllocateInteger)?;
            Ok(self.alloc(TestObject::Integer(value)))
        }

        fn allocate_reference_array(
            &mut self,
            component_class: JavaClassRef,
            length: i32,
        ) -> Result<JavaObjectRef, Self::Error> {
            assert!(length >= 0, "wrapper must reject negative lengths first");
            Ok(self.alloc(TestObject::Array {
                component: component_class,
                elements: vec![None; length as usize],
            }))
        }

        fn is_instance_of_non_null(
            &mut self,
            object: JavaObjectRef,
            class: JavaClassRef,
        ) -> Result<bool, Self::Error> {
            self.type_tests += 1;
            self.fail(Operation::TypeTest)?;
            Ok(match self.object(object)? {
                TestObject::Integer(_) => class.raw() == 1,
                TestObject::String(_) => class.raw() == 2,
                TestObject::Array { .. } => class.raw() == 3,
                TestObject::Other => class.raw() == 4,
            })
        }

        fn reference_array_length_non_null(
            &mut self,
            array: JavaObjectRef,
        ) -> Result<i32, Self::Error> {
            self.array_calls += 1;
            match self.object(array)? {
                TestObject::Array { elements, .. } => Ok(elements.len() as i32),
                _ => Err(JavaError::ClassCast.into()),
            }
        }

        fn reference_array_get_non_null(
            &mut self,
            array: JavaObjectRef,
            index: i32,
        ) -> Result<Option<JavaObjectRef>, Self::Error> {
            self.array_calls += 1;
            self.fail(Operation::ArrayGet)?;
            match self.object(array)? {
                TestObject::Array { elements, .. } => {
                    elements.get(index as usize).copied().ok_or_else(|| {
                        JavaError::ArrayIndexOutOfBounds {
                            index,
                            length: elements.len() as i32,
                        }
                        .into()
                    })
                }
                _ => Err(JavaError::ClassCast.into()),
            }
        }

        fn reference_array_set_non_null(
            &mut self,
            array: JavaObjectRef,
            index: i32,
            value: Option<JavaObjectRef>,
        ) -> Result<(), Self::Error> {
            self.array_calls += 1;
            self.fail(Operation::ArraySet)?;
            let object_count = self.objects.len();
            if value.is_some_and(|value| value.raw() as usize > object_count) {
                return Err(JavaError::IllegalState("unknown Java object handle").into());
            }
            let TestObject::Array {
                component,
                elements,
            } = self
                .objects
                .get_mut((array.raw() - 1) as usize)
                .ok_or(JavaError::IllegalState("unknown Java object handle"))?
            else {
                return Err(JavaError::ClassCast.into());
            };
            let length = elements.len() as i32;
            let slot = elements.get_mut(index as usize).ok_or(TestError::Java(
                JavaError::ArrayIndexOutOfBounds { index, length },
            ))?;
            let _component_type_is_host_owned = component;
            *slot = value;
            Ok(())
        }

        fn integer_to_string_non_null(
            &mut self,
            integer: JavaObjectRef,
        ) -> Result<JavaObjectRef, Self::Error> {
            let value = match self.object(integer)? {
                TestObject::Integer(value) => *value,
                _ => return Err(JavaError::ClassCast.into()),
            };
            Ok(self.alloc(TestObject::String(
                value.to_string().encode_utf16().collect(),
            )))
        }

        fn string_utf16_non_null(&self, string: JavaObjectRef) -> Result<&[u16], Self::Error> {
            match self.object(string)? {
                TestObject::String(units) => Ok(units),
                _ => Err(JavaError::ClassCast.into()),
            }
        }

        fn invoke_object_equals(
            &mut self,
            receiver: JavaObjectRef,
            argument: Option<JavaObjectRef>,
        ) -> Result<bool, Self::Error> {
            self.equals_calls += 1;
            self.fail(Operation::Equals)?;
            if let Some(result) = self.equals_override {
                return Ok(result);
            }
            let Some(argument) = argument else {
                return Ok(false);
            };
            Ok(match (self.object(receiver)?, self.object(argument)?) {
                (TestObject::Integer(left), TestObject::Integer(right)) => left == right,
                (TestObject::String(left), TestObject::String(right)) => left == right,
                _ => receiver == argument,
            })
        }
    }

    fn class(raw: u32) -> JavaClassRef {
        JavaClassRef::from_raw(raw).unwrap()
    }

    #[test]
    fn handles_are_copy_and_java_null_is_separate() {
        let object = JavaObjectRef::from_raw(7).unwrap();
        let copied = object;
        assert_eq!(copied.raw(), 7);
        assert_eq!(JavaObjectRef::from_raw(0), None);
        assert_eq!(JavaClassRef::from_raw(0), None);
        assert_eq!(Some(object), Some(copied));
    }

    #[test]
    fn constructor_and_reference_array_preserve_fresh_identity_and_aliases() {
        let mut runtime = TestRuntime::new();
        let first = new_integer(&mut runtime, 23).unwrap();
        let second = new_integer(&mut runtime, 23).unwrap();
        assert_ne!(first, second, "new Integer must not intern equal values");

        let array = new_reference_array(&mut runtime, class(4), 2).unwrap();
        assert_eq!(reference_array_length(&mut runtime, Some(array)), Ok(2));
        assert_eq!(reference_array_get(&mut runtime, Some(array), 0), Ok(None));
        reference_array_set(&mut runtime, Some(array), 0, Some(first)).unwrap();
        reference_array_set(&mut runtime, Some(array), 1, Some(first)).unwrap();
        assert_eq!(
            reference_array_get(&mut runtime, Some(array), 0),
            Ok(Some(first))
        );
        assert_eq!(
            reference_array_get(&mut runtime, Some(array), 1),
            Ok(Some(first))
        );
    }

    #[test]
    fn wrappers_own_null_negative_length_and_cast_cut_points() {
        let mut runtime = TestRuntime::new();
        assert_eq!(instance_of(&mut runtime, None, class(1)), Ok(false));
        assert_eq!(check_cast(&mut runtime, None, class(1)), Ok(None));
        assert_eq!(runtime.type_tests, 0, "null must suppress host type tests");

        assert_eq!(
            new_reference_array(&mut runtime, class(4), -9),
            Err(TestError::Java(JavaError::NegativeArraySize { length: -9 }))
        );
        assert!(
            runtime.objects.is_empty(),
            "failure must precede allocation"
        );
        assert_eq!(
            reference_array_get(&mut runtime, None, 0),
            Err(TestError::Java(JavaError::NullPointer))
        );
        assert_eq!(runtime.array_calls, 0, "null must suppress array access");

        let object = runtime.alloc(TestObject::Other);
        assert_eq!(
            check_cast(&mut runtime, Some(object), class(1)),
            Err(TestError::Java(JavaError::ClassCast))
        );
        assert_eq!(
            check_cast(&mut runtime, Some(object), class(4)),
            Ok(Some(object))
        );
    }

    #[test]
    fn integer_string_boundary_preserves_the_result_handle_and_utf16() {
        let mut runtime = TestRuntime::new();
        let integer = new_integer(&mut runtime, i32::MIN).unwrap();
        let string = integer_to_string(&mut runtime, Some(integer)).unwrap();
        let expected: Vec<u16> = "-2147483648".encode_utf16().collect();
        assert_eq!(
            string_utf16(&runtime, Some(string)),
            Ok(expected.as_slice())
        );
        assert_eq!(
            string_utf16(&runtime, None),
            Err(TestError::Java(JavaError::NullPointer))
        );

        let hostile_units = vec![0xd800, b'A' as u16, 0xdc00, 0xd83d, 0xde03];
        let hostile_string = runtime.alloc(TestObject::String(hostile_units.clone()));
        assert_eq!(
            string_utf16(&runtime, Some(hostile_string)),
            Ok(hostile_units.as_slice()),
            "String access must not normalize malformed or non-BMP UTF-16"
        );
    }

    #[test]
    fn virtual_equals_is_not_replaced_by_handle_or_value_equality() {
        let mut runtime = TestRuntime::new();
        let first = new_integer(&mut runtime, 5).unwrap();
        let second = new_integer(&mut runtime, 5).unwrap();

        runtime.equals_override = Some(true);
        assert_eq!(
            object_equals(&mut runtime, Some(first), Some(second)),
            Ok(true)
        );
        runtime.equals_override = Some(false);
        assert_eq!(
            object_equals(&mut runtime, Some(first), Some(first)),
            Ok(false)
        );
        assert_eq!(
            runtime.equals_calls, 2,
            "both calls must dispatch virtually"
        );

        assert_eq!(
            object_equals(&mut runtime, None, Some(second)),
            Err(TestError::Java(JavaError::NullPointer))
        );
        assert_eq!(
            runtime.equals_calls, 2,
            "null receiver must fail before dispatch"
        );
    }

    #[test]
    fn held_host_failures_pass_through_unchanged() {
        let held = HeldFailure("oracle cut");
        let mut runtime = TestRuntime::new();
        runtime.failure = Some((Operation::AllocateInteger, &held));
        let error = new_integer(&mut runtime, 1).unwrap_err();
        assert!(matches!(error, TestError::Held(value) if std::ptr::eq(value, &held)));

        runtime.failure = Some((Operation::TypeTest, &held));
        let object = runtime.alloc(TestObject::Other);
        let error = check_cast(&mut runtime, Some(object), class(4)).unwrap_err();
        assert!(matches!(error, TestError::Held(value) if std::ptr::eq(value, &held)));

        runtime.failure = Some((Operation::Equals, &held));
        let error = object_equals(&mut runtime, Some(object), Some(object)).unwrap_err();
        assert!(matches!(error, TestError::Held(value) if std::ptr::eq(value, &held)));

        runtime.failure = None;
        let array = new_reference_array(&mut runtime, class(4), 1).unwrap();
        runtime.failure = Some((Operation::ArraySet, &held));
        let error = reference_array_set(&mut runtime, Some(array), 0, Some(object)).unwrap_err();
        assert!(matches!(error, TestError::Held(value) if std::ptr::eq(value, &held)));
        runtime.failure = None;
        assert_eq!(
            reference_array_get(&mut runtime, Some(array), 0),
            Ok(None),
            "a failed host store must not mutate the array"
        );
    }
}
