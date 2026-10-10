;; Spec 122 worked plugin: wraps its input as {"got": <input>}.
;; ABI: export `memory`, `alloc(len) -> ptr`, and `run(ptr, len) -> (out_ptr << 32 | out_len)`.
;; Build:  wasm-tools parse wrap.wat -o wrap.wasm     (or: wat2wasm wrap.wat -o wrap.wasm)
;; Limits of this toy: input must be under ~3 KiB (one page, bump allocator, no free).
(module
  (memory (export "memory") 1)
  (global $bump (mut i32) (i32.const 1024))
  (data (i32.const 4096) "{\"got\":")
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (local.get $n)))
    (local.get $p))
  (func (export "run") (param $p i32) (param $n i32) (result i64)
    ;; copy the input right after the 7-byte prefix, then close the object
    (memory.copy (i32.const 4103) (local.get $p) (local.get $n))
    (i32.store8 (i32.add (i32.const 4103) (local.get $n)) (i32.const 125))
    (i64.or (i64.shl (i64.const 4096) (i64.const 32))
            (i64.extend_i32_u (i32.add (local.get $n) (i32.const 8))))))
