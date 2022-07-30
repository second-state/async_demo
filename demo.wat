(module
  (import "spectest" "print" (func $print (param i32)))
  (import "spectest" "sleep" (func $sleep))
  (import "spectest" "sleep1" (func $sleep1))
  (func $call_sleep1
    (call $print (i32.const -222))
    (call $sleep1)
    (call $print (i32.const 222))
  )
  (func $main
    (call $print (i32.const 1))
    (call $sleep)
    (call $print (i32.const 3))
  )
  (export "_start" (func $main))
  (export "call_sleep1" (func $call_sleep1))
 )