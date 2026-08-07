use std::ffi::{c_char, c_long, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use rb_sys::{rb_data_type_t, VALUE};

pub type RbResult<T = VALUE> = Result<T, RubyErr>;

#[derive(Debug)]
pub enum RubyErr {
    Exception(VALUE),
    Error { class: VALUE, message: String },
}

impl RubyErr {
    pub fn new(class: VALUE, message: impl Into<String>) -> Self {
        Self::Error {
            class,
            message: message.into(),
        }
    }

    pub fn arg(message: impl Into<String>) -> Self {
        Self::new(unsafe { rb_sys::rb_eArgError }, message)
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self::new(unsafe { rb_sys::rb_eRuntimeError }, message)
    }

    pub fn type_error(message: impl Into<String>) -> Self {
        Self::new(unsafe { rb_sys::rb_eTypeError }, message)
    }

    fn current_exception() -> Self {
        let err = unsafe { rb_sys::rb_errinfo() };
        if err == qnil() {
            Self::runtime("Ruby exception")
        } else {
            Self::Exception(err)
        }
    }
}

pub fn wrap<F>(f: F) -> VALUE
where
    F: FnOnce() -> RbResult<VALUE>,
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(value)) => value,
        Ok(Err(err)) => raise(err),
        Err(_) => raise(RubyErr::runtime("native Rust panic")),
    }
}

pub fn wrap_init<F>(f: F)
where
    F: FnOnce() -> RbResult<()>,
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => {}
        Ok(Err(err)) => raise(err),
        Err(_) => raise(RubyErr::runtime("native Rust panic")),
    }
}

pub fn raise(err: RubyErr) -> ! {
    match err {
        RubyErr::Exception(exc) => unsafe { rb_sys::rb_exc_raise(exc) },
        RubyErr::Error { class, message } => {
            let message = message.replace('\0', "\\0");
            let exc = {
                let c_message =
                    CString::new(message).unwrap_or_else(|_| CString::new("Ruby error").unwrap());
                unsafe { rb_sys::rb_exc_new_cstr(class, c_message.as_ptr()) }
            };
            unsafe { rb_sys::rb_exc_raise(exc) }
        }
    }
}

struct ProtectData<F> {
    func: Option<F>,
    panicked: bool,
}

pub fn protect_value<F>(func: F) -> RbResult<VALUE>
where
    F: FnOnce() -> VALUE,
{
    unsafe extern "C" fn call<F>(arg: VALUE) -> VALUE
    where
        F: FnOnce() -> VALUE,
    {
        let data = unsafe { &mut *(arg as *mut ProtectData<F>) };
        let Some(func) = data.func.take() else {
            data.panicked = true;
            return qnil();
        };

        match catch_unwind(AssertUnwindSafe(func)) {
            Ok(value) => value,
            Err(_) => {
                data.panicked = true;
                qnil()
            }
        }
    }

    let mut data = ProtectData {
        func: Some(func),
        panicked: false,
    };
    let mut state = 0;
    let value = unsafe {
        rb_sys::rb_protect(
            Some(call::<F>),
            &mut data as *mut ProtectData<F> as VALUE,
            &mut state,
        )
    };

    if state != 0 {
        Err(RubyErr::current_exception())
    } else if data.panicked {
        Err(RubyErr::runtime("native Rust panic"))
    } else {
        Ok(value)
    }
}

pub fn protect_unit<F>(func: F) -> RbResult<()>
where
    F: FnOnce(),
{
    protect_value(|| {
        func();
        qnil()
    })?;
    Ok(())
}

pub const fn qnil() -> VALUE {
    rb_sys::ruby_special_consts::RUBY_Qnil as VALUE
}

pub const fn qtrue() -> VALUE {
    rb_sys::ruby_special_consts::RUBY_Qtrue as VALUE
}

pub const fn qfalse() -> VALUE {
    rb_sys::ruby_special_consts::RUBY_Qfalse as VALUE
}

pub fn bool_value(value: bool) -> VALUE {
    if value {
        qtrue()
    } else {
        qfalse()
    }
}

pub fn new_binary_string(bytes: &[u8]) -> RbResult<VALUE> {
    let len = c_long_len(bytes.len())?;
    let ptr = if bytes.is_empty() {
        ptr::null()
    } else {
        bytes.as_ptr() as *const c_char
    };
    protect_value(|| unsafe { rb_sys::rb_str_new(ptr, len) })
}

pub fn string_value(value: VALUE) -> RbResult<VALUE> {
    protect_value(|| unsafe { rb_sys::rb_str_to_str(value) })
}

pub fn freeze_value(value: VALUE) -> RbResult<()> {
    protect_unit(|| unsafe { rb_sys::RB_OBJ_FREEZE(value) })
}

pub fn value_to_bytes(value: VALUE) -> RbResult<Vec<u8>> {
    let string = string_value(value)?;
    let len = unsafe { rb_sys::RSTRING_LEN(string) };
    if len < 0 {
        return Err(RubyErr::runtime("negative String length"));
    }
    if len == 0 {
        return Ok(Vec::new());
    }

    let ptr = unsafe { rb_sys::RSTRING_PTR(string) };
    if ptr.is_null() {
        return Err(RubyErr::runtime("null String pointer"));
    }

    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    Ok(bytes.to_vec())
}

pub fn value_to_option_bytes(value: VALUE) -> RbResult<Option<Vec<u8>>> {
    if value == qnil() {
        Ok(None)
    } else {
        value_to_bytes(value).map(Some)
    }
}

pub fn value_to_usize(value: VALUE) -> RbResult<usize> {
    let mut out = 0i64;
    protect_value(|| {
        out = unsafe { rb_sys::rb_num2long(value) as i64 };
        qnil()
    })?;
    usize::try_from(out).map_err(|_| RubyErr::arg("integer must be non-negative"))
}

pub fn value_to_u32(value: VALUE) -> RbResult<u32> {
    u32::try_from(value_to_usize(value)?).map_err(|_| RubyErr::arg("integer too large"))
}

pub fn value_to_i32(value: VALUE) -> RbResult<i32> {
    let mut out = 0i64;
    protect_value(|| {
        out = unsafe { rb_sys::rb_num2long(value) as i64 };
        qnil()
    })?;
    i32::try_from(out).map_err(|_| RubyErr::arg("integer too large"))
}

pub fn usize_value(value: usize) -> RbResult<VALUE> {
    protect_value(|| unsafe { rb_sys::rb_ull2inum(value as u64) })
}

pub fn i32_value(value: i32) -> VALUE {
    unsafe { rb_sys::rb_int2inum(value as isize) }
}

pub fn u64_option_value(value: Option<u64>) -> RbResult<VALUE> {
    match value {
        Some(value) => protect_value(|| unsafe { rb_sys::rb_ull2inum(value) }),
        None => Ok(qnil()),
    }
}

pub fn u32_option_value(value: Option<u32>) -> RbResult<VALUE> {
    match value {
        Some(value) => usize_value(value as usize),
        None => Ok(qnil()),
    }
}

pub unsafe fn wrap_typed_data<T>(
    class: VALUE,
    value: Box<T>,
    data_type: *const rb_data_type_t,
) -> RbResult<VALUE> {
    let raw = Box::into_raw(value);
    match protect_value(|| unsafe {
        rb_sys::rb_data_typed_object_wrap(class, raw as *mut c_void, data_type)
    }) {
        Ok(value) => Ok(value),
        Err(err) => {
            unsafe { drop(Box::from_raw(raw)) };
            Err(err)
        }
    }
}

pub unsafe fn typed_data_ref<T>(
    value: VALUE,
    data_type: *const rb_data_type_t,
    type_name: &str,
) -> RbResult<&'static T> {
    let mut ptr = std::ptr::null_mut();
    protect_unit(|| unsafe {
        ptr = rb_sys::rb_check_typeddata(value, data_type);
    })
    .map_err(|_| RubyErr::type_error(format!("expected {type_name}")))?;
    if ptr.is_null() {
        return Err(RubyErr::runtime(format!(
            "{type_name} data pointer is null"
        )));
    }

    Ok(unsafe { &*(ptr as *const T) })
}

pub unsafe fn define_module(name: &CStr) -> RbResult<VALUE> {
    protect_value(|| unsafe { rb_sys::rb_define_module(name.as_ptr()) })
}

pub unsafe fn define_class_under(outer: VALUE, name: &CStr, superclass: VALUE) -> RbResult<VALUE> {
    protect_value(|| unsafe { rb_sys::rb_define_class_under(outer, name.as_ptr(), superclass) })
}

pub unsafe fn define_error_under(outer: VALUE, name: &CStr, superclass: VALUE) -> RbResult<VALUE> {
    unsafe { define_class_under(outer, name, superclass) }
}

#[allow(dead_code)]
pub unsafe fn define_module_function_0(
    module: VALUE,
    name: &CStr,
    func: unsafe extern "C" fn(VALUE) -> VALUE,
) -> RbResult<()> {
    protect_unit(|| unsafe {
        rb_sys::rb_define_module_function(module, name.as_ptr(), Some(transmute_0(func)), 0)
    })
}

#[allow(dead_code)]
pub unsafe fn define_module_function_1(
    module: VALUE,
    name: &CStr,
    func: unsafe extern "C" fn(VALUE, VALUE) -> VALUE,
) -> RbResult<()> {
    protect_unit(|| unsafe {
        rb_sys::rb_define_module_function(module, name.as_ptr(), Some(transmute_1(func)), 1)
    })
}

pub unsafe fn define_singleton_method_1(
    object: VALUE,
    name: &CStr,
    func: unsafe extern "C" fn(VALUE, VALUE) -> VALUE,
) -> RbResult<()> {
    protect_unit(|| unsafe {
        rb_sys::rb_define_singleton_method(object, name.as_ptr(), Some(transmute_1(func)), 1)
    })
}

#[allow(dead_code)]
pub unsafe fn define_singleton_method_2(
    object: VALUE,
    name: &CStr,
    func: unsafe extern "C" fn(VALUE, VALUE, VALUE) -> VALUE,
) -> RbResult<()> {
    protect_unit(|| unsafe {
        rb_sys::rb_define_singleton_method(object, name.as_ptr(), Some(transmute_2(func)), 2)
    })
}

pub unsafe fn define_singleton_method_3(
    object: VALUE,
    name: &CStr,
    func: unsafe extern "C" fn(VALUE, VALUE, VALUE, VALUE) -> VALUE,
) -> RbResult<()> {
    protect_unit(|| unsafe {
        rb_sys::rb_define_singleton_method(object, name.as_ptr(), Some(transmute_3(func)), 3)
    })
}

pub unsafe fn define_method_0(
    class: VALUE,
    name: &CStr,
    func: unsafe extern "C" fn(VALUE) -> VALUE,
) -> RbResult<()> {
    protect_unit(|| unsafe {
        rb_sys::rb_define_method(class, name.as_ptr(), Some(transmute_0(func)), 0)
    })
}

pub unsafe fn define_method_1(
    class: VALUE,
    name: &CStr,
    func: unsafe extern "C" fn(VALUE, VALUE) -> VALUE,
) -> RbResult<()> {
    protect_unit(|| unsafe {
        rb_sys::rb_define_method(class, name.as_ptr(), Some(transmute_1(func)), 1)
    })
}

pub unsafe fn define_method_2(
    class: VALUE,
    name: &CStr,
    func: unsafe extern "C" fn(VALUE, VALUE, VALUE) -> VALUE,
) -> RbResult<()> {
    protect_unit(|| unsafe {
        rb_sys::rb_define_method(class, name.as_ptr(), Some(transmute_2(func)), 2)
    })
}

unsafe fn transmute_0(
    func: unsafe extern "C" fn(VALUE) -> VALUE,
) -> unsafe extern "C" fn() -> VALUE {
    unsafe {
        std::mem::transmute::<unsafe extern "C" fn(VALUE) -> VALUE, unsafe extern "C" fn() -> VALUE>(
            func,
        )
    }
}

unsafe fn transmute_1(
    func: unsafe extern "C" fn(VALUE, VALUE) -> VALUE,
) -> unsafe extern "C" fn() -> VALUE {
    unsafe {
        std::mem::transmute::<
            unsafe extern "C" fn(VALUE, VALUE) -> VALUE,
            unsafe extern "C" fn() -> VALUE,
        >(func)
    }
}

unsafe fn transmute_2(
    func: unsafe extern "C" fn(VALUE, VALUE, VALUE) -> VALUE,
) -> unsafe extern "C" fn() -> VALUE {
    unsafe {
        std::mem::transmute::<
            unsafe extern "C" fn(VALUE, VALUE, VALUE) -> VALUE,
            unsafe extern "C" fn() -> VALUE,
        >(func)
    }
}

unsafe fn transmute_3(
    func: unsafe extern "C" fn(VALUE, VALUE, VALUE, VALUE) -> VALUE,
) -> unsafe extern "C" fn() -> VALUE {
    unsafe {
        std::mem::transmute::<
            unsafe extern "C" fn(VALUE, VALUE, VALUE, VALUE) -> VALUE,
            unsafe extern "C" fn() -> VALUE,
        >(func)
    }
}

fn c_long_len(len: usize) -> RbResult<c_long> {
    c_long::try_from(len).map_err(|_| RubyErr::arg("length too large"))
}
