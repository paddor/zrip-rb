mod rb;

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant};

use rb_sys::{rb_data_type_struct__bindgen_ty_1, rb_data_type_t, size_t, VALUE};
use zstd::dict::fastcover::FastCoverParams;
use zstd::{CompressContext, DecompressContext, Dictionary};

use crate::rb::{RbResult, RubyErr};

static DECOMPRESS_ERROR: OnceLock<GlobalValue> = OnceLock::new();
static COMPRESS_ERROR: OnceLock<GlobalValue> = OnceLock::new();
static MISSING_CONTENT_SIZE_ERROR: OnceLock<GlobalValue> = OnceLock::new();
static OUTPUT_SIZE_LIMIT_ERROR: OnceLock<GlobalValue> = OnceLock::new();

const GVL_COMPRESS_THRESHOLD: usize = 64 * 1024;
const GVL_FRAME_DECOMPRESS_THRESHOLD: usize = 64 * 1024;

#[derive(Copy, Clone)]
struct GlobalValue(VALUE);

unsafe impl Send for GlobalValue {}
unsafe impl Sync for GlobalValue {}

fn stored_value(lock: &OnceLock<GlobalValue>, name: &str) -> VALUE {
    lock.get()
        .unwrap_or_else(|| panic!("{name} not initialized"))
        .0
}

fn decompress_error() -> VALUE {
    stored_value(&DECOMPRESS_ERROR, "DecompressError")
}

fn compress_error() -> VALUE {
    stored_value(&COMPRESS_ERROR, "CompressError")
}

fn output_size_limit_error() -> VALUE {
    stored_value(&OUTPUT_SIZE_LIMIT_ERROR, "OutputSizeLimitError")
}

fn should_release_compress_gvl(input_len: usize) -> bool {
    input_len >= GVL_COMPRESS_THRESHOLD
}

fn should_release_frame_decompress_gvl(output_len: usize) -> bool {
    output_len >= GVL_FRAME_DECOMPRESS_THRESHOLD
}

fn with_mutex<T, R, F>(
    mutex: &Mutex<T>,
    release_gvl: bool,
    name: &str,
    input: &mut rb::InputBytes,
    func: F,
) -> RbResult<R>
where
    F: FnOnce(&mut T, &[u8]) -> RbResult<R>,
{
    if release_gvl {
        input.freeze_for_without_gvl(true)?;
        return rb::maybe_without_gvl(true, || {
            let mut guard = mutex
                .lock()
                .map_err(|_| RubyErr::runtime(format!("{name} mutex poisoned")))?;
            func(&mut guard, input.as_slice())
        })?;
    }

    match mutex.try_lock() {
        Ok(mut guard) => func(&mut guard, input.as_slice()),
        Err(TryLockError::WouldBlock) => {
            // Even a small input needs protection when contention releases GVL.
            input.freeze_for_without_gvl(true)?;
            rb::maybe_without_gvl(true, || {
                let mut guard = mutex
                    .lock()
                    .map_err(|_| RubyErr::runtime(format!("{name} mutex poisoned")))?;
                func(&mut guard, input.as_slice())
            })?
        }
        Err(TryLockError::Poisoned(_)) => Err(RubyErr::runtime(format!("{name} mutex poisoned"))),
    }
}

/// Compression work since the last `#take_stats`. `busy` is measured around
/// the compression itself, so it leaves out waits for the GVL and the lock.
#[derive(Default)]
struct CompressStats {
    plain: AtomicU64,
    compressed: AtomicU64,
    busy_ns: AtomicU64,
}

impl CompressStats {
    fn record(&self, plain: usize, compressed: usize, busy: Duration) {
        self.plain.fetch_add(plain as u64, Ordering::Relaxed);
        self.compressed
            .fetch_add(compressed as u64, Ordering::Relaxed);
        let busy_ns = u64::try_from(busy.as_nanos()).unwrap_or(u64::MAX);
        self.busy_ns.fetch_add(busy_ns, Ordering::Relaxed);
    }

    /// Returns `[plain, compressed, busy_ns]` and resets them.
    fn take(&self) -> RbResult<VALUE> {
        rb::new_array(&[
            rb::u64_value(self.plain.swap(0, Ordering::Relaxed))?,
            rb::u64_value(self.compressed.swap(0, Ordering::Relaxed))?,
            rb::u64_value(self.busy_ns.swap(0, Ordering::Relaxed))?,
        ])
    }
}

/// Compresses `input` and records the work in `stats`.
fn compress_recorded(
    cctx: &mut CompressContext,
    stats: &CompressStats,
    input: &[u8],
) -> RbResult<Vec<u8>> {
    let start = Instant::now();
    let frame = cctx
        .compress(input)
        .map_err(|e| RubyErr::new(compress_error(), format!("zstd compress failed: {e}")))?;
    let busy = start.elapsed();
    stats.record(input.len(), frame.len(), busy);
    Ok(frame.into_owned())
}

/// Switches `cctx` to `rb_level` for later frames. A dictionary stays loaded;
/// zrip prepares it again for the new level.
fn set_level(
    cctx: &Mutex<CompressContext>,
    level: &AtomicI32,
    rb_level: VALUE,
    name: &str,
) -> RbResult<VALUE> {
    let new_level = rb::value_to_i32(rb_level)?;
    let apply = |cctx: &mut CompressContext| {
        cctx.set_level(new_level)
            .map_err(|e| RubyErr::arg(format!("zrip level {new_level}: {e}")))
    };
    let poisoned = || RubyErr::runtime(format!("{name} mutex poisoned"));
    match cctx.try_lock() {
        Ok(mut guard) => apply(&mut guard)?,
        // A compression holds it without the GVL: wait without it as well.
        Err(TryLockError::WouldBlock) => rb::maybe_without_gvl(true, || {
            let mut guard = cctx.lock().map_err(|_| poisoned())?;
            apply(&mut guard)
        })??,
        Err(TryLockError::Poisoned(_)) => return Err(poisoned()),
    }
    level.store(new_level, Ordering::Relaxed);
    Ok(rb::i32_value(new_level))
}

// ---------- frame header parsing ----------

const ZSTD_FRAME_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

#[derive(Debug)]
enum BoundedError {
    BadMagic,
    OutputSizeLimit { limit: u64 },
    DecoderFailed(String),
}

fn parse_frame_content_size(input: &[u8]) -> Result<Option<u64>, BoundedError> {
    zstd::frame::header::parse_frame_header(input)
        .map(|header| header.frame_content_size)
        .map_err(|err| match err {
            zstd::DecompressError::BadMagic => BoundedError::BadMagic,
            _ => BoundedError::DecoderFailed(err.to_string()),
        })
}

fn decompress_bounded(
    compressed: &[u8],
    max_output: Option<usize>,
    dctx: &mut DecompressContext,
) -> Result<Vec<u8>, BoundedError> {
    if compressed.len() < ZSTD_FRAME_MAGIC.len() || compressed[..4] != ZSTD_FRAME_MAGIC {
        return Err(BoundedError::BadMagic);
    }

    let limit = max_output.unwrap_or(usize::MAX);
    // A fresh buffer per frame: the context's own one would keep the largest
    // frame's size for the codec's lifetime.
    let mut out = Vec::new();
    dctx.decompress_into_with_limit(compressed, &mut out, limit)
        .map(|_| out)
        .map_err(|err| match err {
            zstd::DecompressError::OutputTooSmall => BoundedError::OutputSizeLimit {
                limit: limit as u64,
            },
            _ => BoundedError::DecoderFailed(err.to_string()),
        })
}

fn decompress_work_size(compressed: &[u8], max_output: Option<usize>) -> usize {
    let limit = max_output.unwrap_or(usize::MAX);
    match parse_frame_content_size(compressed) {
        Ok(Some(n)) => usize::try_from(n).unwrap_or(usize::MAX).min(limit),
        Ok(None) => max_output.unwrap_or(compressed.len()),
        // The decoder rejects the first frame header without decoding.
        Err(_) => 0,
    }
}

fn bounded_err(err: BoundedError, prefix: &str) -> RubyErr {
    match err {
        BoundedError::BadMagic => RubyErr::new(
            decompress_error(),
            format!("{prefix}: bad magic (input is not a Zstd frame)"),
        ),
        BoundedError::OutputSizeLimit { limit } => RubyErr::new(
            output_size_limit_error(),
            format!("{prefix}: decompressed output exceeds limit {limit}"),
        ),
        BoundedError::DecoderFailed(msg) => {
            RubyErr::new(decompress_error(), format!("{prefix}: {msg}"))
        }
    }
}

// ---------- dict helper ----------

fn load_dict(bytes: &[u8]) -> RbResult<Dictionary> {
    Dictionary::from_bytes(bytes).map_err(|_| {
        RubyErr::runtime("dictionary must be in ZDICT format (use DictTrainer to train one)")
    })
}

// ---------- typed data ----------

struct NativeDataType(rb_data_type_t);

unsafe impl Send for NativeDataType {}
unsafe impl Sync for NativeDataType {}

static FRAME_CODEC_DATA_TYPE: OnceLock<NativeDataType> = OnceLock::new();
static BLOCK_CODEC_DATA_TYPE: OnceLock<NativeDataType> = OnceLock::new();
static DICT_TRAINER_DATA_TYPE: OnceLock<NativeDataType> = OnceLock::new();

fn frame_codec_data_type() -> *const rb_data_type_t {
    &FRAME_CODEC_DATA_TYPE
        .get_or_init(|| NativeDataType(make_frame_codec_data_type()))
        .0
}

fn block_codec_data_type() -> *const rb_data_type_t {
    &BLOCK_CODEC_DATA_TYPE
        .get_or_init(|| NativeDataType(make_block_codec_data_type()))
        .0
}

fn dict_trainer_data_type() -> *const rb_data_type_t {
    &DICT_TRAINER_DATA_TYPE
        .get_or_init(|| NativeDataType(make_dict_trainer_data_type()))
        .0
}

fn make_frame_codec_data_type() -> rb_data_type_t {
    rb_data_type_t {
        wrap_struct_name: c"zrip_frame_codec".as_ptr(),
        function: rb_data_type_struct__bindgen_ty_1 {
            dmark: None,
            dfree: Some(frame_codec_free),
            dsize: Some(frame_codec_native_size),
            dcompact: None,
            reserved: [std::ptr::null_mut(); 1],
        },
        parent: std::ptr::null(),
        data: std::ptr::null_mut(),
        flags: rb::frozen_shareable_flags(),
    }
}

fn make_block_codec_data_type() -> rb_data_type_t {
    rb_data_type_t {
        wrap_struct_name: c"zrip_block_codec".as_ptr(),
        function: rb_data_type_struct__bindgen_ty_1 {
            dmark: None,
            dfree: Some(block_codec_free),
            dsize: Some(block_codec_native_size),
            dcompact: None,
            reserved: [std::ptr::null_mut(); 1],
        },
        parent: std::ptr::null(),
        data: std::ptr::null_mut(),
        flags: 1,
    }
}

fn make_dict_trainer_data_type() -> rb_data_type_t {
    rb_data_type_t {
        wrap_struct_name: c"zrip_dict_trainer".as_ptr(),
        function: rb_data_type_struct__bindgen_ty_1 {
            dmark: None,
            dfree: Some(dict_trainer_free),
            dsize: Some(dict_trainer_native_size),
            dcompact: None,
            reserved: [std::ptr::null_mut(); 1],
        },
        parent: std::ptr::null(),
        data: std::ptr::null_mut(),
        flags: 1,
    }
}

unsafe extern "C" fn frame_codec_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(ptr as *mut FrameCodec));
    }));
}

unsafe extern "C" fn block_codec_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(ptr as *mut BlockCodec));
    }));
}

unsafe extern "C" fn dict_trainer_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(ptr as *mut RbDictTrainer));
    }));
}

unsafe extern "C" fn frame_codec_native_size(_ptr: *const c_void) -> size_t {
    std::mem::size_of::<FrameCodec>() as size_t
}

unsafe extern "C" fn block_codec_native_size(_ptr: *const c_void) -> size_t {
    std::mem::size_of::<BlockCodec>() as size_t
}

unsafe extern "C" fn dict_trainer_native_size(_ptr: *const c_void) -> size_t {
    std::mem::size_of::<RbDictTrainer>() as size_t
}

unsafe fn frame_codec_ref(value: VALUE) -> RbResult<&'static FrameCodec> {
    unsafe { rb::typed_data_ref(value, frame_codec_data_type(), "Zrip::FrameCodec") }
}

unsafe fn block_codec_ref(value: VALUE) -> RbResult<&'static BlockCodec> {
    unsafe { rb::typed_data_ref(value, block_codec_data_type(), "Zrip::BlockCodec") }
}

unsafe fn dict_trainer_ref(value: VALUE) -> RbResult<&'static RbDictTrainer> {
    unsafe { rb::typed_data_ref(value, dict_trainer_data_type(), "Zrip::DictTrainer") }
}

// ---------- FrameCodec ----------

struct FrameCodec {
    dict_len: usize,
    dict_id: Option<u32>,
    level: AtomicI32,
    cctx: Mutex<CompressContext>,
    dctx: Mutex<DecompressContext>,
    stats: CompressStats,
}

unsafe impl Send for FrameCodec {}
unsafe impl Sync for FrameCodec {}

fn frame_codec_new_impl(class: VALUE, rb_dict: VALUE, id: VALUE, level: VALUE) -> RbResult<VALUE> {
    let rb_dict = rb::value_to_option_bytes(rb_dict)?;
    let id = rb::value_to_u32(id)?;
    let level = rb::value_to_i32(level)?;
    let (dict_len, dict_id, cctx, dctx) = match rb_dict {
        None => {
            let cctx = CompressContext::new(level).map_err(|e| {
                RubyErr::new(
                    compress_error(),
                    format!("CompressContext::new failed: {e}"),
                )
            })?;
            (0, None, cctx, DecompressContext::new())
        }
        Some(bytes) => {
            let dict = load_dict(&bytes)?;
            let dl = bytes.len();
            let cctx = CompressContext::with_dict(level, dict.clone()).map_err(|e| {
                RubyErr::new(
                    compress_error(),
                    format!("CompressContext::with_dict failed: {e}"),
                )
            })?;
            let dctx = DecompressContext::with_dict(dict.clone());
            (dl, Some(id), cctx, dctx)
        }
    };

    let codec = unsafe {
        rb::wrap_typed_data(
            class,
            Box::new(FrameCodec {
                dict_len,
                dict_id,
                level: AtomicI32::new(level),
                cctx: Mutex::new(cctx),
                dctx: Mutex::new(dctx),
                stats: CompressStats::default(),
            }),
            frame_codec_data_type(),
        )?
    };
    rb::make_shareable(codec)?;
    Ok(codec)
}

fn frame_codec_compress_impl(rb_self: VALUE, rb_input: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { frame_codec_ref(rb_self)? };
    let mut input = rb::input_bytes(rb_input)?;
    let release_gvl = should_release_compress_gvl(input.len());
    let out = with_mutex(
        &rb_self.cctx,
        release_gvl,
        "FrameCodec CCtx",
        &mut input,
        |cctx, input| compress_recorded(cctx, &rb_self.stats, input),
    )?;
    rb::new_binary_string(&out)
}

fn frame_codec_decompress_impl(
    rb_self: VALUE,
    rb_input: VALUE,
    max_output: VALUE,
) -> RbResult<VALUE> {
    let rb_self = unsafe { frame_codec_ref(rb_self)? };
    let max_output = rb::value_to_option_usize(max_output)?;
    let mut compressed = rb::input_bytes(rb_input)?;
    let release_gvl = should_release_frame_decompress_gvl(decompress_work_size(
        compressed.as_slice(),
        max_output,
    ));
    let out = with_mutex(
        &rb_self.dctx,
        release_gvl,
        "FrameCodec DCtx",
        &mut compressed,
        |dctx, compressed| {
            decompress_bounded(compressed, max_output, dctx)
                .map_err(|e| bounded_err(e, "zstd frame decode failed"))
        },
    )?;
    rb::new_binary_string(&out)
}

fn frame_codec_size_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { frame_codec_ref(rb_self)? };
    rb::usize_value(rb_self.dict_len)
}

fn frame_codec_has_dict_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { frame_codec_ref(rb_self)? };
    Ok(rb::bool_value(rb_self.dict_id.is_some()))
}

fn frame_codec_id_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { frame_codec_ref(rb_self)? };
    rb::u32_option_value(rb_self.dict_id)
}

fn frame_codec_level_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { frame_codec_ref(rb_self)? };
    Ok(rb::i32_value(rb_self.level.load(Ordering::Relaxed)))
}

fn frame_codec_get_frame_content_size_impl(rb_input: VALUE) -> RbResult<VALUE> {
    let bytes = rb::input_bytes(rb_input)?;
    match parse_frame_content_size(bytes.as_slice()) {
        Ok(v) => rb::u64_option_value(v),
        Err(BoundedError::BadMagic) => Err(RubyErr::new(
            decompress_error(),
            "zstd frame header parse failed: bad magic (input is not a Zstd frame)",
        )),
        Err(e) => Err(bounded_err(e, "zstd frame header parse failed")),
    }
}

unsafe extern "C" fn frame_codec_new(
    class: VALUE,
    rb_dict: VALUE,
    id: VALUE,
    level: VALUE,
) -> VALUE {
    rb::wrap(|| frame_codec_new_impl(class, rb_dict, id, level))
}

unsafe extern "C" fn frame_codec_compress(rb_self: VALUE, rb_input: VALUE) -> VALUE {
    rb::wrap(|| frame_codec_compress_impl(rb_self, rb_input))
}

unsafe extern "C" fn frame_codec_decompress(
    rb_self: VALUE,
    rb_input: VALUE,
    max_output: VALUE,
) -> VALUE {
    rb::wrap(|| frame_codec_decompress_impl(rb_self, rb_input, max_output))
}

unsafe extern "C" fn frame_codec_size(rb_self: VALUE) -> VALUE {
    rb::wrap(|| frame_codec_size_impl(rb_self))
}

unsafe extern "C" fn frame_codec_has_dict(rb_self: VALUE) -> VALUE {
    rb::wrap(|| frame_codec_has_dict_impl(rb_self))
}

unsafe extern "C" fn frame_codec_id(rb_self: VALUE) -> VALUE {
    rb::wrap(|| frame_codec_id_impl(rb_self))
}

fn frame_codec_set_level_impl(rb_self: VALUE, rb_level: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { frame_codec_ref(rb_self)? };
    set_level(&rb_self.cctx, &rb_self.level, rb_level, "FrameCodec CCtx")
}

fn frame_codec_take_stats_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { frame_codec_ref(rb_self)? };
    rb_self.stats.take()
}

unsafe extern "C" fn frame_codec_set_level(rb_self: VALUE, rb_level: VALUE) -> VALUE {
    rb::wrap(|| frame_codec_set_level_impl(rb_self, rb_level))
}

unsafe extern "C" fn frame_codec_take_stats(rb_self: VALUE) -> VALUE {
    rb::wrap(|| frame_codec_take_stats_impl(rb_self))
}

unsafe extern "C" fn frame_codec_level(rb_self: VALUE) -> VALUE {
    rb::wrap(|| frame_codec_level_impl(rb_self))
}

unsafe extern "C" fn frame_codec_get_frame_content_size(_class: VALUE, rb_input: VALUE) -> VALUE {
    rb::wrap(|| frame_codec_get_frame_content_size_impl(rb_input))
}

// ---------- BlockCodec ----------

struct BlockCodec {
    dict_len: usize,
    dict_id: Option<u32>,
    level: AtomicI32,
    cctx: Mutex<CompressContext>,
    dctx: Mutex<DecompressContext>,
    stats: CompressStats,
}

fn block_codec_new_impl(class: VALUE, rb_dict: VALUE, id: VALUE, level: VALUE) -> RbResult<VALUE> {
    let rb_dict = rb::value_to_option_bytes(rb_dict)?;
    let id = rb::value_to_u32(id)?;
    let level = rb::value_to_i32(level)?;
    let (dict_len, dict_id, cctx, dctx) = match rb_dict {
        None => {
            let cctx = CompressContext::new(level).map_err(|e| {
                RubyErr::new(
                    compress_error(),
                    format!("CompressContext::new failed: {e}"),
                )
            })?;
            (0, None, cctx, DecompressContext::new())
        }
        Some(bytes) => {
            let dict = load_dict(&bytes)?;
            let dl = bytes.len();
            let cctx = CompressContext::with_dict(level, dict.clone()).map_err(|e| {
                RubyErr::new(
                    compress_error(),
                    format!("CompressContext::with_dict failed: {e}"),
                )
            })?;
            let dctx = DecompressContext::with_dict(dict.clone());
            (dl, Some(id), cctx, dctx)
        }
    };

    unsafe {
        rb::wrap_typed_data(
            class,
            Box::new(BlockCodec {
                dict_len,
                dict_id,
                level: AtomicI32::new(level),
                cctx: Mutex::new(cctx),
                dctx: Mutex::new(dctx),
                stats: CompressStats::default(),
            }),
            block_codec_data_type(),
        )
    }
}

fn block_codec_compress_impl(rb_self: VALUE, rb_input: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { block_codec_ref(rb_self)? };
    let mut input = rb::input_bytes(rb_input)?;
    let release_gvl = should_release_compress_gvl(input.len());
    let out = with_mutex(
        &rb_self.cctx,
        release_gvl,
        "BlockCodec CCtx",
        &mut input,
        |cctx, input| compress_recorded(cctx, &rb_self.stats, input),
    )?;
    rb::new_binary_string(&out)
}

fn block_codec_decompress_impl(
    rb_self: VALUE,
    rb_input: VALUE,
    max_output: VALUE,
) -> RbResult<VALUE> {
    let rb_self = unsafe { block_codec_ref(rb_self)? };
    let max_output = rb::value_to_option_usize(max_output)?;
    let mut compressed = rb::input_bytes(rb_input)?;
    let out = with_mutex(
        &rb_self.dctx,
        false,
        "BlockCodec DCtx",
        &mut compressed,
        |dctx, compressed| {
            decompress_bounded(compressed, max_output, dctx)
                .map_err(|e| bounded_err(e, "zstd block decode failed"))
        },
    )?;
    rb::new_binary_string(&out)
}

fn block_codec_size_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { block_codec_ref(rb_self)? };
    rb::usize_value(rb_self.dict_len)
}

fn block_codec_has_dict_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { block_codec_ref(rb_self)? };
    Ok(rb::bool_value(rb_self.dict_id.is_some()))
}

fn block_codec_level_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { block_codec_ref(rb_self)? };
    Ok(rb::i32_value(rb_self.level.load(Ordering::Relaxed)))
}

unsafe extern "C" fn block_codec_new(
    class: VALUE,
    rb_dict: VALUE,
    id: VALUE,
    level: VALUE,
) -> VALUE {
    rb::wrap(|| block_codec_new_impl(class, rb_dict, id, level))
}

unsafe extern "C" fn block_codec_compress(rb_self: VALUE, rb_input: VALUE) -> VALUE {
    rb::wrap(|| block_codec_compress_impl(rb_self, rb_input))
}

unsafe extern "C" fn block_codec_decompress(
    rb_self: VALUE,
    rb_input: VALUE,
    max_output: VALUE,
) -> VALUE {
    rb::wrap(|| block_codec_decompress_impl(rb_self, rb_input, max_output))
}

unsafe extern "C" fn block_codec_size(rb_self: VALUE) -> VALUE {
    rb::wrap(|| block_codec_size_impl(rb_self))
}

unsafe extern "C" fn block_codec_has_dict(rb_self: VALUE) -> VALUE {
    rb::wrap(|| block_codec_has_dict_impl(rb_self))
}

fn block_codec_set_level_impl(rb_self: VALUE, rb_level: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { block_codec_ref(rb_self)? };
    set_level(&rb_self.cctx, &rb_self.level, rb_level, "BlockCodec CCtx")
}

fn block_codec_take_stats_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { block_codec_ref(rb_self)? };
    rb_self.stats.take()
}

unsafe extern "C" fn block_codec_set_level(rb_self: VALUE, rb_level: VALUE) -> VALUE {
    rb::wrap(|| block_codec_set_level_impl(rb_self, rb_level))
}

unsafe extern "C" fn block_codec_take_stats(rb_self: VALUE) -> VALUE {
    rb::wrap(|| block_codec_take_stats_impl(rb_self))
}

unsafe extern "C" fn block_codec_level(rb_self: VALUE) -> VALUE {
    rb::wrap(|| block_codec_level_impl(rb_self))
}

// ---------- DictTrainer ----------

struct RbDictTrainer {
    inner: Mutex<Option<TrainerState>>,
    max_dict_size: usize,
}

struct TrainerState {
    samples: Vec<Vec<u8>>,
    total_bytes: usize,
}

fn dict_trainer_new_impl(class: VALUE, max_dict_size: VALUE) -> RbResult<VALUE> {
    // The trainer gives a MIN_DICT_SIZE dict for smaller sizes.
    let max_dict_size = rb::value_to_usize(max_dict_size)?.max(zstd::dict::MIN_DICT_SIZE);
    unsafe {
        rb::wrap_typed_data(
            class,
            Box::new(RbDictTrainer {
                max_dict_size,
                inner: Mutex::new(Some(TrainerState {
                    samples: Vec::new(),
                    total_bytes: 0,
                })),
            }),
            dict_trainer_data_type(),
        )
    }
}

fn dict_trainer_add_sample_impl(rb_self: VALUE, rb_data: VALUE) -> RbResult<VALUE> {
    let data = rb::value_to_bytes(rb_data)?;
    let rb_self = unsafe { dict_trainer_ref(rb_self)? };
    let mut borrow = rb_self
        .inner
        .lock()
        .map_err(|_| RubyErr::runtime("DictTrainer mutex poisoned"))?;
    let state = borrow
        .as_mut()
        .ok_or_else(|| RubyErr::runtime("DictTrainer already consumed by #train"))?;
    if data.len() < 4 {
        return Ok(rb::qnil());
    }
    state.total_bytes += data.len();
    state.samples.push(data);
    Ok(rb::qnil())
}

fn dict_trainer_sample_count_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { dict_trainer_ref(rb_self)? };
    let borrow = rb_self
        .inner
        .lock()
        .map_err(|_| RubyErr::runtime("DictTrainer mutex poisoned"))?;
    let value = borrow
        .as_ref()
        .map(|s| s.samples.len())
        .ok_or_else(|| RubyErr::runtime("DictTrainer already consumed by #train"))?;
    rb::usize_value(value)
}

fn dict_trainer_total_bytes_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { dict_trainer_ref(rb_self)? };
    let borrow = rb_self
        .inner
        .lock()
        .map_err(|_| RubyErr::runtime("DictTrainer mutex poisoned"))?;
    let value = borrow
        .as_ref()
        .map(|s| s.total_bytes)
        .ok_or_else(|| RubyErr::runtime("DictTrainer already consumed by #train"))?;
    rb::usize_value(value)
}

fn dict_trainer_train_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { dict_trainer_ref(rb_self)? };
    let state = rb_self
        .inner
        .lock()
        .map_err(|_| RubyErr::runtime("DictTrainer mutex poisoned"))?
        .take()
        .ok_or_else(|| RubyErr::runtime("DictTrainer already consumed by #train"))?;

    if state.samples.len() < 2 {
        return rb::new_binary_string(b"");
    }

    let refs: Vec<&[u8]> = state.samples.iter().map(|s| s.as_slice()).collect();

    let content = zstd::dict::fastcover::select_segments(
        &refs,
        rb_self.max_dict_size,
        &FastCoverParams::default(),
    );
    let dict_bytes =
        zstd::dict::finalize::finalize_dictionary(&content, &refs, rb_self.max_dict_size);

    rb::new_binary_string(&dict_bytes)
}

fn dict_trainer_max_dict_size_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { dict_trainer_ref(rb_self)? };
    rb::usize_value(rb_self.max_dict_size)
}

fn dict_trainer_trained_impl(rb_self: VALUE) -> RbResult<VALUE> {
    let rb_self = unsafe { dict_trainer_ref(rb_self)? };
    let borrow = rb_self
        .inner
        .lock()
        .map_err(|_| RubyErr::runtime("DictTrainer mutex poisoned"))?;
    Ok(rb::bool_value(borrow.is_none()))
}

unsafe extern "C" fn dict_trainer_new(class: VALUE, max_dict_size: VALUE) -> VALUE {
    rb::wrap(|| dict_trainer_new_impl(class, max_dict_size))
}

unsafe extern "C" fn dict_trainer_add_sample(rb_self: VALUE, rb_data: VALUE) -> VALUE {
    rb::wrap(|| dict_trainer_add_sample_impl(rb_self, rb_data))
}

unsafe extern "C" fn dict_trainer_sample_count(rb_self: VALUE) -> VALUE {
    rb::wrap(|| dict_trainer_sample_count_impl(rb_self))
}

unsafe extern "C" fn dict_trainer_total_bytes(rb_self: VALUE) -> VALUE {
    rb::wrap(|| dict_trainer_total_bytes_impl(rb_self))
}

unsafe extern "C" fn dict_trainer_train(rb_self: VALUE) -> VALUE {
    rb::wrap(|| dict_trainer_train_impl(rb_self))
}

unsafe extern "C" fn dict_trainer_max_dict_size(rb_self: VALUE) -> VALUE {
    rb::wrap(|| dict_trainer_max_dict_size_impl(rb_self))
}

unsafe extern "C" fn dict_trainer_trained(rb_self: VALUE) -> VALUE {
    rb::wrap(|| dict_trainer_trained_impl(rb_self))
}

// ---------- module init ----------

/// # Safety
///
/// Ruby calls this function while loading the native extension. The Ruby VM
/// must be initialized, and the symbol must only be entered by Ruby's extension
/// loader.
#[no_mangle]
pub unsafe extern "C" fn Init_zrip() {
    rb::wrap_init(init);
}

fn init() -> RbResult<()> {
    #[cfg(ruby_engine = "mri")]
    unsafe {
        rb_sys::rb_ext_ractor_safe(true);
    }

    let module = unsafe { rb::define_module(c"Zrip")? };

    let decompress_error_class =
        unsafe { rb::define_error_under(module, c"DecompressError", rb_sys::rb_eStandardError)? };
    DECOMPRESS_ERROR
        .set(GlobalValue(decompress_error_class))
        .unwrap_or_else(|_| panic!("init called more than once"));

    let compress_error_class =
        unsafe { rb::define_error_under(module, c"CompressError", rb_sys::rb_eStandardError)? };
    COMPRESS_ERROR
        .set(GlobalValue(compress_error_class))
        .unwrap_or_else(|_| panic!("init called more than once"));

    let missing_content_size_error_class = unsafe {
        rb::define_error_under(module, c"MissingContentSizeError", decompress_error_class)?
    };
    MISSING_CONTENT_SIZE_ERROR
        .set(GlobalValue(missing_content_size_error_class))
        .unwrap_or_else(|_| panic!("init called more than once"));

    let output_size_limit_error_class =
        unsafe { rb::define_error_under(module, c"OutputSizeLimitError", decompress_error_class)? };
    OUTPUT_SIZE_LIMIT_ERROR
        .set(GlobalValue(output_size_limit_error_class))
        .unwrap_or_else(|_| panic!("init called more than once"));

    let frame_codec_class =
        unsafe { rb::define_class_under(module, c"FrameCodec", rb_sys::rb_cObject)? };
    unsafe {
        rb::undef_alloc_func(frame_codec_class)?;
        rb::define_singleton_method_3(frame_codec_class, c"_native_new", frame_codec_new)?;
        rb::define_singleton_method_1(
            frame_codec_class,
            c"get_frame_content_size",
            frame_codec_get_frame_content_size,
        )?;
        rb::define_method_1(frame_codec_class, c"compress", frame_codec_compress)?;
        rb::define_method_2(
            frame_codec_class,
            c"_native_decompress",
            frame_codec_decompress,
        )?;
        rb::define_method_0(frame_codec_class, c"size", frame_codec_size)?;
        rb::define_method_0(frame_codec_class, c"has_dict?", frame_codec_has_dict)?;
        rb::define_method_0(frame_codec_class, c"id", frame_codec_id)?;
        rb::define_method_0(frame_codec_class, c"level", frame_codec_level)?;
        rb::define_method_1(frame_codec_class, c"level=", frame_codec_set_level)?;
        rb::define_method_0(frame_codec_class, c"_take_stats", frame_codec_take_stats)?;
    }

    let block_codec_class =
        unsafe { rb::define_class_under(module, c"BlockCodec", rb_sys::rb_cObject)? };
    unsafe {
        rb::undef_alloc_func(block_codec_class)?;
        rb::define_singleton_method_3(block_codec_class, c"_native_new", block_codec_new)?;
        rb::define_method_1(block_codec_class, c"compress", block_codec_compress)?;
        rb::define_method_2(
            block_codec_class,
            c"_native_decompress",
            block_codec_decompress,
        )?;
        rb::define_method_0(block_codec_class, c"size", block_codec_size)?;
        rb::define_method_0(block_codec_class, c"has_dict?", block_codec_has_dict)?;
        rb::define_method_0(block_codec_class, c"level", block_codec_level)?;
        rb::define_method_1(block_codec_class, c"level=", block_codec_set_level)?;
        rb::define_method_0(block_codec_class, c"_take_stats", block_codec_take_stats)?;
    }

    let trainer_class =
        unsafe { rb::define_class_under(module, c"DictTrainer", rb_sys::rb_cObject)? };
    unsafe {
        rb::undef_alloc_func(trainer_class)?;
        rb::define_singleton_method_1(trainer_class, c"_native_new", dict_trainer_new)?;
        rb::define_method_1(trainer_class, c"add_sample", dict_trainer_add_sample)?;
        rb::define_method_0(trainer_class, c"sample_count", dict_trainer_sample_count)?;
        rb::define_method_0(trainer_class, c"total_bytes", dict_trainer_total_bytes)?;
        rb::define_method_0(trainer_class, c"train", dict_trainer_train)?;
        rb::define_method_0(trainer_class, c"max_dict_size", dict_trainer_max_dict_size)?;
        rb::define_method_0(trainer_class, c"trained?", dict_trainer_trained)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let data = b"the quick brown fox jumps over the lazy dog ".repeat(100);
        let compressed = zstd::compress(&data, 1).unwrap();
        assert!(compressed.len() < data.len());
        assert_eq!(&compressed[..4], &ZSTD_FRAME_MAGIC);
        let decompressed = zstd::decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn empty_round_trip() {
        let compressed = zstd::compress(b"", 1).unwrap();
        let decompressed = zstd::decompress(&compressed).unwrap();
        assert!(decompressed.is_empty());
    }

    #[test]
    fn context_round_trip() {
        let mut cctx = CompressContext::new(1).unwrap();
        let mut dctx = DecompressContext::new();
        let data = b"hello world hello world hello world";
        let ct = cctx.compress(data).unwrap();
        let pt = dctx.decompress(&ct).unwrap();
        assert_eq!(&*pt, data);
    }

    #[test]
    fn parse_fcs() {
        let data = b"test data for fcs parsing".repeat(10);
        let compressed = zstd::compress(&data, 1).unwrap();
        let fcs = parse_frame_content_size(&compressed).unwrap();
        assert_eq!(fcs, Some(data.len() as u64));
    }

    #[test]
    fn bounded_decompress_uses_total_concatenated_limit() {
        let mut dctx = DecompressContext::new();
        let mut compressed = zstd::compress(&[b'a'; 100], 1).unwrap();
        compressed.extend_from_slice(&zstd::compress(&[b'b'; 100], 1).unwrap());

        assert!(matches!(
            decompress_bounded(&compressed, Some(199), &mut dctx),
            Err(BoundedError::OutputSizeLimit { .. })
        ));
        assert_eq!(
            decompress_bounded(&compressed, Some(200), &mut dctx)
                .unwrap()
                .len(),
            200
        );
    }

    #[test]
    fn bounded_decompress_treats_zero_as_limit() {
        let mut dctx = DecompressContext::new();
        let empty = zstd::compress(b"", 1).unwrap();
        let one = zstd::compress(b"a", 1).unwrap();

        assert!(decompress_bounded(&empty, Some(0), &mut dctx)
            .unwrap()
            .is_empty());
        assert!(matches!(
            decompress_bounded(&one, Some(0), &mut dctx),
            Err(BoundedError::OutputSizeLimit { limit: 0 })
        ));
        assert_eq!(decompress_bounded(&one, None, &mut dctx).unwrap(), b"a");
    }
}
