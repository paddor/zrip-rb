//! `Zrip::AdaptiveLevel`: zrip's hill climb over compression levels.

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rb_sys::{rb_data_type_struct__bindgen_ty_1, rb_data_type_t, size_t, VALUE};
use zstd::adaptive::{AdaptiveLevel, Bottleneck, CompressStats};

use crate::rb::{self, RbResult, RubyErr};

/// Lowest and highest level `AdaptiveLevel` accepts in a range: -8 runs as
/// -7, and 0 as the default level.
const RANGE_MIN: i32 = -8;
const RANGE_MAX: i32 = 4;

struct NativeDataType(rb_data_type_t);

unsafe impl Send for NativeDataType {}
unsafe impl Sync for NativeDataType {}

static DATA_TYPE: OnceLock<NativeDataType> = OnceLock::new();

fn data_type() -> *const rb_data_type_t {
    &DATA_TYPE
        .get_or_init(|| {
            NativeDataType(rb_data_type_t {
                wrap_struct_name: c"zrip_adaptive_level".as_ptr(),
                function: rb_data_type_struct__bindgen_ty_1 {
                    dmark: None,
                    dfree: Some(free),
                    dsize: Some(native_size),
                    dcompact: None,
                    reserved: [std::ptr::null_mut(); 1],
                },
                parent: std::ptr::null(),
                data: std::ptr::null_mut(),
                flags: 1,
            })
        })
        .0
}

/// The controller. Ruby calls hold the GVL, so the mutex never waits.
struct RbAdaptiveLevel(Mutex<AdaptiveLevel>);

unsafe extern "C" fn free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(ptr as *mut RbAdaptiveLevel));
    }));
}

unsafe extern "C" fn native_size(_ptr: *const c_void) -> size_t {
    std::mem::size_of::<RbAdaptiveLevel>() as size_t
}

unsafe fn adaptive_ref(value: VALUE) -> RbResult<&'static RbAdaptiveLevel> {
    unsafe { rb::typed_data_ref(value, data_type(), "Zrip::AdaptiveLevel") }
}

fn with_level<R>(rb_self: VALUE, func: impl FnOnce(&mut AdaptiveLevel) -> R) -> RbResult<R> {
    let rb_self = unsafe { adaptive_ref(rb_self)? };
    let mut guard = rb_self
        .0
        .lock()
        .map_err(|_| RubyErr::runtime("AdaptiveLevel mutex poisoned"))?;
    Ok(func(&mut guard))
}

fn new_impl(class: VALUE, min: VALUE, max: VALUE, start: VALUE) -> RbResult<VALUE> {
    let min = rb::value_to_i32(min)?;
    let max = rb::value_to_i32(max)?;
    let start = rb::value_to_i32(start)?;
    if min > max || max < RANGE_MIN || min > RANGE_MAX {
        return Err(RubyErr::arg(format!(
            "{min}..{max} holds no zrip level ({RANGE_MIN}..{RANGE_MAX})"
        )));
    }
    let level = AdaptiveLevel::new(min..=max, start);
    unsafe {
        rb::wrap_typed_data(
            class,
            Box::new(RbAdaptiveLevel(Mutex::new(level))),
            data_type(),
        )
    }
}

/// `bottleneck` is -1 (none), 0 (output), 1 (CPU) or 2 (input).
fn update_impl(
    rb_self: VALUE,
    plain: VALUE,
    compressed: VALUE,
    busy_ns: VALUE,
    bottleneck: VALUE,
) -> RbResult<VALUE> {
    let stats = CompressStats {
        plain: rb::value_to_u64(plain)?,
        compressed: rb::value_to_u64(compressed)?,
        busy: Duration::from_nanos(rb::value_to_u64(busy_ns)?),
    };
    let bottleneck = match rb::value_to_i32(bottleneck)? {
        -1 => None,
        0 => Some(Bottleneck::Output),
        1 => Some(Bottleneck::Cpu),
        2 => Some(Bottleneck::Input),
        code => return Err(RubyErr::arg(format!("unknown bottleneck code {code}"))),
    };
    let level = with_level(rb_self, |level| level.update(stats, bottleneck))?;
    Ok(rb::i32_value(level))
}

fn level_impl(rb_self: VALUE) -> RbResult<VALUE> {
    Ok(rb::i32_value(with_level(rb_self, |level| level.level())?))
}

unsafe extern "C" fn adaptive_new(class: VALUE, min: VALUE, max: VALUE, start: VALUE) -> VALUE {
    rb::wrap(|| new_impl(class, min, max, start))
}

unsafe extern "C" fn adaptive_update(
    rb_self: VALUE,
    plain: VALUE,
    compressed: VALUE,
    busy_ns: VALUE,
    bottleneck: VALUE,
) -> VALUE {
    rb::wrap(|| update_impl(rb_self, plain, compressed, busy_ns, bottleneck))
}

unsafe extern "C" fn adaptive_level(rb_self: VALUE) -> VALUE {
    rb::wrap(|| level_impl(rb_self))
}

/// Defines `Zrip::AdaptiveLevel` under `module`.
pub fn define(module: VALUE) -> RbResult<()> {
    let class = unsafe { rb::define_class_under(module, c"AdaptiveLevel", rb_sys::rb_cObject)? };
    unsafe {
        rb::undef_alloc_func(class)?;
        rb::define_singleton_method_3(class, c"_native_new", adaptive_new)?;
        rb::define_method_4(class, c"_update", adaptive_update)?;
        rb::define_method_0(class, c"level", adaptive_level)?;
    }
    Ok(())
}
