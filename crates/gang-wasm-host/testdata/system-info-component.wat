;; Test fixture: a `ganglion-capability`-world component that CALLS
;; `diagnostics-collect.system-info` and returns "<key>=<value>" of the first
;; diagnostic-entry as its run() payload.
;;
;; Generated from a hand-written wasm32 core module with:
;;   wasm-tools component embed --world ganglion-capability crates/gang-wasm-host/wit core.wasm
;;   wasm-tools component new core.embedded.wasm
;; (wasm-tools print of the result; the shim/fixup modules and the
;; canon-lower wiring are exactly what wit-bindgen/cargo-component emit.)
;;
;; Used by runtime tests to pin the host's typed `list<diagnostic-entry>`
;; return shape — a host returning JSON bytes here traps the component's
;; first import call with a Val type mismatch.
(component
  (type $ty-ganglion:capability/diagnostics-collect@0.5.0 (;0;)
    (instance
      (type (;0;) (record (field "key" string) (field "value" string)))
      (export (;1;) "diagnostic-entry" (type (eq 0)))
      (type (;2;) (list 1))
      (type (;3;) (result 2 (error string)))
      (type (;4;) (func (result 3)))
      (export (;0;) "system-info" (func (type 4)))
    )
  )
  (import "ganglion:capability/diagnostics-collect@0.5.0" (instance $ganglion:capability/diagnostics-collect@0.5.0 (;0;) (type $ty-ganglion:capability/diagnostics-collect@0.5.0)))
  (core module $main (;0;)
    (type (;0;) (func (param i32)))
    (type (;1;) (func (param i32 i32 i32 i32) (result i32)))
    (type (;2;) (func (param i32 i32 i32)))
    (type (;3;) (func (param i32 i32) (result i32)))
    (import "ganglion:capability/diagnostics-collect@0.5.0" "system-info" (func $si (;0;) (type 0)))
    (memory (;0;) 2)
    (global $heap (;0;) (mut i32) i32.const 16384)
    (export "memory" (memory 0))
    (export "cabi_realloc" (func 1))
    (export "run" (func 3))
    (func (;1;) (type 1) (param $old i32) (param $oldsz i32) (param $align i32) (param $size i32) (result i32)
      (local $ret i32) (local $end i32)
      global.get $heap
      local.get $align
      i32.const 1
      i32.sub
      i32.add
      local.get $align
      i32.const 1
      i32.sub
      i32.const -1
      i32.xor
      i32.and
      local.set $ret
      local.get $ret
      local.get $size
      i32.add
      local.set $end
      local.get $end
      memory.size
      i32.const 65536
      i32.mul
      i32.gt_u
      if ;; label = @1
        local.get $end
        memory.size
        i32.const 65536
        i32.mul
        i32.sub
        i32.const 65535
        i32.add
        i32.const 65536
        i32.div_u
        memory.grow
        drop
      end
      local.get $end
      global.set $heap
      local.get $ret
    )
    (func $memcpy (;2;) (type 2) (param $dst i32) (param $src i32) (param $n i32)
      (local $i i32)
      block $done
        loop $loop
          local.get $i
          local.get $n
          i32.ge_u
          br_if $done
          local.get $dst
          local.get $i
          i32.add
          local.get $src
          local.get $i
          i32.add
          i32.load8_u
          i32.store8
          local.get $i
          i32.const 1
          i32.add
          local.set $i
          br $loop
        end
      end
    )
    (func (;3;) (type 3) (param i32 i32) (result i32)
      (local $out i32) (local $list_ptr i32) (local $key_ptr i32) (local $key_len i32) (local $val_ptr i32) (local $val_len i32)
      i32.const 8192
      call $si
      i32.const 8192
      i32.load
      i32.const 0
      i32.ne
      if ;; label = @1
        i32.const 512
        i32.const 1
        i32.store
        i32.const 516
        i32.const 1024
        i32.store
        i32.const 520
        i32.const 16
        i32.store
        i32.const 512
        return
      end
      i32.const 8196
      i32.load
      local.set $list_ptr
      i32.const 8200
      i32.load
      i32.eqz
      if ;; label = @1
        i32.const 512
        i32.const 1
        i32.store
        i32.const 516
        i32.const 1024
        i32.store
        i32.const 520
        i32.const 16
        i32.store
        i32.const 512
        return
      end
      local.get $list_ptr
      i32.load
      local.set $key_ptr
      local.get $list_ptr
      i32.const 4
      i32.add
      i32.load
      local.set $key_len
      local.get $list_ptr
      i32.const 8
      i32.add
      i32.load
      local.set $val_ptr
      local.get $list_ptr
      i32.const 12
      i32.add
      i32.load
      local.set $val_len
      i32.const 2048
      local.set $out
      local.get $out
      local.get $key_ptr
      local.get $key_len
      call $memcpy
      local.get $out
      local.get $key_len
      i32.add
      local.set $out
      local.get $out
      i32.const 61
      i32.store8
      local.get $out
      i32.const 1
      i32.add
      local.set $out
      local.get $out
      local.get $val_ptr
      local.get $val_len
      call $memcpy
      local.get $out
      local.get $val_len
      i32.add
      local.set $out
      i32.const 512
      i32.const 0
      i32.store
      i32.const 516
      i32.const 2048
      i32.store
      i32.const 520
      local.get $out
      i32.const 2048
      i32.sub
      i32.store
      i32.const 512
    )
    (data (;0;) (i32.const 1024) "host call failed")
    (@producers
      (processed-by "wit-component" "0.255.0")
    )
  )
  (core module $wit-component-shim-module (;1;)
    (type (;0;) (func (param i32)))
    (table (;0;) 1 1 funcref)
    (export "0" (func 0))
    (export "$imports" (table 0))
    (func (;0;) (type 0) (param i32)
      local.get 0
      i32.const 0
      call_indirect (type 0)
    )
    (@producers
      (processed-by "wit-component" "0.255.0")
    )
  )
  (core module $wit-component-fixup (;2;)
    (type (;0;) (func (param i32)))
    (import "" "0" (func (;0;) (type 0)))
    (import "" "$imports" (table (;0;) 1 1 funcref))
    (elem (;0;) (i32.const 0) func 0)
    (@producers
      (processed-by "wit-component" "0.255.0")
    )
  )
  (core instance $wit-component-shim-instance (;0;) (instantiate $wit-component-shim-module))
  (alias core export $wit-component-shim-instance "0" (core func $indirect-ganglion:capability/diagnostics-collect@0.5.0-system-info (;0;)))
  (core instance $ganglion:capability/diagnostics-collect@0.5.0 (;1;)
    (export "system-info" (func $indirect-ganglion:capability/diagnostics-collect@0.5.0-system-info))
  )
  (core instance $main (;2;) (instantiate $main
      (with "ganglion:capability/diagnostics-collect@0.5.0" (instance $ganglion:capability/diagnostics-collect@0.5.0))
    )
  )
  (alias core export $main "memory" (core memory $memory (;0;)))
  (alias core export $wit-component-shim-instance "$imports" (core table $"shim table" (;0;)))
  (alias export $ganglion:capability/diagnostics-collect@0.5.0 "system-info" (func $system-info (;0;)))
  (alias core export $main "cabi_realloc" (core func $realloc (;1;)))
  (core func $"#core-func2 indirect-ganglion:capability/diagnostics-collect@0.5.0-system-info" (@name "indirect-ganglion:capability/diagnostics-collect@0.5.0-system-info") (;2;) (canon lower (func $system-info) (memory $memory) (realloc $realloc) string-encoding=utf8))
  (core instance $fixup-args (;3;)
    (export "$imports" (table $"shim table"))
    (export "0" (func $"#core-func2 indirect-ganglion:capability/diagnostics-collect@0.5.0-system-info"))
  )
  (core instance $fixup (;4;) (instantiate $wit-component-fixup
      (with "" (instance $fixup-args))
    )
  )
  (type (;1;) (list string))
  (type (;2;) (list u8))
  (type (;3;) (result 2 (error string)))
  (type (;4;) (func (param "args" 1) (result 3)))
  (alias core export $main "run" (core func $run (;3;)))
  (func $run (;1;) (type 4) (canon lift (core func $run) (memory $memory) (realloc $realloc) string-encoding=utf8))
  (export $"#func2 run" (@name "run") (;2;) "run" (func $run))
  (@producers
    (processed-by "wit-component" "0.255.0")
  )
)
