use magnus::{
    exception::ExceptionClass, function, method, prelude::*, r_string::RString, value::Opaque,
    Error, Ruby,
};
use std::cell::RefCell;
use std::sync::{Mutex, OnceLock};

use zstd::dict::fastcover::FastCoverParams;
use zstd::{CompressContext, DecompressContext, Dictionary};

static DECOMPRESS_ERROR: OnceLock<Opaque<ExceptionClass>> = OnceLock::new();
static COMPRESS_ERROR: OnceLock<Opaque<ExceptionClass>> = OnceLock::new();
static MISSING_CONTENT_SIZE_ERROR: OnceLock<Opaque<ExceptionClass>> = OnceLock::new();
static OUTPUT_SIZE_LIMIT_ERROR: OnceLock<Opaque<ExceptionClass>> = OnceLock::new();

fn decompress_error(ruby: &Ruby) -> ExceptionClass {
    ruby.get_inner(
        *DECOMPRESS_ERROR
            .get()
            .expect("DecompressError not initialized"),
    )
}

fn compress_error(ruby: &Ruby) -> ExceptionClass {
    ruby.get_inner(*COMPRESS_ERROR.get().expect("CompressError not initialized"))
}

fn missing_content_size_error(ruby: &Ruby) -> ExceptionClass {
    ruby.get_inner(
        *MISSING_CONTENT_SIZE_ERROR
            .get()
            .expect("MissingContentSizeError not initialized"),
    )
}

fn output_size_limit_error(ruby: &Ruby) -> ExceptionClass {
    ruby.get_inner(
        *OUTPUT_SIZE_LIMIT_ERROR
            .get()
            .expect("OutputSizeLimitError not initialized"),
    )
}

// ---------- frame header parsing ----------

const ZSTD_FRAME_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

#[derive(Debug)]
enum BoundedError {
    BadMagic,
    MissingContentSize,
    OutputSizeLimit { declared: u64, limit: u64 },
    DecoderFailed(String),
}

fn parse_frame_content_size(input: &[u8]) -> Result<Option<u64>, BoundedError> {
    if input.len() < 5 {
        return Err(BoundedError::BadMagic);
    }
    if input[..4] != ZSTD_FRAME_MAGIC {
        return Err(BoundedError::BadMagic);
    }
    let fhd = input[4];
    let fcs_flag = (fhd >> 6) & 3;
    let single_segment = (fhd >> 5) & 1;
    let dict_id_flag = fhd & 3;

    let window_desc_size = if single_segment == 0 { 1usize } else { 0 };
    let dict_id_size = [0usize, 1, 2, 4][dict_id_flag as usize];
    let fcs_field_size = match fcs_flag {
        0 => {
            if single_segment == 1 {
                1usize
            } else {
                return Ok(None);
            }
        }
        1 => 2,
        2 => 4,
        3 => 8,
        _ => unreachable!(),
    };

    let fcs_offset = 5 + window_desc_size + dict_id_size;
    if input.len() < fcs_offset + fcs_field_size {
        return Err(BoundedError::BadMagic);
    }

    let fcs_bytes = &input[fcs_offset..fcs_offset + fcs_field_size];
    let value = match fcs_field_size {
        1 => fcs_bytes[0] as u64,
        2 => u16::from_le_bytes([fcs_bytes[0], fcs_bytes[1]]) as u64 + 256,
        4 => u32::from_le_bytes([fcs_bytes[0], fcs_bytes[1], fcs_bytes[2], fcs_bytes[3]]) as u64,
        8 => u64::from_le_bytes(fcs_bytes.try_into().unwrap()),
        _ => unreachable!(),
    };

    Ok(Some(value))
}

fn decompress_bounded(
    compressed: &[u8],
    max_output: usize,
    dctx: &mut DecompressContext,
) -> Result<Vec<u8>, BoundedError> {
    if compressed.len() < ZSTD_FRAME_MAGIC.len() || compressed[..4] != ZSTD_FRAME_MAGIC {
        return Err(BoundedError::BadMagic);
    }

    let upper = match parse_frame_content_size(compressed)? {
        Some(n) => {
            if max_output != 0 && n > max_output as u64 {
                return Err(BoundedError::OutputSizeLimit {
                    declared: n,
                    limit: max_output as u64,
                });
            }
            if n > u64::from(u32::MAX) {
                return Err(BoundedError::OutputSizeLimit {
                    declared: n,
                    limit: u64::from(u32::MAX),
                });
            }
            n as usize
        }
        None => {
            if max_output != 0 {
                return Err(BoundedError::MissingContentSize);
            }
            1024 * 1024
        }
    };

    let result = dctx
        .decompress_with_limit(compressed, upper)
        .map_err(|e| BoundedError::DecoderFailed(format!("{e}")))?;

    Ok(result.into_owned())
}

fn raise_bounded(ruby: &Ruby, err: BoundedError, prefix: &str) -> Error {
    match err {
        BoundedError::BadMagic => Error::new(
            decompress_error(ruby),
            format!("{prefix}: bad magic (input is not a Zstd frame)"),
        ),
        BoundedError::MissingContentSize => Error::new(
            missing_content_size_error(ruby),
            format!("{prefix}: Frame_Content_Size absent from frame header"),
        ),
        BoundedError::OutputSizeLimit { declared, limit } => Error::new(
            output_size_limit_error(ruby),
            format!("{prefix}: declared content size {declared} exceeds limit {limit}"),
        ),
        BoundedError::DecoderFailed(msg) => {
            Error::new(decompress_error(ruby), format!("{prefix}: {msg}"))
        }
    }
}

// ---------- dict helper ----------

fn load_dict(ruby: &Ruby, bytes: &[u8]) -> Result<Dictionary, Error> {
    Dictionary::from_bytes(bytes).map_err(|_| {
        Error::new(
            ruby.exception_runtime_error(),
            "dictionary must be in ZDICT format (use DictTrainer to train one)",
        )
    })
}

// ---------- FrameCodec ----------

#[magnus::wrap(class = "Zrip::FrameCodec", free_immediately, size)]
struct FrameCodec {
    dict_len: usize,
    dict_id: Option<u32>,
    level: i32,
    cctx: Mutex<CompressContext>,
    dctx: Mutex<DecompressContext>,
}

unsafe impl Send for FrameCodec {}
unsafe impl Sync for FrameCodec {}

fn frame_codec_new(
    ruby: &Ruby,
    rb_dict: Option<RString>,
    id: u32,
    level: i32,
) -> Result<FrameCodec, Error> {
    let (dict_len, dict_id, cctx, dctx) = match rb_dict {
        None => {
            let cctx = CompressContext::new(level).map_err(|e| {
                Error::new(
                    compress_error(ruby),
                    format!("CompressContext::new failed: {e}"),
                )
            })?;
            (0, None, cctx, DecompressContext::new())
        }
        Some(s) => {
            let bytes: Vec<u8> = unsafe { s.as_slice().to_vec() };
            s.freeze();
            let dict = load_dict(ruby, &bytes)?;
            let dl = bytes.len();
            let cctx = CompressContext::with_dict(level, dict.clone()).map_err(|e| {
                Error::new(
                    compress_error(ruby),
                    format!("CompressContext::with_dict failed: {e}"),
                )
            })?;
            let dctx = DecompressContext::with_dict(dict);
            (dl, Some(id), cctx, dctx)
        }
    };

    Ok(FrameCodec {
        dict_len,
        dict_id,
        level,
        cctx: Mutex::new(cctx),
        dctx: Mutex::new(dctx),
    })
}

fn frame_codec_compress(
    ruby: &Ruby,
    rb_self: &FrameCodec,
    rb_input: RString,
) -> Result<RString, Error> {
    let input: &[u8] = unsafe { rb_input.as_slice() };
    let mut cctx = rb_self.cctx.lock().expect("FrameCodec CCtx mutex poisoned");
    let out = cctx
        .compress(input)
        .map_err(|e| Error::new(compress_error(ruby), format!("zstd compress failed: {e}")))?;
    Ok(ruby.str_from_slice(&out))
}

fn frame_codec_decompress(
    ruby: &Ruby,
    rb_self: &FrameCodec,
    rb_input: RString,
    max_output: usize,
) -> Result<RString, Error> {
    let compressed: &[u8] = unsafe { rb_input.as_slice() };
    let mut dctx = rb_self.dctx.lock().expect("FrameCodec DCtx mutex poisoned");
    let out = decompress_bounded(compressed, max_output, &mut dctx)
        .map_err(|e| raise_bounded(ruby, e, "zstd frame decode failed"))?;
    Ok(ruby.str_from_slice(&out))
}

fn frame_codec_size(rb_self: &FrameCodec) -> usize {
    rb_self.dict_len
}

fn frame_codec_has_dict(rb_self: &FrameCodec) -> bool {
    rb_self.dict_id.is_some()
}

fn frame_codec_id(rb_self: &FrameCodec) -> Option<u32> {
    rb_self.dict_id
}

fn frame_codec_level(rb_self: &FrameCodec) -> i32 {
    rb_self.level
}

fn frame_codec_get_frame_content_size(
    ruby: &Ruby,
    rb_input: RString,
) -> Result<Option<u64>, Error> {
    let bytes: &[u8] = unsafe { rb_input.as_slice() };
    match parse_frame_content_size(bytes) {
        Ok(v) => Ok(v),
        Err(BoundedError::BadMagic) => Err(Error::new(
            decompress_error(ruby),
            "zstd frame header parse failed: bad magic (input is not a Zstd frame)",
        )),
        Err(e) => Err(raise_bounded(ruby, e, "zstd frame header parse failed")),
    }
}

// ---------- BlockCodec ----------

#[magnus::wrap(class = "Zrip::BlockCodec", free_immediately, size)]
struct BlockCodec {
    dict_len: usize,
    dict_id: Option<u32>,
    level: i32,
    cctx: RefCell<CompressContext>,
    dctx: RefCell<DecompressContext>,
}

fn block_codec_new(
    ruby: &Ruby,
    rb_dict: Option<RString>,
    id: u32,
    level: i32,
) -> Result<BlockCodec, Error> {
    let (dict_len, dict_id, cctx, dctx) = match rb_dict {
        None => {
            let cctx = CompressContext::new(level).map_err(|e| {
                Error::new(
                    compress_error(ruby),
                    format!("CompressContext::new failed: {e}"),
                )
            })?;
            (0, None, cctx, DecompressContext::new())
        }
        Some(s) => {
            let bytes: Vec<u8> = unsafe { s.as_slice().to_vec() };
            let dict = load_dict(ruby, &bytes)?;
            let dl = bytes.len();
            let cctx = CompressContext::with_dict(level, dict.clone()).map_err(|e| {
                Error::new(
                    compress_error(ruby),
                    format!("CompressContext::with_dict failed: {e}"),
                )
            })?;
            let dctx = DecompressContext::with_dict(dict);
            (dl, Some(id), cctx, dctx)
        }
    };

    Ok(BlockCodec {
        dict_len,
        dict_id,
        level,
        cctx: RefCell::new(cctx),
        dctx: RefCell::new(dctx),
    })
}

fn block_codec_compress(
    ruby: &Ruby,
    rb_self: &BlockCodec,
    rb_input: RString,
) -> Result<RString, Error> {
    let input: &[u8] = unsafe { rb_input.as_slice() };
    let mut cctx = rb_self.cctx.borrow_mut();
    let out = cctx
        .compress(input)
        .map_err(|e| Error::new(compress_error(ruby), format!("zstd compress failed: {e}")))?;
    Ok(ruby.str_from_slice(&out))
}

fn block_codec_decompress(
    ruby: &Ruby,
    rb_self: &BlockCodec,
    rb_input: RString,
    max_output: usize,
) -> Result<RString, Error> {
    let compressed: &[u8] = unsafe { rb_input.as_slice() };
    let mut dctx = rb_self.dctx.borrow_mut();
    let out = decompress_bounded(compressed, max_output, &mut dctx)
        .map_err(|e| raise_bounded(ruby, e, "zstd block decode failed"))?;
    Ok(ruby.str_from_slice(&out))
}

fn block_codec_size(rb_self: &BlockCodec) -> usize {
    rb_self.dict_len
}

fn block_codec_has_dict(rb_self: &BlockCodec) -> bool {
    rb_self.dict_id.is_some()
}

fn block_codec_level(rb_self: &BlockCodec) -> i32 {
    rb_self.level
}

// ---------- DictTrainer ----------

#[magnus::wrap(class = "Zrip::DictTrainer", free_immediately, size)]
struct RbDictTrainer {
    inner: RefCell<Option<TrainerState>>,
    max_dict_size: usize,
}

struct TrainerState {
    samples: Vec<Vec<u8>>,
    total_bytes: usize,
}

fn dict_trainer_new(_ruby: &Ruby, max_dict_size: usize) -> RbDictTrainer {
    RbDictTrainer {
        max_dict_size,
        inner: RefCell::new(Some(TrainerState {
            samples: Vec::new(),
            total_bytes: 0,
        })),
    }
}

fn dict_trainer_add_sample(
    ruby: &Ruby,
    rb_self: &RbDictTrainer,
    rb_data: RString,
) -> Result<(), Error> {
    let mut borrow = rb_self.inner.borrow_mut();
    let state = borrow.as_mut().ok_or_else(|| {
        Error::new(
            ruby.exception_runtime_error(),
            "DictTrainer already consumed by #train",
        )
    })?;
    let data: Vec<u8> = unsafe { rb_data.as_slice().to_vec() };
    if data.len() < 4 {
        return Ok(());
    }
    state.total_bytes += data.len();
    state.samples.push(data);
    Ok(())
}

fn dict_trainer_sample_count(ruby: &Ruby, rb_self: &RbDictTrainer) -> Result<usize, Error> {
    let borrow = rb_self.inner.borrow();
    borrow.as_ref().map(|s| s.samples.len()).ok_or_else(|| {
        Error::new(
            ruby.exception_runtime_error(),
            "DictTrainer already consumed by #train",
        )
    })
}

fn dict_trainer_total_bytes(ruby: &Ruby, rb_self: &RbDictTrainer) -> Result<usize, Error> {
    let borrow = rb_self.inner.borrow();
    borrow.as_ref().map(|s| s.total_bytes).ok_or_else(|| {
        Error::new(
            ruby.exception_runtime_error(),
            "DictTrainer already consumed by #train",
        )
    })
}

fn dict_trainer_train(ruby: &Ruby, rb_self: &RbDictTrainer) -> Result<RString, Error> {
    let state = rb_self.inner.borrow_mut().take().ok_or_else(|| {
        Error::new(
            ruby.exception_runtime_error(),
            "DictTrainer already consumed by #train",
        )
    })?;

    if state.samples.len() < 2 {
        return Ok(ruby.str_from_slice(b""));
    }

    let refs: Vec<&[u8]> = state.samples.iter().map(|s| s.as_slice()).collect();

    let content = zstd::dict::fastcover::select_segments(
        &refs,
        rb_self.max_dict_size,
        &FastCoverParams::default(),
    );
    let dict_bytes =
        zstd::dict::finalize::finalize_dictionary(&content, &refs, rb_self.max_dict_size);

    Ok(ruby.str_from_slice(&dict_bytes))
}

fn dict_trainer_max_dict_size(rb_self: &RbDictTrainer) -> usize {
    rb_self.max_dict_size
}

fn dict_trainer_trained(rb_self: &RbDictTrainer) -> bool {
    rb_self.inner.borrow().is_none()
}

// ---------- module init ----------

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    unsafe { rb_sys::rb_ext_ractor_safe(true) };

    let module = ruby.define_module("Zrip")?;

    let decompress_error_class =
        module.define_error("DecompressError", ruby.exception_standard_error())?;
    DECOMPRESS_ERROR
        .set(Opaque::from(decompress_error_class))
        .unwrap_or_else(|_| panic!("init called more than once"));

    let compress_error_class =
        module.define_error("CompressError", ruby.exception_standard_error())?;
    COMPRESS_ERROR
        .set(Opaque::from(compress_error_class))
        .unwrap_or_else(|_| panic!("init called more than once"));

    let missing_content_size_error_class =
        module.define_error("MissingContentSizeError", decompress_error_class)?;
    MISSING_CONTENT_SIZE_ERROR
        .set(Opaque::from(missing_content_size_error_class))
        .unwrap_or_else(|_| panic!("init called more than once"));

    let output_size_limit_error_class =
        module.define_error("OutputSizeLimitError", decompress_error_class)?;
    OUTPUT_SIZE_LIMIT_ERROR
        .set(Opaque::from(output_size_limit_error_class))
        .unwrap_or_else(|_| panic!("init called more than once"));

    // FrameCodec
    let frame_codec_class = module.define_class("FrameCodec", ruby.class_object())?;
    frame_codec_class.define_singleton_method("_native_new", function!(frame_codec_new, 3))?;
    frame_codec_class.define_singleton_method(
        "get_frame_content_size",
        function!(frame_codec_get_frame_content_size, 1),
    )?;
    frame_codec_class.define_method("compress", method!(frame_codec_compress, 1))?;
    frame_codec_class.define_method("_native_decompress", method!(frame_codec_decompress, 2))?;
    frame_codec_class.define_method("size", method!(frame_codec_size, 0))?;
    frame_codec_class.define_method("has_dict?", method!(frame_codec_has_dict, 0))?;
    frame_codec_class.define_method("id", method!(frame_codec_id, 0))?;
    frame_codec_class.define_method("level", method!(frame_codec_level, 0))?;

    // BlockCodec
    let block_codec_class = module.define_class("BlockCodec", ruby.class_object())?;
    block_codec_class.define_singleton_method("_native_new", function!(block_codec_new, 3))?;
    block_codec_class.define_method("compress", method!(block_codec_compress, 1))?;
    block_codec_class.define_method("_native_decompress", method!(block_codec_decompress, 2))?;
    block_codec_class.define_method("size", method!(block_codec_size, 0))?;
    block_codec_class.define_method("has_dict?", method!(block_codec_has_dict, 0))?;
    block_codec_class.define_method("level", method!(block_codec_level, 0))?;

    // DictTrainer
    let trainer_class = module.define_class("DictTrainer", ruby.class_object())?;
    trainer_class.define_singleton_method("_native_new", function!(dict_trainer_new, 1))?;
    trainer_class.define_method("add_sample", method!(dict_trainer_add_sample, 1))?;
    trainer_class.define_method("sample_count", method!(dict_trainer_sample_count, 0))?;
    trainer_class.define_method("total_bytes", method!(dict_trainer_total_bytes, 0))?;
    trainer_class.define_method("train", method!(dict_trainer_train, 0))?;
    trainer_class.define_method("max_dict_size", method!(dict_trainer_max_dict_size, 0))?;
    trainer_class.define_method("trained?", method!(dict_trainer_trained, 0))?;

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
}
