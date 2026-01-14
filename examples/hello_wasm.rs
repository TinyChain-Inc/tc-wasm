#![allow(improper_ctypes_definitions)]

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
fn main() {
    eprintln!(
        "Build with `cargo build --target wasm32-unknown-unknown --example hello_wasm --release` \
         to produce a TinyChain-compatible WASM library."
    );
}

#[cfg(target_arch = "wasm32")]
mod wasm_example {
    use once_cell::sync::Lazy;
    use std::str::FromStr;
    use pathlink::Link;
    use tc_error::{TCError, TCResult};
    use tc_ir::{
        Claim, Dir, HandleGet, LibrarySchema, NetworkTime, StaticLibrary, Transaction, TxnHeader,
        TxnId, tc_library_routes,
    };
    use tc_wasm::{RouteExport, WasmTransaction, dispatch_get, manifest_bytes};
    use umask::Mode;

    #[derive(Clone)]
    struct ExampleTxn {
        header: TxnHeader,
    }

    impl ExampleTxn {
        fn new() -> Self {
            let id = TxnId::from_parts(NetworkTime::from_nanos(42), 0);
            let claim = Claim::new(
                Link::from_str("/library/example").expect("schema link"),
                Mode::all(),
            );
            let header = TxnHeader::new(id, id.timestamp(), claim);
            Self { header }
        }

        fn from_header(header: TxnHeader) -> Self {
            Self { header }
        }
    }

    impl Transaction for ExampleTxn {
        fn id(&self) -> TxnId {
            self.header.id()
        }

        fn timestamp(&self) -> NetworkTime {
            self.header.timestamp()
        }

        fn claim(&self) -> &Claim {
            self.header.claim()
        }
    }

    impl WasmTransaction for ExampleTxn {
        fn from_wasm_header(header: TxnHeader) -> TCResult<Self> {
            Ok(Self::from_header(header))
        }
    }

    struct HelloHandler;

    impl HandleGet<ExampleTxn> for HelloHandler {
        type Request = String;
        type RequestContext = ();
        type Response = String;
        type Error = TCError;
        type Fut<'a> = std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send + 'a>,
        >;

        fn get<'a>(
            &'a self,
            _txn: &'a ExampleTxn,
            request: Self::Request,
        ) -> TCResult<Self::Fut<'a>> {
            Ok(Box::pin(async move { Ok(format!("Hello, {request}!")) }))
        }
    }

    type HelloLibrary = StaticLibrary<ExampleTxn, Dir<HelloHandler>>;

    fn hello_library() -> TCResult<HelloLibrary> {
        let schema = LibrarySchema::new(
            Link::from_str("/library/example").expect("schema link"),
            "0.1.0",
            vec![],
        );

        let routes = tc_library_routes! {
            "/hello" => HelloHandler,
        }?;

        Ok(StaticLibrary::new(schema, routes))
    }

    static LIBRARY: Lazy<HelloLibrary> = Lazy::new(|| hello_library().expect("library"));
    static HELLO_HANDLER: Lazy<HelloHandler> = Lazy::new(|| HelloHandler);

    const ROUTES: &[RouteExport] = &[RouteExport::new("/hello", "hello")];

    #[unsafe(no_mangle)]
    pub extern "C" fn alloc(len: i32) -> i32 {
        tc_wasm::alloc(len)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn free(ptr: i32, len: i32) {
        tc_wasm::free(ptr, len)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn tc_library_entry() -> i64 {
        tc_wasm::leak_bytes(manifest_bytes(&*LIBRARY, ROUTES))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn hello(
        header_ptr: i32,
        header_len: i32,
        body_ptr: i32,
        body_len: i32,
    ) -> i64 {
        dispatch_get::<_, ExampleTxn, String, String>(
            &*HELLO_HANDLER,
            header_ptr,
            header_len,
            body_ptr,
            body_len,
        )
    }
}
