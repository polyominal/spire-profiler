use std::collections::BTreeMap;

use super::*;

#[repr(C)]
#[derive(Clone, Copy)]
struct MockVariant {
    tag: c_int,
    object: ObjectPtr,
}

const _: () = assert!(std::mem::size_of::<MockVariant>() <= OPAQUE_SIZE);
const _: () = assert!(std::mem::align_of::<MockVariant>() <= std::mem::align_of::<Opaque>());

#[derive(Default)]
struct Mock {
    objects: BTreeMap<ObjectId, Box<u64>>,
    variants: BTreeMap<usize, c_int>,
    initialized: usize,
    destroyed: usize,
    double_init: bool,
    invalid_destroy: bool,
    stale_use: bool,
    calls: usize,
    error: c_int,
    return_object: Option<ObjectId>,
    unexpected: usize,
}

thread_local! {
    static MOCK: RefCell<Mock> = RefCell::new(Mock::default());
}

impl Mock {
    fn pointer(&self, id: ObjectId) -> ObjectPtr {
        self.objects.get(&id).map_or(ptr::null_mut(), |object| {
            (&**object as *const u64).cast_mut().cast()
        })
    }

    fn id(&mut self, pointer: ObjectPtr) -> ObjectId {
        let found = self.objects.iter().find_map(|(id, object)| {
            (std::ptr::eq((&**object as *const u64).cast::<c_void>(), pointer)).then_some(*id)
        });
        self.stale_use |= found.is_none();
        found.unwrap_or(0)
    }

    /// # Safety
    /// Destination has Opaque capacity/alignment; source points to the input
    /// type paired with TAG by the constructor table.
    unsafe extern "C" fn construct<const TAG: c_int>(destination: VariantPtr, source: TypePtr) {
        let object = if TAG == VT_OBJECT {
            // SAFETY: the OBJECT source is a live ObjectPtr local in from_typed.
            unsafe { source.cast::<ObjectPtr>().read() }
        } else {
            ptr::null_mut()
        };
        MOCK.with(|cell| {
            let mut mock = cell.borrow_mut();
            if TAG == VT_OBJECT {
                mock.id(object);
            }
            mock.double_init |= mock.variants.insert(destination as usize, TAG).is_some();
            mock.initialized += 1;
        });
        // SAFETY: the destination is exclusive aligned Opaque storage, large
        // enough for MockVariant. Its foreign padding need not be initialized.
        unsafe {
            destination
                .cast::<MockVariant>()
                .write(MockVariant { tag: TAG, object })
        };
    }

    extern "C" fn constructor(tag: c_int) -> Option<FromTypeConstructorFn> {
        match tag {
            VT_OBJECT => Some(Self::construct::<VT_OBJECT>),
            VT_BOOL => Some(Self::construct::<VT_BOOL>),
            VT_INT => Some(Self::construct::<VT_INT>),
            VT_FLOAT => Some(Self::construct::<VT_FLOAT>),
            VT_STRING => Some(Self::construct::<VT_STRING>),
            VT_VECTOR2 => Some(Self::construct::<VT_VECTOR2>),
            VT_RECT2 => Some(Self::construct::<VT_RECT2>),
            VT_COLOR => Some(Self::construct::<VT_COLOR>),
            _ => None,
        }
    }

    extern "C" fn lookup(id: ObjectId) -> ObjectPtr {
        MOCK.with(|cell| cell.borrow().pointer(id))
    }

    extern "C" fn object_id(pointer: ObjectPtr) -> ObjectId {
        MOCK.with(|cell| cell.borrow_mut().id(pointer))
    }

    /// # Safety
    /// Receiver and each argument were initialized by this mock; the result
    /// and error pointers designate exclusive writable output storage.
    unsafe extern "C" fn call(
        receiver: VariantPtr,
        _method: ConstStringNamePtr,
        args: *const ConstVariantPtr,
        count: GDExtensionInt,
        result: VariantPtr,
        error: *mut CallError,
    ) {
        // SAFETY: the signature's mock contract provides the initialized
        // receiver and argument array; zero arguments allow a null pointer.
        let (receiver, args) = unsafe {
            (
                receiver.cast::<MockVariant>().read(),
                if count == 0 {
                    &[][..]
                } else {
                    std::slice::from_raw_parts(args, count as usize)
                },
            )
        };
        let (returned, code) = MOCK.with(|cell| {
            let mut mock = cell.borrow_mut();
            mock.id(receiver.object);
            for arg in args {
                // SAFETY: each argument is a live MockVariant for this call.
                let arg = unsafe { arg.cast::<MockVariant>().read() };
                if arg.tag == VT_OBJECT {
                    mock.id(arg.object);
                }
            }
            mock.calls += 1;
            let returned = MockVariant {
                tag: if mock.return_object.is_some() {
                    VT_OBJECT
                } else {
                    0
                },
                object: mock
                    .return_object
                    .map_or(ptr::null_mut(), |id| mock.pointer(id)),
            };
            mock.double_init |= mock
                .variants
                .insert(result as usize, returned.tag)
                .is_some();
            mock.initialized += 1;
            (returned, mock.error)
        });
        // SAFETY: the two outputs are disjoint, aligned and exclusively
        // writable; return construction occurs even for a method error.
        unsafe {
            result.cast::<MockVariant>().write(returned);
            error.write(CallError {
                error: code,
                argument: 0,
                expected: 0,
            });
        }
    }

    extern "C" fn destroy(pointer: VariantPtr) {
        MOCK.with(|cell| {
            let mut mock = cell.borrow_mut();
            mock.invalid_destroy |= mock.variants.remove(&(pointer as usize)).is_none();
            mock.destroyed += 1;
        });
    }

    /// # Safety
    /// The pointer addresses a live value constructed by the mock.
    unsafe extern "C" fn variant_type(pointer: ConstVariantPtr) -> c_int {
        // SAFETY: callers obtain storage only from a constructed Variant.
        unsafe { (*pointer.cast::<MockVariant>()).tag }
    }

    /// # Safety
    /// The pointer addresses a live OBJECT MockVariant.
    unsafe extern "C" fn variant_object_id(pointer: ConstVariantPtr) -> ObjectId {
        // SAFETY: the OBJECT check precedes this read of initialized storage.
        let object = unsafe { (*pointer.cast::<MockVariant>()).object };
        MOCK.with(|cell| cell.borrow_mut().id(object))
    }

    extern "C" fn getter(_tag: c_int) -> Option<InternalGetterFn> {
        None
    }
}

const MOCK_API: Api = Api {
    classdb_construct_object: unused_lookup,
    classdb_register_extension_class5: unused_register,
    classdb_unregister_extension_class: unused_unregister,
    classdb_register_extension_class_method: unused_method,
    object_set_instance: unused_set_instance,
    object_get_instance_from_id: Mock::lookup,
    object_get_instance_id: Mock::object_id,
    global_get_singleton: unused_lookup,
    get_variant_from_type_constructor: Mock::constructor,
    variant_get_ptr_internal_getter: Mock::getter,
    variant_call: Mock::call,
    variant_destroy: Mock::destroy,
    variant_get_type: Mock::variant_type,
    variant_get_object_instance_id: Mock::variant_object_id,
    variant_get_ptr_destructor: unused_destructor,
    string_new_with_utf8_chars: unused_string,
    string_name_new_with_utf8_chars: unused_string,
};

macro_rules! unused {
    ($name:ident($($ty:ty),* $(,)?) -> $ret:ty, $value:expr) => {
        extern "C" fn $name($(_: $ty),*) -> $ret {
            MOCK.with(|cell| cell.borrow_mut().unexpected += 1);
            $value
        }
    };
}

unused!(
    unused_lookup(ConstStringNamePtr) -> ObjectPtr,
    ptr::null_mut()
);
unused!(
    unused_register(
        ClassLibraryPtr,
        ConstStringNamePtr,
        ConstStringNamePtr,
        *const ClassCreationInfo4,
    ) -> (),
    ()
);
unused!(
    unused_unregister(ClassLibraryPtr, ConstStringNamePtr) -> (),
    ()
);
unused!(
    unused_method(ClassLibraryPtr, ConstStringNamePtr, *const ClassMethodInfo) -> (),
    ()
);
unused!(
    unused_set_instance(ObjectPtr, ConstStringNamePtr, ClassInstancePtr) -> (),
    ()
);
unused!(unused_destructor(c_int) -> Option<PtrDestructorFn>, None);
unused!(unused_string(TypePtr, *const c_char) -> (), ());

#[test]
fn calls_construct_once_and_reject_deleted_object_identities() {
    assert!(ENGINE_THREAD.set(std::thread::current().id()).is_ok());
    assert!(API.set(MOCK_API).is_ok());
    let method = 1_u64;
    let name = (&method as *const u64).cast::<c_void>();
    GLOBAL.with(|cell| {
        let mut global = cell.borrow_mut();
        global.sn_set_visible = name.cast_mut();
        global.sn_queue_redraw = name.cast_mut();
        global.sn_add_child = name.cast_mut();
        global.sn_get_theme_default_font = name.cast_mut();
        global.sn_draw_style_box = name.cast_mut();
    });
    MOCK.with(|cell| {
        let mut mock = cell.borrow_mut();
        mock.objects.insert(1, Box::new(1));
        mock.objects.insert(2, Box::new(2));
    });
    let parent = Object::from_id(1);
    let child = Object::from_id(2);
    parent.set_visible(true);
    MOCK.with(|cell| {
        let mut mock = cell.borrow_mut();
        assert_eq!((mock.calls, mock.initialized, mock.destroyed), (1, 3, 3));
        mock.error = CALL_ERROR_INVALID_METHOD;
    });
    parent.queue_redraw();
    MOCK.with(|cell| {
        let mut mock = cell.borrow_mut();
        assert_eq!((mock.calls, mock.initialized, mock.destroyed), (2, 5, 5));
        mock.error = CALL_OK;
        mock.return_object = Some(2);
    });
    let resource = parent
        .get_theme_default_font()
        .expect("the mock returns live object 2");
    MOCK.with(|cell| {
        let mut mock = cell.borrow_mut();
        assert_eq!(mock.initialized - mock.destroyed, 1);
        // Retention releases byte storage without the engine destructor.
        mock.variants.clear();
        mock.return_object = None;
    });
    let rect = crate::engine::math::Rect2::new(Vector2::new(0.0, 0.0), Vector2::new(1.0, 1.0));
    assert!(parent.draw_style_box(&resource, rect));
    assert!(parent.draw_style_box(&resource, rect));
    MOCK.with(|cell| {
        let mut mock = cell.borrow_mut();
        mock.objects.remove(&2);
    });
    parent.add_child(child);
    assert!(!parent.draw_style_box(&resource, rect));
    MOCK.with(|cell| cell.borrow_mut().objects.remove(&1));
    parent.queue_redraw();
    assert!(parent.get_theme_default_font().is_none());
    std::thread::spawn(|| Object::from_id(1).set_visible(true))
        .join()
        .expect("off-thread calls return without engine access");
    MOCK.with(|cell| {
        let mock = cell.borrow();
        assert_eq!(mock.calls, 5);
        assert!(!mock.double_init && !mock.invalid_destroy && !mock.stale_use);
        assert_eq!(mock.unexpected, 0);
        assert!(mock.variants.is_empty());
    });
}
