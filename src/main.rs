use async_sdk::types::WasmVal;
use wasmedge_types::ValType;

use crate::async_sdk::{ast_module::Loader, config::Config, AsLinker, Linker};
mod async_sdk;

fn pass_and_load_wasm(path: &str) -> Vec<u8> {
    let wasm = std::fs::read(path).unwrap();
    let mut module = binaryen::Module::read(&wasm).unwrap();
    let mut codegen_config = binaryen::CodegenConfig::default();
    codegen_config.shrink_level = 2;
    codegen_config.optimization_level = 3;
    module
        .run_optimization_passes(
            ["asyncify"],
            &[("asyncify-imports", "spectest.sleep*")],
            &codegen_config,
        )
        .unwrap();
    module.write()
}

fn linker_prn(_linker: Option<&mut Linker>, input: &[WasmVal]) -> Result<Vec<WasmVal>, u32> {
    let n = input.first();
    if let Some(WasmVal::I32(n)) = n {
        println!("n={}", *n);
    }
    Ok(vec![])
}

static mut G: i32 = 0;
static mut G1: i32 = 0;
static mut G2: i32 = 0;

fn linker_sleep(linker: Option<&mut Linker>, _: &[WasmVal]) -> Result<Vec<WasmVal>, u32> {
    let linker = linker.unwrap();
    println!("enter sleep");

    unsafe {
        // await
        // do something
        if G == 0 {
            // linker.run("asyncify_stop_unwind", &[]).unwrap();
            linker.run("call_sleep1", &[]).unwrap();
            if G1 == 0 {
                // sleep1 pending
                linker.run("asyncify_start_unwind", &[]).unwrap();
            } else {
                if G2 == 0 {
                    println!("sleep pending");
                    // self pending
                    linker.run("asyncify_start_unwind", &[]).unwrap();
                } else {
                    println!("sleep done1");
                    linker.run("asyncify_stop_unwind", &[]).unwrap();
                }
            }
        } else {
            println!("sleep done");
            linker.run("asyncify_stop_unwind", &[]).unwrap();
        }
    }

    println!("return sleep");
    Ok(vec![])
}

fn linker_sleep1(linker: Option<&mut Linker>, _: &[WasmVal]) -> Result<Vec<WasmVal>, u32> {
    println!("enter sleep1");

    let linker = linker.unwrap();
    unsafe {
        if G1 == 0 {
            println!("sleep1 pending");
            linker.run("asyncify_start_unwind", &[]).unwrap();
        } else {
            println!("sleep1 done");
            linker.run("asyncify_stop_unwind", &[]).unwrap();
        }
    }
    println!("return sleep1");
    Ok(vec![])
}

fn try_asyncify() {
    println!("Hello, world!");
    let wasm = pass_and_load_wasm("demo2.wasm");
    let mut config = Config::create().unwrap();
    config.bulk_memory_operations(true);
    config.multi_memories(true);
    let config = Some(config);

    let loader = Loader::create(&config).unwrap();
    let ast_module = loader.load_module_from_bytes(&wasm).unwrap();

    let mut vm = Linker::new(&config).unwrap();

    vm.new_import_object("spectest", &mut |builder| {
        builder.add_func("sleep", (vec![], vec![]), linker_sleep, 0)?;
        builder.add_func("sleep1", (vec![], vec![]), linker_sleep1, 0)?;
        builder.add_func("print", (vec![ValType::I32], vec![]), linker_prn, 0)?;
        Ok(())
    })
    .unwrap();

    vm.active_module(&ast_module).unwrap();

    println!("active_module ok");
    println!();

    println!("start => 0");
    vm.run("asyncify_stop_unwind", &[]).unwrap();
    vm.run("_start", &[]).unwrap();

    println!("start => 1");
    vm.run("asyncify_start_rewind", &[]).unwrap();
    vm.run("_start", &[]).unwrap();

    unsafe {
        println!("start => 2");
        println!("asyncify_start_rewind... G1");
        G1 = 1;
        // G2 = 1;

        vm.run("asyncify_start_rewind", &[]).unwrap();
        vm.run("_start", &[]).unwrap();
    }
    unsafe {
        println!("start => 3");
        println!("asyncify_start_rewind... G");
        G = 1;
        vm.run("asyncify_start_rewind", &[]).unwrap();
        vm.run("_start", &[]).unwrap();
    }
}

// 每次 call_func 的时候 asyncify_stop_unwind
// 每次 恢复的时候 asyncify_start_rewind，会从 host_func 进入

fn main() {
    println!("Hello, world!");
    let wasm = pass_and_load_wasm("demo2.wasm");
    std::fs::write("demo_async.wasm", &wasm).unwrap();
    let mut config = Config::create().unwrap();
    config.bulk_memory_operations(true);
    config.multi_memories(true);
    let config = Some(config);

    let loader = Loader::create(&config).unwrap();
    // let ast_module = loader.load_async_module_from_bytes(&wasm,&[]).unwrap();
    let ast_module = loader.load_module_from_bytes(&wasm).unwrap();

    async_sdk::async_mod::try_(&config, ast_module);
}
