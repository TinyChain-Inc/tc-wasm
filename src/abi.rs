use bytes::Bytes;
use futures::{TryStreamExt, executor::block_on, stream};
use serde_json::json;
use std::{io, mem, slice};
use tc_error::{TCError, TCResult};
use tc_ir::{Library, Transaction, TxnHeader};
use tc_value::Value;

/// Routes exported by a WASM library (path -> wasm export name).
#[derive(Clone, Copy)]
pub struct RouteExport {
    pub path: &'static str,
    pub export: &'static str,
}

impl RouteExport {
    pub const fn new(path: &'static str, export: &'static str) -> Self {
        Self { path, export }
    }
}

pub trait WasmTransaction: Transaction + Sized {
    fn from_wasm_header(header: TxnHeader) -> TCResult<Self>;
}

pub trait WasmRequest: Sized {
    fn decode(bytes: &[u8]) -> TCResult<Self>;
}

pub trait WasmResponse {
    fn encode(self) -> TCResult<Vec<u8>>;
}

impl WasmRequest for String {
    fn decode(bytes: &[u8]) -> TCResult<Self> {
        if bytes.is_empty() {
            return Ok(String::new());
        }

        match serde_json::from_slice::<serde_json::Value>(bytes) {
            Ok(serde_json::Value::String(s)) => Ok(s),
            Ok(other) => Ok(other.to_string()),
            Err(_) => Ok(String::from_utf8(bytes.to_vec())
                .map_err(|err| TCError::bad_request(format!("invalid utf-8 string: {err}")))?),
        }
    }
}

impl WasmRequest for Value {
    fn decode(bytes: &[u8]) -> TCResult<Self> {
        if bytes.is_empty() {
            return Ok(Value::None);
        }

        let stream = stream::iter(vec![Ok::<Bytes, io::Error>(Bytes::copy_from_slice(bytes))]);
        block_on(destream_json::try_decode((), stream))
            .map_err(|err| TCError::bad_request(err.to_string()))
    }
}

impl WasmResponse for String {
    fn encode(self) -> TCResult<Vec<u8>> {
        serde_json::to_vec(&json!(self)).map_err(|err| TCError::bad_request(err.to_string()))
    }
}

impl WasmResponse for Value {
    fn encode(self) -> TCResult<Vec<u8>> {
        let stream =
            destream_json::encode(self).map_err(|err| TCError::bad_request(err.to_string()))?;
        let bytes = block_on(stream.try_fold(Vec::new(), |mut acc, chunk| async move {
            acc.extend_from_slice(&chunk);
            Ok(acc)
        }))
        .map_err(|err| TCError::bad_request(err.to_string()))?;
        Ok(bytes)
    }
}

impl WasmResponse for () {
    fn encode(self) -> TCResult<Vec<u8>> {
        serde_json::to_vec(&serde_json::Value::Null)
            .map_err(|err| TCError::bad_request(err.to_string()))
    }
}

pub fn manifest_bytes<L: Library>(library: &L, routes: &[RouteExport]) -> Vec<u8> {
    let schema = library.schema();
    let deps: Vec<String> = schema
        .dependencies()
        .iter()
        .map(|link| link.to_string())
        .collect();

    let routes_json: Vec<_> = routes
        .iter()
        .map(|route| json!({ "path": route.path, "export": route.export }))
        .collect();

    serde_json::to_vec(&json!({
        "schema": {
            "id": schema.id().to_string(),
            "version": schema.version(),
            "dependencies": deps,
        },
        "routes": routes_json,
    }))
    .expect("manifest json")
}

pub fn alloc(len: i32) -> i32 {
    if len <= 0 {
        return 0;
    }

    let mut buffer = vec![0_u8; len as usize];
    let ptr = buffer.as_mut_ptr() as i32;
    mem::forget(buffer);
    ptr
}

pub fn free(ptr: i32, len: i32) {
    if ptr == 0 || len <= 0 {
        return;
    }

    unsafe {
        let _ = Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize);
    }
}

pub fn leak_bytes(bytes: Vec<u8>) -> (i32, i32) {
    if bytes.is_empty() {
        return (0, 0);
    }

    let len = bytes.len() as i32;
    let ptr = bytes.as_ptr() as i32;
    mem::forget(bytes);
    (ptr, len)
}

fn read_bytes(ptr: i32, len: i32) -> Vec<u8> {
    if ptr == 0 || len <= 0 {
        return Vec::new();
    }

    unsafe { slice::from_raw_parts(ptr as *const u8, len as usize).to_vec() }
}

fn decode_header(ptr: i32, len: i32) -> TCResult<TxnHeader> {
    let bytes = read_bytes(ptr, len);
    if bytes.is_empty() {
        return Err(TCError::bad_request("missing transaction header"));
    }

    let stream = stream::iter(vec![Ok::<Bytes, io::Error>(Bytes::from(bytes))]);
    block_on(destream_json::try_decode((), stream))
        .map_err(|err| TCError::bad_request(err.to_string()))
}

fn encode_error(err: TCError) -> Vec<u8> {
    serde_json::to_vec(&json!({ "error": err.to_string() }))
        .unwrap_or_else(|_| br#"{"error":"internal"}"#.to_vec())
}

pub fn dispatch_get<H, Txn, Req, Res>(
    handler: &H,
    header_ptr: i32,
    header_len: i32,
    body_ptr: i32,
    body_len: i32,
) -> (i32, i32)
where
    Txn: WasmTransaction,
    H: tc_ir::HandleGet<Txn, Request = Req, RequestContext = (), Response = Res, Error = TCError>,
    Req: WasmRequest,
    Res: WasmResponse,
{
    let result = try_dispatch_get(handler, header_ptr, header_len, body_ptr, body_len);
    match result {
        Ok(bytes) => leak_bytes(bytes),
        Err(err) => leak_bytes(encode_error(err)),
    }
}

fn try_dispatch_get<H, Txn, Req, Res>(
    handler: &H,
    header_ptr: i32,
    header_len: i32,
    body_ptr: i32,
    body_len: i32,
) -> TCResult<Vec<u8>>
where
    Txn: WasmTransaction,
    H: tc_ir::HandleGet<Txn, Request = Req, RequestContext = (), Response = Res, Error = TCError>,
    Req: WasmRequest,
    Res: WasmResponse,
{
    let header = decode_header(header_ptr, header_len)?;
    let txn = Txn::from_wasm_header(header)?;
    let request = Req::decode(&read_bytes(body_ptr, body_len))?;
    let fut = handler.get(&txn, request)?;
    let response = block_on(fut)?;
    response.encode()
}
