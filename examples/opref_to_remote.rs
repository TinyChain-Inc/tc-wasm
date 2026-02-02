#![allow(improper_ctypes_definitions)]

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
fn main() {
    eprintln!(
        "Build with `cargo build --target wasm32-unknown-unknown --example opref_to_remote --release` \
         to produce a TinyChain-compatible WASM library."
    );
}

#[cfg(target_arch = "wasm32")]
mod wasm_example {
    use once_cell::sync::Lazy;
    use pathlink::Link;
    use std::str::FromStr;
    use tc_error::TCResult;
    use tc_ir::{
        Claim, Dir, HandleGet, LibrarySchema, NetworkTime, OpRef, Scalar, StaticLibrary, Subject,
        Transaction, TxnHeader, TxnId,
    };
    use tc_value::Value;
    use tc_wasm::{RouteExport, WasmTransaction, dispatch_get, manifest_bytes};
    use umask::Mode;

    const A_ROOT: &str = "/lib/example-devco/a/0.1.0";
    const B_ROOT: &str = "/lib/example-devco/example/0.1.0";
    const B_HELLO: &str = "/lib/example-devco/example/0.1.0/hello";

    #[derive(Clone)]
    struct NoopTxn;

    impl Transaction for NoopTxn {
        fn id(&self) -> TxnId {
            TxnId::from_parts(NetworkTime::from_nanos(0), 0)
        }

        fn timestamp(&self) -> NetworkTime {
            NetworkTime::from_nanos(0)
        }

        fn claim(&self) -> &Claim {
            static CLAIM: Lazy<Claim> = Lazy::new(|| {
                Claim::new(Link::from_str(A_ROOT).expect("claim link"), Mode::from(0u32))
            });
            &CLAIM
        }
    }

    impl WasmTransaction for NoopTxn {
        fn from_wasm_header(_header: TxnHeader) -> TCResult<Self> {
            Ok(Self)
        }
    }

    type Library = StaticLibrary<NoopTxn, Dir<()>>;

    struct FromBHandler;

    impl HandleGet<NoopTxn> for FromBHandler {
        type Request = Value;
        type RequestContext = ();
        type Response = OpRef;
        type Error = tc_error::TCError;
        type Fut<'a> = std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send + 'a>,
        >;

        fn get<'a>(&'a self, _txn: &'a NoopTxn, request: Self::Request) -> TCResult<Self::Fut<'a>> {
            Ok(Box::pin(async move {
                let link = Link::from_str(B_HELLO).expect("B_HELLO link");
                let scalar = Scalar::Value(request);
                Ok(OpRef::Get((Subject::Link(link), scalar)))
            }))
        }
    }

    fn library() -> TCResult<Library> {
        let schema = LibrarySchema::new(
            Link::from_str(A_ROOT).expect("schema link"),
            "0.1.0",
            vec![Link::from_str(B_ROOT).expect("dependency link")],
        );
        Ok(StaticLibrary::new(schema, Dir::new()))
    }

    static LIBRARY: Lazy<Library> = Lazy::new(|| library().expect("library"));
    static FROM_B_HANDLER: Lazy<FromBHandler> = Lazy::new(|| FromBHandler);
    const ROUTES: &[RouteExport] = &[RouteExport::new("/from_b", "from_b")];

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
    pub extern "C" fn from_b(
        header_ptr: i32,
        header_len: i32,
        body_ptr: i32,
        body_len: i32,
    ) -> i64 {
        dispatch_get::<_, NoopTxn, Value, OpRef>(
            &*FROM_B_HANDLER,
            header_ptr,
            header_len,
            body_ptr,
            body_len,
        )
    }
}
