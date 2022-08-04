#[link(wasm_import_module = "spectest")]
extern "C" {
    fn sleep();
}

#[no_mangle]
extern "C" fn run() {
    println!("hello");
    unsafe {
        sleep();
    }
    println!("world")
}

fn main() {
    run()
}
