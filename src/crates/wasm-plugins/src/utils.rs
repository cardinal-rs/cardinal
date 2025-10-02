use crate::SharedExecutionContext;
use cardinal_errors::internal::CardinalInternalError;
use cardinal_errors::CardinalError;
use cardinal_errors::CardinalError::InternalError;
use wasmer::{FunctionEnvMut, MemoryView};

pub(crate) fn with_mem_view<'a>(
    ctx: &'a FunctionEnvMut<SharedExecutionContext>,
) -> Result<MemoryView<'a>, CardinalError> {
    let inner = ctx.data().read();
    let mem = inner.memory().as_ref().ok_or_else(|| {
        InternalError(CardinalInternalError::InvalidWasmModule(
            "memory not set".into(),
        ))
    })?;
    Ok(mem.view(ctx))
}

pub fn read_bytes(view: &MemoryView, ptr: i32, len: i32) -> Result<Vec<u8>, CardinalError> {
    let mut buf = vec![0u8; len as usize];
    view.read(ptr as u64, &mut buf)
        .map_err(|e| InternalError(CardinalInternalError::InvalidWasmModule(e.to_string())))?;
    Ok(buf)
}

pub fn write_bytes(view: &MemoryView, ptr: i32, data: &[u8]) -> Result<(), CardinalError> {
    view.write(ptr as u64, data)
        .map_err(|e| InternalError(CardinalInternalError::InvalidWasmModule(e.to_string())))
}
