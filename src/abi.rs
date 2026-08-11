use std::collections::HashMap;
use std::io;
use std::sync::{Mutex, OnceLock};

use bytes::Bytes;
use destream::de;
use destream::en::{self, EncodeMap, EncodeSeq};
use futures::{FutureExt, TryStreamExt, stream};
use tc_error::{TCError, TCResult};
use tc_ir::{Library, LibrarySchema, OpRef, TCRef, TxnHeader};
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

impl<'en> en::IntoStream<'en> for RouteExport {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(2))?;
        map.encode_entry("path", self.path)?;
        map.encode_entry("export", self.export)?;
        map.end()
    }
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

        match try_decode_json_slice((), bytes) {
            Ok(value) => Ok(value),
            Err(_) => String::from_utf8(bytes.to_vec())
                .map_err(|err| TCError::bad_request(format!("invalid utf-8 string: {err}"))),
        }
    }
}

impl WasmRequest for Value {
    fn decode(bytes: &[u8]) -> TCResult<Self> {
        if bytes.is_empty() {
            return Ok(Value::None);
        }

        try_decode_json_slice((), bytes).map_err(TCError::bad_request)
    }
}

impl WasmResponse for String {
    fn encode(self) -> TCResult<Vec<u8>> {
        encode_json_bytes(self)
    }
}

impl WasmResponse for Value {
    fn encode(self) -> TCResult<Vec<u8>> {
        encode_json_bytes(self)
    }
}

impl WasmResponse for () {
    fn encode(self) -> TCResult<Vec<u8>> {
        encode_json_bytes(())
    }
}

impl WasmResponse for OpRef {
    fn encode(self) -> TCResult<Vec<u8>> {
        encode_json_bytes(self)
    }
}

impl WasmResponse for TCRef {
    fn encode(self) -> TCResult<Vec<u8>> {
        encode_json_bytes(self)
    }
}

pub fn manifest_bytes<L: Library>(library: &L, routes: &[RouteExport]) -> Vec<u8> {
    let payload = ManifestPayload {
        schema: library.schema().clone(),
        routes: routes.to_vec(),
    };

    encode_json_bytes(payload).expect("manifest json")
}

pub fn alloc(len: i32) -> i32 {
    if len <= 0 {
        return 0;
    }

    register(vec![0_u8; len as usize].into_boxed_slice())
}

pub fn free(ptr: i32, len: i32) {
    if ptr == 0 || len <= 0 {
        return;
    }

    let removed = allocations()
        .lock()
        .expect("WASM allocations lock")
        .remove(&ptr);
    if let Some(bytes) = removed {
        debug_assert_eq!(bytes.len(), len as usize, "WASM allocation length");
    }
}

fn allocations() -> &'static Mutex<HashMap<i32, Box<[u8]>>> {
    // Exported C-style alloc/free functions have no instance receiver. This is
    // the sole project-owned mutable global and is isolated to the WASM ABI;
    // each WASM instance receives its own linear memory and allocation table.
    static ALLOCATIONS: OnceLock<Mutex<HashMap<i32, Box<[u8]>>>> = OnceLock::new();
    ALLOCATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register(mut bytes: Box<[u8]>) -> i32 {
    let ptr = bytes.as_mut_ptr() as i32;
    let replaced = allocations()
        .lock()
        .expect("WASM allocations lock")
        .insert(ptr, bytes);
    assert!(replaced.is_none(), "WASM allocation pointer collision");
    ptr
}

fn pack_wasm_pair(ptr: i32, len: i32) -> i64 {
    let ptr = ptr as u32 as u64;
    let len = len as u32 as u64;
    ((len << 32) | ptr) as i64
}

pub fn leak_bytes(bytes: Vec<u8>) -> i64 {
    if bytes.is_empty() {
        return 0;
    }

    let len = bytes.len() as i32;
    let ptr = register(bytes.into_boxed_slice());
    pack_wasm_pair(ptr, len)
}

struct ManifestPayload {
    schema: LibrarySchema,
    routes: Vec<RouteExport>,
}

impl<'en> en::IntoStream<'en> for ManifestPayload {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(2))?;
        map.encode_entry("schema", self.schema)?;
        map.encode_entry(
            "routes",
            ManifestRoutes {
                routes: self.routes,
            },
        )?;
        map.end()
    }
}

struct ManifestRoutes {
    routes: Vec<RouteExport>,
}

impl<'en> en::IntoStream<'en> for ManifestRoutes {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut seq = encoder.encode_seq(Some(self.routes.len()))?;
        for route in self.routes {
            seq.encode_element(route)?;
        }
        seq.end()
    }
}

struct ErrorPayload {
    message: String,
}

impl<'en> en::IntoStream<'en> for ErrorPayload {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(1))?;
        map.encode_entry("error", self.message)?;
        map.end()
    }
}

fn encode_json_bytes<T>(value: T) -> TCResult<Vec<u8>>
where
    T: for<'en> en::IntoStream<'en>,
{
    let stream =
        destream_json::encode(value).map_err(|err| TCError::bad_request(err.to_string()))?;
    stream
        .try_fold(Vec::new(), |mut acc, chunk| async move {
            acc.extend_from_slice(&chunk);
            Ok(acc)
        })
        .now_or_never()
        .ok_or_else(|| TCError::internal("WASM JSON encoder yielded across the synchronous ABI"))?
        .map_err(|err| TCError::bad_request(err.to_string()))
}

fn decode_json_bytes<T>(context: T::Context, bytes: Vec<u8>) -> TCResult<T>
where
    T: de::FromStream,
{
    let stream = stream::iter(vec![Ok::<Bytes, io::Error>(Bytes::from(bytes))]);
    destream_json::try_decode(context, stream)
        .now_or_never()
        .ok_or_else(|| TCError::internal("WASM JSON decoder yielded across the synchronous ABI"))?
        .map_err(|err| TCError::bad_request(err.to_string()))
}

fn try_decode_json_slice<T>(context: T::Context, bytes: &[u8]) -> Result<T, String>
where
    T: de::FromStream,
{
    let stream = stream::iter(vec![Ok::<Bytes, io::Error>(Bytes::copy_from_slice(bytes))]);
    destream_json::try_decode(context, stream)
        .now_or_never()
        .ok_or_else(|| "WASM JSON decoder yielded across the synchronous ABI".to_string())?
        .map_err(|err| err.to_string())
}

fn read_bytes(ptr: i32, len: i32) -> TCResult<Vec<u8>> {
    if ptr == 0 || len <= 0 {
        return Ok(Vec::new());
    }

    let allocations = allocations().lock().expect("WASM allocations lock");
    let bytes = allocations
        .get(&ptr)
        .ok_or_else(|| TCError::bad_request("unknown WASM allocation"))?;
    if bytes.len() != len as usize {
        return Err(TCError::bad_request("invalid WASM allocation length"));
    }

    Ok(bytes.to_vec())
}

fn decode_header_bytes(bytes: &[u8]) -> TCResult<TxnHeader> {
    if bytes.is_empty() {
        return Err(TCError::bad_request("missing transaction header"));
    }

    decode_json_bytes((), bytes.to_vec())
}

fn encode_error(err: TCError) -> Vec<u8> {
    encode_json_bytes(ErrorPayload {
        message: err.to_string(),
    })
    .unwrap_or_else(|_| br#"{"error":"internal"}"#.to_vec())
}

macro_rules! define_wasm_handler {
    ($trait_name:ident, $method:ident) => {
        pub trait $trait_name<Txn> {
            type Request;
            type Response;
            type Fut<'a>: std::future::Future<Output = TCResult<Self::Response>> + Send + 'a
            where
                Self: 'a,
                Txn: 'a,
                Self::Request: 'a;

            fn $method<'a>(
                &'a self,
                txn: &'a Txn,
                request: Self::Request,
            ) -> TCResult<Self::Fut<'a>>;
        }
    };
}

define_wasm_handler!(WasmGet, get);
define_wasm_handler!(WasmPut, put);
define_wasm_handler!(WasmPost, post);
define_wasm_handler!(WasmDelete, delete);

macro_rules! define_dispatch {
    (
        $dispatch_fn:ident,
        $try_dispatch_fn:ident,
        $try_dispatch_bytes_fn:ident,
        $handler_trait:ident,
        $handler_method:ident,
    ) => {
        pub fn $dispatch_fn<H, Req, Res>(
            handler: &H,
            header_ptr: i32,
            header_len: i32,
            body_ptr: i32,
            body_len: i32,
        ) -> i64
        where
            H: $handler_trait<TxnHeader, Request = Req, Response = Res>,
            Req: WasmRequest,
            Res: WasmResponse,
        {
            let result = $try_dispatch_fn(handler, header_ptr, header_len, body_ptr, body_len);
            match result {
                Ok(bytes) => leak_bytes(bytes),
                Err(err) => leak_bytes(encode_error(err)),
            }
        }

        fn $try_dispatch_fn<H, Req, Res>(
            handler: &H,
            header_ptr: i32,
            header_len: i32,
            body_ptr: i32,
            body_len: i32,
        ) -> TCResult<Vec<u8>>
        where
            H: $handler_trait<TxnHeader, Request = Req, Response = Res>,
            Req: WasmRequest,
            Res: WasmResponse,
        {
            let header_bytes = read_bytes(header_ptr, header_len)?;
            let body_bytes = read_bytes(body_ptr, body_len)?;
            $try_dispatch_bytes_fn(handler, &header_bytes, &body_bytes)
        }

        fn $try_dispatch_bytes_fn<H, Req, Res>(
            handler: &H,
            header_bytes: &[u8],
            body_bytes: &[u8],
        ) -> TCResult<Vec<u8>>
        where
            H: $handler_trait<TxnHeader, Request = Req, Response = Res>,
            Req: WasmRequest,
            Res: WasmResponse,
        {
            let header = decode_header_bytes(header_bytes)?;
            let txn = header;
            let request = Req::decode(body_bytes)?;
            let fut = handler.$handler_method(&txn, request)?;
            let response = fut.now_or_never().ok_or_else(|| {
                TCError::internal("WASM handler yielded across the synchronous ABI")
            })??;
            response.encode()
        }
    };
}

define_dispatch!(
    dispatch_get,
    try_dispatch_get,
    try_dispatch_get_bytes,
    WasmGet,
    get,
);

define_dispatch!(
    dispatch_put,
    try_dispatch_put,
    try_dispatch_put_bytes,
    WasmPut,
    put,
);

define_dispatch!(
    dispatch_post,
    try_dispatch_post,
    try_dispatch_post_bytes,
    WasmPost,
    post,
);

define_dispatch!(
    dispatch_delete,
    try_dispatch_delete,
    try_dispatch_delete_bytes,
    WasmDelete,
    delete,
);

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::str::FromStr;

    use futures::Future;
    use pathlink::Link;
    use tc_ir::{Claim, NetworkTime, Transaction, TxnHeader, TxnId};
    use umask::Mode;

    use super::*;

    struct VerbHandler;

    #[test]
    fn allocation_registry_validates_pointer_ownership_and_length() {
        let ptr = alloc(4);
        assert_eq!(read_bytes(ptr, 4).expect("allocated bytes"), vec![0; 4]);
        assert!(read_bytes(ptr, 3).is_err());

        free(ptr, 4);
        assert!(read_bytes(ptr, 4).is_err());
    }

    struct TestTxn {
        claim: Claim,
    }

    impl Transaction for TestTxn {
        fn id(&self) -> TxnId {
            TxnId::from_parts(NetworkTime::from_nanos(1), 7)
        }

        fn timestamp(&self) -> NetworkTime {
            NetworkTime::from_nanos(1)
        }

        fn claim(&self) -> &Claim {
            &self.claim
        }
    }

    impl WasmPut<TxnHeader> for VerbHandler {
        type Request = Value;
        type Response = Value;
        type Fut<'a> = Pin<Box<dyn Future<Output = TCResult<Self::Response>> + Send + 'a>>;

        fn put<'a>(
            &'a self,
            _txn: &'a TxnHeader,
            request: Self::Request,
        ) -> TCResult<Self::Fut<'a>> {
            Ok(Box::pin(async move { Ok(request) }))
        }
    }

    impl WasmPost<TxnHeader> for VerbHandler {
        type Request = Value;
        type Response = Value;
        type Fut<'a> = Pin<Box<dyn Future<Output = TCResult<Self::Response>> + Send + 'a>>;

        fn post<'a>(
            &'a self,
            _txn: &'a TxnHeader,
            request: Self::Request,
        ) -> TCResult<Self::Fut<'a>> {
            Ok(Box::pin(async move {
                Ok(Value::String(format!("post:{request:?}")))
            }))
        }
    }

    impl WasmDelete<TxnHeader> for VerbHandler {
        type Request = Value;
        type Response = Value;
        type Fut<'a> = Pin<Box<dyn Future<Output = TCResult<Self::Response>> + Send + 'a>>;

        fn delete<'a>(
            &'a self,
            _txn: &'a TxnHeader,
            request: Self::Request,
        ) -> TCResult<Self::Fut<'a>> {
            Ok(Box::pin(async move {
                Ok(Value::String(format!("delete:{request:?}")))
            }))
        }
    }

    fn txn_header_bytes() -> Vec<u8> {
        let claim = Claim::new(Link::from_str("/lib").expect("claim link"), Mode::all());
        let header = TxnHeader::from_transaction(&TestTxn { claim });
        encode_json_bytes(header).expect("header json")
    }

    #[test]
    fn dispatch_put_works() {
        let handler = VerbHandler;
        let header_bytes = txn_header_bytes();
        let request = Value::from(42u64);
        let body_bytes = encode_json_bytes(request.clone()).expect("body json");

        let response_bytes =
            try_dispatch_put_bytes::<_, Value, Value>(&handler, &header_bytes, &body_bytes)
                .expect("put response");

        let response: Value = try_decode_json_slice((), &response_bytes).expect("decode response");
        assert_eq!(response, request);
    }

    #[test]
    fn dispatch_post_works() {
        let handler = VerbHandler;
        let header_bytes = txn_header_bytes();
        let request = Value::from("hello");
        let body_bytes = encode_json_bytes(request.clone()).expect("body json");

        let response_bytes =
            try_dispatch_post_bytes::<_, Value, Value>(&handler, &header_bytes, &body_bytes)
                .expect("post response");

        let response: Value = try_decode_json_slice((), &response_bytes).expect("decode response");
        assert_eq!(response, Value::String(format!("post:{request:?}")));
    }

    #[test]
    fn dispatch_delete_works() {
        let handler = VerbHandler;
        let header_bytes = txn_header_bytes();
        let request = Value::from("goodbye");
        let body_bytes = encode_json_bytes(request.clone()).expect("body json");

        let response_bytes =
            try_dispatch_delete_bytes::<_, Value, Value>(&handler, &header_bytes, &body_bytes)
                .expect("delete response");

        let response: Value = try_decode_json_slice((), &response_bytes).expect("decode response");
        assert_eq!(response, Value::String(format!("delete:{request:?}")));
    }
}
