#![forbid(unsafe_code)]

pub mod abi;
pub use abi::*;

#[cfg(test)]
mod tests {
    #[test]
    fn abi_uses_the_kernel_transaction_header_directly() {
        let source = include_str!("abi.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("WASM ABI production source");

        for forbidden in [
            "null_transaction",
            "NullTransaction",
            "WasmTransaction",
            "WasmTxn",
            "impl Transaction for",
        ] {
            assert!(
                !source.contains(forbidden),
                "the WASM ABI must not expose {forbidden}"
            );
        }

        assert!(source.contains("let txn = header;"));
    }
}
